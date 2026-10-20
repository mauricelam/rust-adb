use adb_proto::{Banner, Command, Feature, A_VERSION, MAX_PAYLOAD};
use mock_adbd::{read_packet, write_packet, MockAdbdServer};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_mock_adbd_in_process_protocol() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut reader, mut writer) = client.split();

    // 1. Handshake
    write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();

    let (cnxn_hdr, cnxn_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(cnxn_hdr.command, Command::Cnxn);
    assert_eq!(cnxn_hdr.arg0, A_VERSION);

    let banner_str = String::from_utf8_lossy(&cnxn_payload);
    let banner = Banner::parse(&banner_str);
    assert_eq!(banner.product(), Some("mock_device"));
    assert_eq!(banner.model(), Some("mock_model"));
    assert_eq!(banner.device(), Some("mock_device"));
    assert!(!banner.features.contains(&Feature::Shell2));

    // 2. Stream Open & WRTE/OKAY ordering for echo
    let host_local_id = 100u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_local_id,
        0,
        b"shell:echo hello world\0",
    )
    .await
    .unwrap();

    // Expect OKAY from mock accepting OPEN
    let (okay_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(okay_hdr.command, Command::Okay);
    assert_eq!(okay_hdr.arg1, host_local_id);
    let mock_local_id = okay_hdr.arg0;
    assert!(mock_local_id > 0);

    // Expect WRTE from mock with command output
    let (wrte_hdr, wrte_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_hdr.command, Command::Wrte);
    assert_eq!(wrte_hdr.arg0, mock_local_id);
    assert_eq!(wrte_hdr.arg1, host_local_id);
    assert_eq!(String::from_utf8_lossy(&wrte_payload), "hello world\n");

    // Send OKAY acknowledging WRTE (1-WRTE-in-flight rule)
    write_packet(
        &mut writer,
        Command::Okay,
        host_local_id,
        mock_local_id,
        &[],
    )
    .await
    .unwrap();

    // Expect CLSE from mock closing the stream
    let (clse_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse_hdr.command, Command::Clse);
    assert_eq!(clse_hdr.arg0, mock_local_id);
    assert_eq!(clse_hdr.arg1, host_local_id);

    // 3. Stream Open for unsupported shell command
    let host_local_id2 = 101u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_local_id2,
        0,
        b"shell:ls -l\0",
    )
    .await
    .unwrap();

    let (okay_hdr2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(okay_hdr2.command, Command::Okay);
    let mock_local_id2 = okay_hdr2.arg0;

    let (wrte_hdr2, wrte_payload2) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_hdr2.command, Command::Wrte);
    assert_eq!(
        String::from_utf8_lossy(&wrte_payload2),
        "mock-adbd: unsupported command\n"
    );

    write_packet(
        &mut writer,
        Command::Okay,
        host_local_id2,
        mock_local_id2,
        &[],
    )
    .await
    .unwrap();

    let (clse_hdr2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse_hdr2.command, Command::Clse);

    // 4. Unknown service stream refusal
    let unknown_host_id = 200u32;
    write_packet(
        &mut writer,
        Command::Open,
        unknown_host_id,
        0,
        b"unsupported_service:foo\0",
    )
    .await
    .unwrap();

    // Expect CLSE(0, unknown_host_id) refusing stream
    let (refuse_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(refuse_hdr.command, Command::Clse);
    assert_eq!(refuse_hdr.arg0, 0);
    assert_eq!(refuse_hdr.arg1, unknown_host_id);

    server.shutdown();
}
