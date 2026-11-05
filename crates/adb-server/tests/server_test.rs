use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use adb_proto::smart_socket::{parse_protocol_string, parse_reply, Decoded, SmartSocketReply};
use adb_server::{ConnectionState, DeviceRegistry, SmartSocketServer, TransportType};

#[tokio::test]
async fn test_smart_socket_valid_length_request_routed() {
    // Valid length request: 000C123456789012 parsed and routed.
    // 0x000C = 12 bytes. "123456789012" is a non-host device service request.
    // When no transport has been selected on this socket connection, matching C++ ADB behavior,
    // the server replies with FAIL payload: "device offline (no transport)" and terminates connection.
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(b"000C123456789012").await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert!(n > 0, "server should reply before closing socket");

    let Decoded::Complete {
        value: reply,
        consumed,
    } = parse_reply(&buf[..n]).expect("parse smart socket reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(consumed, n);
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "device offline (no transport)");
        }
        other => panic!("expected Fail, got: {other:?}"),
    }

    // Verify stream was closed by server
    let next_read = stream.read(&mut buf).await.expect("read eof");
    assert_eq!(next_read, 0, "client socket should be closed after failure");

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_target_selection_by_serial() {
    let registry = Arc::new(DeviceRegistry::new());
    let id1 = registry
        .register("123456", TransportType::Usb)
        .expect("register usb");
    registry.update_state(id1, ConnectionState::Device);

    let id2 = registry
        .register("789012", TransportType::Local)
        .expect("register local");
    registry.update_state(id2, ConnectionState::Device);

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let req = "host-serial:123456:get-serialno";
    let framed = format!("{:04x}{req}", req.len());
    stream.write_all(framed.as_bytes()).await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let Decoded::Complete {
        value: reply,
        consumed,
    } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(reply, SmartSocketReply::Okay);
    let proto_str: Decoded<String> =
        parse_protocol_string(&buf[consumed..n]).expect("parse payload");
    match proto_str {
        Decoded::Complete { value: s, .. } => assert_eq!(s, "123456"),
        other => panic!("expected Complete, got {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_target_ambiguous_error() {
    let registry = Arc::new(DeviceRegistry::new());
    let id1 = registry
        .register("dev1", TransportType::Usb)
        .expect("register dev1");
    registry.update_state(id1, ConnectionState::Device);

    let id2 = registry
        .register("dev2", TransportType::Local)
        .expect("register dev2");
    registry.update_state(id2, ConnectionState::Device);

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // 1. host:get-state without selector when 2 devices connected returns "more than one device/emulator"
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let req = "host:get-state";
    let framed = format!("{:04x}{req}", req.len());
    stream.write_all(framed.as_bytes()).await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply, .. } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "more than one device/emulator");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_missing_device_errors() {
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // 1. host-serial:nonexistent:get-state returns "device 'nonexistent' not found"
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let req = "host-serial:nonexistent:get-state";
    let framed = format!("{:04x}{req}", req.len());
    stream.write_all(framed.as_bytes()).await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply, .. } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "device 'nonexistent' not found");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    // 2. host:get-state when no devices connected returns "no devices/emulators found"
    let mut stream2 = TcpStream::connect(addr).await.expect("connect");
    let req2 = "host:get-state";
    let framed2 = format!("{:04x}{req2}", req2.len());
    stream2.write_all(framed2.as_bytes()).await.expect("write");

    let n2 = stream2.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply2, .. } = parse_reply(&buf[..n2]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply2 {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "no devices/emulators found");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    // 3. host-usb:get-state when no devices connected returns "no devices found"
    let mut stream3 = TcpStream::connect(addr).await.expect("connect");
    let req3 = "host-usb:get-state";
    let framed3 = format!("{:04x}{req3}", req3.len());
    stream3.write_all(framed3.as_bytes()).await.expect("write");

    let n3 = stream3.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply3, .. } = parse_reply(&buf[..n3]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply3 {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "no devices found");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    // 4. host-local:get-state when no devices connected returns "no emulators found"
    let mut stream4 = TcpStream::connect(addr).await.expect("connect");
    let req4 = "host-local:get-state";
    let framed4 = format!("{:04x}{req4}", req4.len());
    stream4.write_all(framed4.as_bytes()).await.expect("write");

    let n4 = stream4.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply4, .. } = parse_reply(&buf[..n4]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply4 {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "no emulators found");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    // 5. host-transport-id:99:get-state when no device returns "no device with transport id '99'"
    let mut stream5 = TcpStream::connect(addr).await.expect("connect");
    let req5 = "host-transport-id:99:get-state";
    let framed5 = format!("{:04x}{req5}", req5.len());
    stream5.write_all(framed5.as_bytes()).await.expect("write");

    let n5 = stream5.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply5, .. } = parse_reply(&buf[..n5]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply5 {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "no device with transport id '99'");
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_malformed_length_prefix_returns_fail() {
    // Malformed length prefix: Non-hex characters in prefix returns FAIL payload and closes client socket.
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(b"ZZZZhello").await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert!(
        n > 0,
        "server should reply with FAIL payload before closing"
    );

    let Decoded::Complete {
        value: reply,
        consumed,
    } = parse_reply(&buf[..n]).expect("parse smart socket reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(consumed, n);
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert!(
                reason.contains("length") || reason.contains("hex"),
                "fail reason should explain malformed length: {reason}"
            );
        }
        other => panic!("expected Fail, got: {other:?}"),
    }

    // Verify socket closed: either clean EOF (0 bytes) or ConnectionReset confirms closure by peer
    match stream.read(&mut buf).await {
        Ok(0) => {}
        Err(e)
            if e.kind() == std::io::ErrorKind::ConnectionReset
                || e.kind() == std::io::ErrorKind::BrokenPipe => {}
        other => {
            panic!("expected eof or connection reset confirming socket closure, got: {other:?}")
        }
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_host_version() {
    // host:version returns OKAY00040029
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(b"000chost:version").await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[..n], b"OKAY00040029");

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_host_version_mismatch_triggers_clean_exit() {
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // Incompatible client sends host:version:0028 or host:version:40
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0011host:version:0028")
        .await
        .expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("FAIL"),
        "Expected FAIL mismatch reply, got: {resp}"
    );
    assert!(
        resp.to_lowercase().contains("mismatch")
            || resp.contains("version")
            || resp.contains("client"),
        "Expected mismatch error message in FAIL reply, got: {resp}"
    );

    // Server should shut down cleanly
    let server_result = tokio::time::timeout(std::time::Duration::from_secs(2), server_task).await;
    assert!(
        server_result.is_ok(),
        "Server should have exited cleanly on version mismatch"
    );
}

#[tokio::test]
async fn test_smart_socket_host_version_matching_client_succeeds() {
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0011host:version:0029")
        .await
        .expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[..n], b"OKAY00040029");

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_unknown_host_service() {
    // Unrecognized host service returns FAIL + "unknown host service '<cmd>'"
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let cmd = "host:nonexistent_service_12345";
    let req = format!("{:04x}{cmd}", cmd.len());
    stream.write_all(req.as_bytes()).await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");

    let Decoded::Complete {
        value: reply,
        consumed,
    } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(consumed, n);
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "unknown host service 'nonexistent_service_12345'");
        }
        other => panic!("expected Fail, got: {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_transport_switching_and_stream_routing() {
    let registry = Arc::new(DeviceRegistry::new());
    let _id = registry
        .register("emulator-5554", TransportType::Local)
        .expect("register device");

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");

    // 1. Switch transport to emulator-5554
    let switch_cmd = "host:transport:emulator-5554";
    let switch_req = format!("{:04x}{switch_cmd}", switch_cmd.len());
    stream
        .write_all(switch_req.as_bytes())
        .await
        .expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[..n], b"OKAY");

    // 2. Now on the same connection, send device service: 000C123456789012
    stream.write_all(b"000C123456789012").await.expect("write");

    // Because device stream routing is invoked for the selected transport,
    // we get a routed response (e.g. OKAY or stream handling) instead of "device offline (no transport)".
    let n2 = stream.read(&mut buf).await.expect("read response 2");
    assert!(n2 > 0);
    // Transport was selected, so error is NOT "device offline (no transport)"
    if let Ok(Decoded::Complete {
        value: SmartSocketReply::Fail(reason),
        ..
    }) = parse_reply(&buf[..n2])
    {
        assert_ne!(reason, "device offline (no transport)");
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_host_kill() {
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(b"0009host:kill").await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[..n], b"OKAY");

    // Server run loop should terminate cleanly upon kill request
    server_task.await.expect("server task join");
}

