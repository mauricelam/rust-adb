use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use adb_proto::smart_socket::{parse_reply, SmartSocketReply};
use adb_server::{DeviceRegistry, SmartSocketServer, TransportType};

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

    let (reply, consumed) = parse_reply(&buf[..n]).expect("parse smart socket reply");
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

    let (reply, consumed) = parse_reply(&buf[..n]).expect("parse smart socket reply");
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

    let (reply, consumed) = parse_reply(&buf[..n]).expect("parse reply");
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
    if let Ok((SmartSocketReply::Fail(reason), _)) = parse_reply(&buf[..n2]) {
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
