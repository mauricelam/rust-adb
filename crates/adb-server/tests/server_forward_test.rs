use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use adb_proto::smart_socket::{
    parse_protocol_string, parse_reply, ProtocolString, SmartSocketReply,
};
use adb_server::{ConnectionState, DeviceRegistry, SmartSocketServer, TransportType};

#[tokio::test]
async fn test_forward_lifecycle_and_norebind() {
    let registry = Arc::new(DeviceRegistry::new());
    let dev_id = registry
        .register("device-1", TransportType::Usb)
        .expect("register device");
    registry.update_state(dev_id, ConnectionState::Device);

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // 1. Initial list-forward is empty
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0011host:list-forward")
        .await
        .expect("write");
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let (reply, consumed) = parse_reply(&buf[..n]).expect("parse reply");
    assert_eq!(reply, SmartSocketReply::Okay);
    let (proto_str, _) = parse_protocol_string(&buf[consumed..n]).expect("parse payload");
    match proto_str {
        ProtocolString::Complete(list) => assert_eq!(list, ""),
        other => panic!("expected Complete, got {other:?}"),
    }

    // 2. Add forward tcp:0 -> tcp:8080 (dynamic port)
    // Format on wire: host:forward:tcp:0;tcp:8080
    // Request string length: 30 bytes -> 001e
    let req = "host:forward:tcp:0;tcp:8080";
    let req_framed = format!("{:04x}{req}", req.len());
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(req_framed.as_bytes())
        .await
        .expect("write");

    let n = stream.read(&mut buf).await.expect("read response");
    // Host forward response has two OKAYs: 1st for connect/route, 2nd for status, followed by port protocol string
    assert!(n >= 8);
    assert_eq!(&buf[0..4], b"OKAY");
    assert_eq!(&buf[4..8], b"OKAY");
    let (proto_str, _) = parse_protocol_string(&buf[8..n]).expect("parse port");
    let allocated_port: u16 = match proto_str {
        ProtocolString::Complete(port_str) => port_str.parse().expect("port number"),
        other => panic!("expected Complete, got {other:?}"),
    };
    assert!(allocated_port > 0);

    // 3. List-forward now shows the forward rule: "<serial> tcp:<port> tcp:8080\n"
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0011host:list-forward")
        .await
        .expect("write");
    let n = stream.read(&mut buf).await.expect("read response");
    let (reply, consumed) = parse_reply(&buf[..n]).expect("parse reply");
    assert_eq!(reply, SmartSocketReply::Okay);
    let (proto_str, _) = parse_protocol_string(&buf[consumed..n]).expect("parse payload");
    match proto_str {
        ProtocolString::Complete(list) => {
            let expected_line = format!("device-1 tcp:{allocated_port} tcp:8080\n");
            assert_eq!(list, expected_line);
        }
        other => panic!("expected Complete, got {other:?}"),
    }

    // 4. Try forward with norebind on same local port -> should fail with "cannot rebind existing socket"
    let norebind_req = format!("host:forward:norebind:tcp:{allocated_port};tcp:9090");
    let norebind_framed = format!("{:04x}{norebind_req}", norebind_req.len());
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(norebind_framed.as_bytes())
        .await
        .expect("write");
    let n = stream.read(&mut buf).await.expect("read response");
    let (reply, _) = parse_reply(&buf[..n]).expect("parse reply");
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "cannot rebind existing socket");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    // 5. Rebind without norebind flag -> succeeds
    let rebind_req = format!("host:forward:tcp:{allocated_port};tcp:9090");
    let rebind_framed = format!("{:04x}{rebind_req}", rebind_req.len());
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(rebind_framed.as_bytes())
        .await
        .expect("write");
    let _n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[0..4], b"OKAY");
    assert_eq!(&buf[4..8], b"OKAY");

    // 6. Remove specific forward listener
    let kill_req = format!("host:killforward:tcp:{allocated_port}");
    let kill_framed = format!("{:04x}{kill_req}", kill_req.len());
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(kill_framed.as_bytes())
        .await
        .expect("write");
    let _n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[0..4], b"OKAY");
    assert_eq!(&buf[4..8], b"OKAY");

    // 7. Verify it is removed
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0011host:list-forward")
        .await
        .expect("write");
    let n = stream.read(&mut buf).await.expect("read response");
    let (reply, consumed) = parse_reply(&buf[..n]).expect("parse reply");
    assert_eq!(reply, SmartSocketReply::Okay);
    let (proto_str, _) = parse_protocol_string(&buf[consumed..n]).expect("parse payload");
    match proto_str {
        ProtocolString::Complete(list) => assert_eq!(list, ""),
        other => panic!("expected Complete, got {other:?}"),
    }

    // 8. Removing nonexistent listener returns listener not found error
    let kill_nonexistent = "host:killforward:tcp:64000";
    let kill_framed = format!("{:04x}{kill_nonexistent}", kill_nonexistent.len());
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(kill_framed.as_bytes())
        .await
        .expect("write");
    let n = stream.read(&mut buf).await.expect("read response");
    let (reply, _) = parse_reply(&buf[..n]).expect("parse reply");
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert!(
                reason.contains("listener 'tcp:64000' not found") || reason.contains("not found"),
                "unexpected fail reason: {reason}"
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn test_forward_proxy_traffic_end_to_end() {
    let registry = Arc::new(DeviceRegistry::new());
    let dev_id = registry
        .register("device-echo", TransportType::Local)
        .expect("register device");
    registry.update_state(dev_id, ConnectionState::Device);

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let server_addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // Start a mock target TCP echo server representing tcp:8080 service on the device
    let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind target listener");
    let target_port = target_listener.local_addr().unwrap().port();

    let echo_server_task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = target_listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = socket.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    if socket.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    // Forward local tcp:0 -> tcp:<target_port>
    let req = format!("host:forward:tcp:0;tcp:{target_port}");
    let req_framed = format!("{:04x}{req}", req.len());
    let mut stream = TcpStream::connect(server_addr).await.expect("connect");
    stream
        .write_all(req_framed.as_bytes())
        .await
        .expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[0..4], b"OKAY");
    assert_eq!(&buf[4..8], b"OKAY");
    let (proto_str, _) = parse_protocol_string(&buf[8..n]).expect("parse port");
    let local_port: u16 = match proto_str {
        ProtocolString::Complete(port_str) => port_str.parse().expect("port number"),
        other => panic!("expected Complete, got {other:?}"),
    };

    // Now connect to the local forwarded port and verify traffic is proxied end-to-end to target echo server!
    let mut client = TcpStream::connect(format!("127.0.0.1:{local_port}"))
        .await
        .expect("connect to forwarded port");

    let msg = b"hello forwarded port";
    client.write_all(msg).await.expect("write to client");
    let mut recv_buf = vec![0u8; msg.len()];
    client.read_exact(&mut recv_buf).await.expect("read echo");
    assert_eq!(&recv_buf, msg);

    server_task.abort();
    echo_server_task.abort();
}
