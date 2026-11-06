use std::sync::Arc;
use tokio::sync::mpsc;

use adb_proto::{AckBytes, Command, RemoteId};
use adb_server::{OpenDecision, OpenRequest, PacketRouter, StreamMessage, StreamRegistry};
use adb_transport::{Packet, Transport, TransportError};

struct MockTransport {
    incoming_rx: mpsc::Receiver<Packet>,
    outgoing_tx: mpsc::Sender<Packet>,
}

impl MockTransport {
    fn new() -> (Self, mpsc::Sender<Packet>, mpsc::Receiver<Packet>) {
        let (in_tx, in_rx) = mpsc::channel(64);
        let (out_tx, out_rx) = mpsc::channel(64);
        (
            Self {
                incoming_rx: in_rx,
                outgoing_tx: out_tx,
            },
            in_tx,
            out_rx,
        )
    }
}

impl Transport for MockTransport {
    async fn read_packet(&mut self) -> Result<Packet, TransportError> {
        self.incoming_rx.recv().await.ok_or_else(|| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "closed",
            ))
        })
    }

    async fn write_packet(&mut self, packet: &Packet) -> Result<(), TransportError> {
        self.outgoing_tx.send(packet.clone()).await.map_err(|_| {
            TransportError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "closed",
            ))
        })
    }
}

#[tokio::test]
async fn test_multi_stream_data_interleaved() {
    let (transport, in_tx, _out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s1_tx, mut s1_rx) = mpsc::unbounded_channel();
    let (s2_tx, mut s2_rx) = mpsc::unbounded_channel();
    let (s3_tx, mut s3_rx) = mpsc::unbounded_channel();

    let id1 = registry
        .allocate_with_remote(RemoteId::from_u32(101).unwrap(), s1_tx)
        .unwrap();
    let id2 = registry
        .allocate_with_remote(RemoteId::from_u32(102).unwrap(), s2_tx)
        .unwrap();
    let id3 = registry
        .allocate_with_remote(RemoteId::from_u32(103).unwrap(), s3_tx)
        .unwrap();

    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // Send chunk 1 on stream 1, 2, and 3 concurrently (1 WRTE in flight per stream)
    in_tx
        .send(Packet::new(
            Command::Wrte,
            101,
            id1.get(),
            b"stream1-chunk1".to_vec(),
            true,
        ))
        .await
        .unwrap();
    in_tx
        .send(Packet::new(
            Command::Wrte,
            102,
            id2.get(),
            b"stream2-chunk1".to_vec(),
            true,
        ))
        .await
        .unwrap();
    in_tx
        .send(Packet::new(
            Command::Wrte,
            103,
            id3.get(),
            b"stream3-chunk1".to_vec(),
            true,
        ))
        .await
        .unwrap();

    // Stream 1 receives chunk 1 and sends OKAY
    match s1_rx.recv().await.unwrap() {
        StreamMessage::Data(data, acker) => {
            assert_eq!(data, b"stream1-chunk1");
            acker.send_okay().await.unwrap();
        }
        other => panic!("unexpected message: {other:?}"),
    }

    // Stream 2 receives chunk 1 and sends OKAY
    match s2_rx.recv().await.unwrap() {
        StreamMessage::Data(data, acker) => {
            assert_eq!(data, b"stream2-chunk1");
            acker.send_okay().await.unwrap();
        }
        other => panic!("unexpected message: {other:?}"),
    }

    // Stream 3 receives chunk 1 and sends OKAY
    match s3_rx.recv().await.unwrap() {
        StreamMessage::Data(data, acker) => {
            assert_eq!(data, b"stream3-chunk1");
            acker.send_okay().await.unwrap();
        }
        other => panic!("unexpected message: {other:?}"),
    }

    // After OKAYs have been received by peer, peer sends chunk 2 on stream 1 and stream 2
    in_tx
        .send(Packet::new(
            Command::Wrte,
            101,
            id1.get(),
            b"stream1-chunk2".to_vec(),
            true,
        ))
        .await
        .unwrap();
    in_tx
        .send(Packet::new(
            Command::Wrte,
            102,
            id2.get(),
            b"stream2-chunk2".to_vec(),
            true,
        ))
        .await
        .unwrap();

    // Stream 1 receives chunk 2 and sends OKAY
    match s1_rx.recv().await.unwrap() {
        StreamMessage::Data(data, acker) => {
            assert_eq!(data, b"stream1-chunk2");
            acker.send_okay().await.unwrap();
        }
        other => panic!("unexpected message: {other:?}"),
    }

    // Stream 2 receives chunk 2 and sends OKAY
    match s2_rx.recv().await.unwrap() {
        StreamMessage::Data(data, acker) => {
            assert_eq!(data, b"stream2-chunk2");
            acker.send_okay().await.unwrap();
        }
        other => panic!("unexpected message: {other:?}"),
    }

    // Clean drop to finish router loop
    drop(in_tx);
    let router_res = router_handle.await.unwrap();
    assert!(router_res.is_ok());
}