#[tokio::test]
async fn test_smart_socket_host_devices_empty() {
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(b"000chost:devices").await.expect("write");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    assert_eq!(&buf[..n], b"OKAY0000");

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_host_devices_multiple() {
    let registry = Arc::new(DeviceRegistry::new());
    let id1 = registry
        .register("serial-usb-1", TransportType::Usb)
        .expect("register usb");
    registry.update_state(id1, ConnectionState::Device);
    registry.set_device_properties(
        id1,
        Some("mock_product".to_string()),
        Some("mock_model".to_string()),
        Some("mock_device".to_string()),
    );

    let id2 = registry
        .register("127.0.0.1:5555", TransportType::Local)
        .expect("register tcp");
    registry.update_state(id2, ConnectionState::Device);

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // Test host:devices (short format)
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream.write_all(b"000chost:devices").await.expect("write");
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let Decoded::Complete {
        value: reply,
        consumed,
    } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(reply, SmartSocketReply::Okay);
    let proto_str: Decoded<String> =
        parse_protocol_string(&buf[consumed..n]).expect("parse payload");
    match proto_str {
        Decoded::Complete { value: payload, .. } => {
            assert_eq!(payload, "serial-usb-1\tdevice\n127.0.0.1:5555\tdevice\n");
        }
        other => panic!("expected Complete, got: {other:?}"),
    }

    // Test host:devices-l (long format)
    let mut stream_l = TcpStream::connect(addr).await.expect("connect");
    stream_l
        .write_all(b"000ehost:devices-l")
        .await
        .expect("write");
    let n_l = stream_l.read(&mut buf).await.expect("read response");
    let Decoded::Complete {
        value: reply_l,
        consumed: consumed_l,
    } = parse_reply(&buf[..n_l]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(reply_l, SmartSocketReply::Okay);
    let proto_str_l: Decoded<String> =
        parse_protocol_string(&buf[consumed_l..n_l]).expect("parse payload");
    match proto_str_l {
        Decoded::Complete { value: payload, .. } => {
            assert!(
                payload.contains("serial-usb-1           device product:mock_product model:mock_model device:mock_device transport_id:1\n"),
                "devices-l should contain tabular info for serial-usb-1: {payload}"
            );
            assert!(
                payload.contains("127.0.0.1:5555         device transport_id:2\n"),
                "devices-l should contain info for 127.0.0.1:5555: {payload}"
            );
        }
        other => panic!("expected Complete, got: {other:?}"),
    }

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_host_disconnect() {
    let registry = Arc::new(DeviceRegistry::new());
    let _id = registry
        .register("127.0.0.1:5555", TransportType::Local)
        .expect("register");

    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // 1. Disconnect non-existent device returns FAIL
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0017host:disconnect:1.2.3.4")
        .await
        .expect("write");
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let Decoded::Complete { value: reply, .. } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    match reply {
        SmartSocketReply::Fail(reason) => {
            assert_eq!(reason, "no such device '1.2.3.4:5555'");
        }
        other => panic!("expected Fail, got: {other:?}"),
    }

    // 2. Disconnect existing device returns OKAY + "disconnected 127.0.0.1:5555"
    let mut stream2 = TcpStream::connect(addr).await.expect("connect");
    stream2
        .write_all(b"001ehost:disconnect:127.0.0.1:5555")
        .await
        .expect("write");
    let n2 = stream2.read(&mut buf).await.expect("read response");
    let Decoded::Complete {
        value: reply2,
        consumed: consumed2,
    } = parse_reply(&buf[..n2]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(reply2, SmartSocketReply::Okay);
    let proto_str2: Decoded<String> =
        parse_protocol_string(&buf[consumed2..n2]).expect("parse payload");
    match proto_str2 {
        Decoded::Complete { value: msg, .. } => {
            assert_eq!(msg, "disconnected 127.0.0.1:5555");
        }
        other => panic!("expected Complete, got: {other:?}"),
    }

    // Verify it was unregistered from registry
    assert!(registry.get_by_serial("127.0.0.1:5555").is_none());

    // 3. Disconnect all (empty target)
    let _id3 = registry
        .register("127.0.0.1:5557", TransportType::Local)
        .expect("register");
    let mut stream3 = TcpStream::connect(addr).await.expect("connect");
    stream3
        .write_all(b"0010host:disconnect:")
        .await
        .expect("write");
    let n3 = stream3.read(&mut buf).await.expect("read response");
    let Decoded::Complete {
        value: reply3,
        consumed: consumed3,
    } = parse_reply(&buf[..n3]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(reply3, SmartSocketReply::Okay);
    let proto_str3: Decoded<String> =
        parse_protocol_string(&buf[consumed3..n3]).expect("parse payload");
    match proto_str3 {
        Decoded::Complete { value: msg, .. } => {
            assert_eq!(msg, "disconnected everything");
        }
        other => panic!("expected Complete, got: {other:?}"),
    }
    assert!(registry.list_devices().is_empty());

    server_task.abort();
}

#[tokio::test]
async fn test_smart_socket_host_connect() {
    let registry = Arc::new(DeviceRegistry::new());
    let server = SmartSocketServer::bind("127.0.0.1:0", Arc::clone(&registry))
        .await
        .expect("bind server");
    let addr = server.local_addr().expect("local addr");

    let server_task = tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // Connecting to an unreachable port returns OKAY with failure message matching C++ ADB:
    // "failed to connect to '...': Connection refused"
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(b"0018host:connect:127.0.0.1:1")
        .await
        .expect("write");
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let Decoded::Complete {
        value: reply,
        consumed,
    } = parse_reply(&buf[..n]).expect("parse reply")
    else {
        panic!("incomplete")
    };
    assert_eq!(reply, SmartSocketReply::Okay);
    let proto_str: Decoded<String> =
        parse_protocol_string(&buf[consumed..n]).expect("parse payload");
    match proto_str {
        Decoded::Complete { value: msg, .. } => {
            assert!(
                msg.starts_with("failed to connect to '127.0.0.1:1'"),
                "unexpected message: {msg}"
            );
        }
        other => panic!("expected Complete, got: {other:?}"),
    }

    server_task.abort();
}
