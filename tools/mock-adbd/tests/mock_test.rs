use adb_proto::{Apacket, A_CLSE, A_CNXN, A_OKAY, A_OPEN, A_VERSION, A_WRTE, MAX_PAYLOAD};
use mock_adbd::{read_packet, write_packet, MockAdbdServer};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_mock_adbd_handshake_and_shell_echo() {
    let server = MockAdbdServer::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");
    let addr = server.local_addr();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut client = TcpStream::connect(addr)
        .await
        .expect("Failed to connect to mock adbd");

    // 1. Send host CNXN
    let host_cnxn = Apacket::new(A_CNXN, A_VERSION, MAX_PAYLOAD, b"host::".to_vec());
    write_packet(&mut client, &host_cnxn)
        .await
        .expect("Failed to write host CNXN");

    // 2. Read device CNXN response
    let dev_cnxn = read_packet(&mut client)
        .await
        .expect("Failed to read device CNXN")
        .expect("Stream closed unexpectedly");

    assert_eq!(dev_cnxn.msg.command, A_CNXN);
    assert_eq!(dev_cnxn.msg.arg0, A_VERSION);
    let banner_str = String::from_utf8_lossy(&dev_cnxn.payload);
    assert!(banner_str.starts_with("device::"));

    // 3. Send OPEN shell:echo hello
    let host_open = Apacket::new(A_OPEN, 100, 0, b"shell:echo hello\0".to_vec());
    write_packet(&mut client, &host_open)
        .await
        .expect("Failed to write OPEN");

    // 4. Read OKAY response
    let dev_okay = read_packet(&mut client)
        .await
        .expect("Failed to read OKAY")
        .expect("Stream closed unexpectedly");
    assert_eq!(dev_okay.msg.command, A_OKAY);
    let mock_stream_id = dev_okay.msg.arg0;
    assert_eq!(dev_okay.msg.arg1, 100);

    // 5. Read WRTE response
    let dev_wrte = read_packet(&mut client)
        .await
        .expect("Failed to read WRTE")
        .expect("Stream closed unexpectedly");
    assert_eq!(dev_wrte.msg.command, A_WRTE);
    assert_eq!(dev_wrte.msg.arg0, mock_stream_id);
    assert_eq!(dev_wrte.msg.arg1, 100);
    assert_eq!(String::from_utf8_lossy(&dev_wrte.payload), "hello\n");

    // Verify 1-WRTE-in-flight: ensure mock does not send anything further before receiving host OKAY
    let timeout_res = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        read_packet(&mut client),
    )
    .await;
    assert!(
        timeout_res.is_err(),
        "Mock sent a second packet prior to host OKAY ack"
    );

    // 6. Send host OKAY ack for the WRTE
    let host_okay = Apacket::new(A_OKAY, 100, mock_stream_id, vec![]);
    write_packet(&mut client, &host_okay)
        .await
        .expect("Failed to write OKAY ack");

    // 7. Read device CLSE
    let dev_clse = read_packet(&mut client)
        .await
        .expect("Failed to read CLSE")
        .expect("Stream closed unexpectedly");
    assert_eq!(dev_clse.msg.command, A_CLSE);
    assert_eq!(dev_clse.msg.arg0, mock_stream_id);
    assert_eq!(dev_clse.msg.arg1, 100);
}

#[tokio::test]
async fn test_mock_adbd_unsupported_command() {
    let server = MockAdbdServer::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");
    let addr = server.local_addr();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut client = TcpStream::connect(addr).await.unwrap();

    // Handshake
    let host_cnxn = Apacket::new(A_CNXN, A_VERSION, MAX_PAYLOAD, b"host::".to_vec());
    write_packet(&mut client, &host_cnxn).await.unwrap();
    let _ = read_packet(&mut client).await.unwrap();

    // Send unsupported shell command
    let host_open = Apacket::new(A_OPEN, 200, 0, b"shell:ls -la\0".to_vec());
    write_packet(&mut client, &host_open).await.unwrap();

    // Read OKAY
    let dev_okay = read_packet(&mut client).await.unwrap().unwrap();
    assert_eq!(dev_okay.msg.command, A_OKAY);
    let mock_stream_id = dev_okay.msg.arg0;

    // Read WRTE with unsupported message
    let dev_wrte = read_packet(&mut client).await.unwrap().unwrap();
    assert_eq!(dev_wrte.msg.command, A_WRTE);
    assert_eq!(
        String::from_utf8_lossy(&dev_wrte.payload),
        "mock-adbd: unsupported command\n"
    );

    // Send host OKAY ack
    let host_okay = Apacket::new(A_OKAY, 200, mock_stream_id, vec![]);
    write_packet(&mut client, &host_okay).await.unwrap();

    // Read device CLSE
    let dev_clse = read_packet(&mut client).await.unwrap().unwrap();
    assert_eq!(dev_clse.msg.command, A_CLSE);
}

#[tokio::test]
async fn test_mock_adbd_unknown_service() {
    let server = MockAdbdServer::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");
    let addr = server.local_addr();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut client = TcpStream::connect(addr).await.unwrap();

    // Handshake
    let host_cnxn = Apacket::new(A_CNXN, A_VERSION, MAX_PAYLOAD, b"host::".to_vec());
    write_packet(&mut client, &host_cnxn).await.unwrap();
    let _ = read_packet(&mut client).await.unwrap();

    // Send unknown service OPEN request
    let host_open = Apacket::new(A_OPEN, 300, 0, b"unknown_service_name\0".to_vec());
    write_packet(&mut client, &host_open).await.unwrap();

    // Device MUST refuse directly with CLSE (arg0 = 0) and NO OKAY
    let dev_clse = read_packet(&mut client).await.unwrap().unwrap();
    assert_eq!(dev_clse.msg.command, A_CLSE);
    assert_eq!(dev_clse.msg.arg0, 0);
    assert_eq!(dev_clse.msg.arg1, 300);
}

#[tokio::test]
async fn test_mock_adbd_malformed_packet_no_panic() {
    use tokio::io::AsyncWriteExt;

    let server = MockAdbdServer::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind server");
    let addr = server.local_addr();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    let mut client = TcpStream::connect(addr).await.unwrap();

    // Send malformed junk bytes
    client
        .write_all(b"invalid_garbage_packet_header_bytes")
        .await
        .unwrap();

    // Server should close connection gracefully without panicking
    let res = read_packet(&mut client).await;
    assert!(res.is_err() || res.unwrap().is_none());
}