#[tokio::test]
async fn test_unmapped_stream_target_triggers_clse() {
    let (transport, in_tx, mut out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // 1. Send WRTE to non-existent local_id 999 with remote_id 42
    in_tx
        .send(Packet::new(Command::Wrte, 42, 999, b"hello".to_vec(), true))
        .await
        .unwrap();

    // Should receive CLSE(arg0 = 999, arg1 = 42)
    let reply = out_rx.recv().await.unwrap();
    assert_eq!(reply.header.command, Command::Clse);
    assert_eq!(reply.header.arg0, 999);
    assert_eq!(reply.header.arg1, 42);

    // 2. Send OKAY to non-existent local_id 888 with remote_id 55
    in_tx
        .send(Packet::new(Command::Okay, 55, 888, Vec::new(), true))
        .await
        .unwrap();

    let reply = out_rx.recv().await.unwrap();
    assert_eq!(reply.header.command, Command::Clse);
    assert_eq!(reply.header.arg0, 888);
    assert_eq!(reply.header.arg1, 55);

    // 3. Router should continue running without crashing
    let (s_tx, mut s_rx) = mpsc::unbounded_channel();
    let local_id = registry
        .allocate_with_remote(RemoteId::from_u32(77).unwrap(), s_tx)
        .unwrap();

    in_tx
        .send(Packet::new(
            Command::Wrte,
            77,
            local_id.get(),
            b"valid".to_vec(),
            true,
        ))
        .await
        .unwrap();

    match s_rx.recv().await.unwrap() {
        StreamMessage::Data(data, _) => assert_eq!(data, b"valid"),
        other => panic!("unexpected message: {other:?}"),
    }

    drop(in_tx);
    let router_res = router_handle.await.unwrap();
    assert!(router_res.is_ok());
}

#[tokio::test]
async fn test_open_packet_lifecycle() {
    let (transport, in_tx, mut out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    // Case A: Default unhandled OPEN -> replies with CLSE(0, remote_id)
    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // Send OPEN(arg0 = 123 (remote_id), arg1 = 0, payload = "shell:ls\0")
    in_tx
        .send(Packet::new(
            Command::Open,
            123,
            0,
            b"shell:ls\0".to_vec(),
            true,
        ))
        .await
        .unwrap();

    let reply = out_rx.recv().await.unwrap();
    assert_eq!(reply.header.command, Command::Clse);
    assert_eq!(reply.header.arg0, 0);
    assert_eq!(reply.header.arg1, 123);

    drop(in_tx);
    let _ = router_handle.await.unwrap();

    // Case B: Configured open handler accepting or rejecting
    let (transport2, in_tx2, mut out_rx2) = MockTransport::new();
    let registry2 = Arc::new(StreamRegistry::new());
    let reg_clone2 = Arc::clone(&registry2);

    let router_handle2 = tokio::spawn(async move {
        let router =
            PacketRouter::new(transport2, reg_clone2).with_open_handler(|req: OpenRequest| {
                if req.destination == "shell:echo hello" {
                    let (tx, _rx) = mpsc::unbounded_channel();
                    OpenDecision::Accept(tx)
                } else {
                    OpenDecision::Reject
                }
            });
        router.run().await
    });

    // Send rejected destination
    in_tx2
        .send(Packet::new(
            Command::Open,
            201,
            0,
            b"unsupported\0".to_vec(),
            true,
        ))
        .await
        .unwrap();

    let reply = out_rx2.recv().await.unwrap();
    assert_eq!(reply.header.command, Command::Clse);
    assert_eq!(reply.header.arg0, 0);
    assert_eq!(reply.header.arg1, 201);

    // Send accepted destination
    in_tx2
        .send(Packet::new(
            Command::Open,
            202,
            0,
            b"shell:echo hello\0".to_vec(),
            true,
        ))
        .await
        .unwrap();

    let reply = out_rx2.recv().await.unwrap();
    assert_eq!(reply.header.command, Command::Okay);
    assert_ne!(reply.header.arg0, 0); // local_id allocated
    assert_eq!(reply.header.arg1, 202); // remote_id echoed in arg1

    drop(in_tx2);
    let _ = router_handle2.await.unwrap();
}

#[tokio::test]
async fn test_okay_packet_lifecycle() {
    let (transport, in_tx, _out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s_tx, mut s_rx) = mpsc::unbounded_channel();
    // Initially allocated without remote_id (e.g. host issued OPEN and awaits OKAY)
    let local_id = registry.allocate(s_tx).unwrap();

    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // OKAY: arg0 = remote_id (peer), arg1 = local_id, optional payload for delayed acks
    let ack_bytes = AckBytes(4096);
    in_tx
        .send(Packet::new(
            Command::Okay,
            333,
            local_id.get(),
            ack_bytes.to_le_bytes().to_vec(),
            true,
        ))
        .await
        .unwrap();

    match s_rx.recv().await.unwrap() {
        StreamMessage::Ready {
            remote_id,
            ack_bytes: got_ack,
        } => {
            assert_eq!(remote_id.get(), 333);
            assert_eq!(got_ack, Some(AckBytes(4096)));
        }
        other => panic!("unexpected message: {other:?}"),
    }

    // Check that stream entry in registry has remote_id set to 333
    let entry = registry.get(local_id).unwrap();
    assert_eq!(entry.remote_id(), RemoteId::from_u32(333));

    drop(in_tx);
    let router_res = router_handle.await.unwrap();
    assert!(router_res.is_ok());
}

#[tokio::test]
async fn test_clse_packet_lifecycle() {
    let (transport, in_tx, _out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s_tx, mut s_rx) = mpsc::unbounded_channel();
    let local_id = registry
        .allocate_with_remote(RemoteId::from_u32(555).unwrap(), s_tx)
        .unwrap();

    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // Send CLSE(arg0 = 555, arg1 = local_id)
    in_tx
        .send(Packet::new(
            Command::Clse,
            555,
            local_id.get(),
            Vec::new(),
            true,
        ))
        .await
        .unwrap();

    match s_rx.recv().await.unwrap() {
        StreamMessage::Close => {}
        other => panic!("unexpected message: {other:?}"),
    }

    // Stream should be removed from registry
    assert!(registry.get(local_id).is_none());

    // Sending another CLSE to already closed local_id should be ignored (no response per protocol)
    in_tx
        .send(Packet::new(
            Command::Clse,
            555,
            local_id.get(),
            Vec::new(),
            true,
        ))
        .await
        .unwrap();

    drop(in_tx);
    let router_res = router_handle.await.unwrap();
    assert!(router_res.is_ok());
}

#[tokio::test]
async fn test_transport_disconnect_cleanup() {
    let (transport, in_tx, _out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s1_tx, mut s1_rx) = mpsc::unbounded_channel();
    let (s2_tx, mut s2_rx) = mpsc::unbounded_channel();

    let id1 = registry
        .allocate_with_remote(RemoteId::from_u32(1).unwrap(), s1_tx)
        .unwrap();
    let id2 = registry
        .allocate_with_remote(RemoteId::from_u32(2).unwrap(), s2_tx)
        .unwrap();

    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // Drop in_tx simulating unexpected EOF / disconnect
    drop(in_tx);

    let router_res = router_handle.await.unwrap();
    assert!(router_res.is_ok());

    // Both streams should receive Close and registry should be cleared
    match s1_rx.recv().await.unwrap() {
        StreamMessage::Close => {}
        other => panic!("unexpected message: {other:?}"),
    }
    match s2_rx.recv().await.unwrap() {
        StreamMessage::Close => {}
        other => panic!("unexpected message: {other:?}"),
    }

    assert!(registry.get(id1).is_none());
    assert!(registry.get(id2).is_none());
    assert_eq!(registry.count(), 0);
}

#[tokio::test]
async fn test_stream_write_normal_flow_after_okay() {
    let (transport, in_tx, mut out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s_tx, _s_rx) = mpsc::unbounded_channel();
    let local_id = registry
        .allocate_with_remote(RemoteId::from_u32(100).unwrap(), s_tx)
        .unwrap();

    let reg_clone = Arc::clone(&registry);
    let router = PacketRouter::new(transport, reg_clone);
    let writer = router.create_stream_writer(local_id).unwrap();

    let router_handle = tokio::spawn(async move { router.run().await });

    // First write should succeed and send WRTE packet
    let writer_clone = writer.clone();
    let write_handle1 = tokio::spawn(async move { writer_clone.write(b"chunk1".to_vec()).await });

    let pkt1 = out_rx.recv().await.unwrap();
    assert_eq!(pkt1.header.command, Command::Wrte);
    assert_eq!(pkt1.header.arg0, local_id.get());
    assert_eq!(pkt1.header.arg1, 100);
    assert_eq!(pkt1.payload, b"chunk1");

    // In-flight: simulate peer sending A_OKAY back
    in_tx
        .send(Packet::new(
            Command::Okay,
            100,
            local_id.get(),
            Vec::new(),
            true,
        ))
        .await
        .unwrap();

    assert!(write_handle1.await.unwrap().is_ok());

    // Second write should now succeed after A_OKAY arrived
    let writer_clone2 = writer.clone();
    let write_handle2 = tokio::spawn(async move { writer_clone2.write(b"chunk2".to_vec()).await });

    let pkt2 = out_rx.recv().await.unwrap();
    assert_eq!(pkt2.header.command, Command::Wrte);
    assert_eq!(pkt2.header.arg0, local_id.get());
    assert_eq!(pkt2.header.arg1, 100);
    assert_eq!(pkt2.payload, b"chunk2");

    in_tx
        .send(Packet::new(
            Command::Okay,
            100,
            local_id.get(),
            Vec::new(),
            true,
        ))
        .await
        .unwrap();

    assert!(write_handle2.await.unwrap().is_ok());

    drop(in_tx);
    let _ = router_handle.await.unwrap();
}

#[tokio::test]
async fn test_stream_write_flow_stall_backpressure() {
    let (transport, in_tx, mut out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s1_tx, _s1_rx) = mpsc::unbounded_channel();
    let (s2_tx, _s2_rx) = mpsc::unbounded_channel();
    let id1 = registry
        .allocate_with_remote(RemoteId::from_u32(101).unwrap(), s1_tx)
        .unwrap();
    let id2 = registry
        .allocate_with_remote(RemoteId::from_u32(102).unwrap(), s2_tx)
        .unwrap();

    let reg_clone = Arc::clone(&registry);
    let router = PacketRouter::new(transport, reg_clone);
    let writer1 = router.create_stream_writer(id1).unwrap();
    let writer2 = router.create_stream_writer(id2).unwrap();

    let router_handle = tokio::spawn(async move { router.run().await });

    // Stream 1 sends first write
    let w1_c1 = writer1.clone();
    let h1_w1 = tokio::spawn(async move { w1_c1.write(b"s1-pkt1".to_vec()).await });

    let pkt1 = out_rx.recv().await.unwrap();
    assert_eq!(pkt1.header.command, Command::Wrte);
    assert_eq!(pkt1.header.arg0, id1.get());

    // Stream 1 immediately queues second write without OKAY having arrived
    let w1_c2 = writer1.clone();
    let h1_w2 = tokio::spawn(async move { w1_c2.write(b"s1-pkt2".to_vec()).await });

    // Ensure h1_w2 does not complete (it is stalled awaiting OKAY)
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!h1_w2.is_finished(), "second write must await OKAY");

    // But Stream 2 is independent and can write concurrently!
    let w2_c1 = writer2.clone();
    let _h2_w1 = tokio::spawn(async move { w2_c1.write(b"s2-pkt1".to_vec()).await });

    let pkt2 = out_rx.recv().await.unwrap();
    assert_eq!(pkt2.header.command, Command::Wrte);
    assert_eq!(pkt2.header.arg0, id2.get());
    assert_eq!(pkt2.payload, b"s2-pkt1");

    // Now send OKAY for stream 1
    in_tx
        .send(Packet::new(Command::Okay, 101, id1.get(), Vec::new(), true))
        .await
        .unwrap();

    assert!(h1_w1.await.unwrap().is_ok());

    // Stream 1's second write unblocks and emits WRTE
    let pkt3 = out_rx.recv().await.unwrap();
    assert_eq!(pkt3.header.command, Command::Wrte);
    assert_eq!(pkt3.header.arg0, id1.get());
    assert_eq!(pkt3.payload, b"s1-pkt2");

    // Send OKAY for stream 1 second write
    in_tx
        .send(Packet::new(Command::Okay, 101, id1.get(), Vec::new(), true))
        .await
        .unwrap();

    assert!(h1_w2.await.unwrap().is_ok());

    drop(in_tx);
    let _ = router_handle.await.unwrap();
}

#[tokio::test]
async fn test_protocol_violation_back_to_back_wrte_terminates_transport() {
    let (transport, in_tx, _out_rx) = MockTransport::new();
    let registry = Arc::new(StreamRegistry::new());

    let (s_tx, mut s_rx) = mpsc::unbounded_channel();
    let local_id = registry
        .allocate_with_remote(RemoteId::from_u32(200).unwrap(), s_tx)
        .unwrap();

    let reg_clone = Arc::clone(&registry);
    let router_handle = tokio::spawn(async move {
        let router = PacketRouter::new(transport, reg_clone);
        router.run().await
    });

    // Remote peer sends first A_WRTE packet
    in_tx
        .send(Packet::new(
            Command::Wrte,
            200,
            local_id.get(),
            b"first_write".to_vec(),
            true,
        ))
        .await
        .unwrap();

    // First write data is received
    match s_rx.recv().await.unwrap() {
        StreamMessage::Data(data, _) => assert_eq!(data, b"first_write"),
        other => panic!("unexpected message: {other:?}"),
    }

    // Remote peer sends SECOND A_WRTE without waiting for A_OKAY from us!
    in_tx
        .send(Packet::new(
            Command::Wrte,
            200,
            local_id.get(),
            b"violating_write".to_vec(),
            true,
        ))
        .await
        .unwrap();

    // The router loop must terminate with ProtocolViolation
    let res = router_handle.await.unwrap();
    match res {
        Err(adb_server::RouterError::ProtocolViolation(msg)) => {
            assert!(msg.contains("consecutive WRTE without OKAY"));
        }
        other => panic!("expected RouterError::ProtocolViolation, got {other:?}"),
    }
}
