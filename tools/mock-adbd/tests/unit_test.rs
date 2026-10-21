use adb_proto::{
    Banner, Command, Feature, ShellDecoder, ShellPacket, ShellPacketId, WindowSize, A_VERSION,
    MAX_PAYLOAD,
};
use mock_adbd::{read_packet, write_packet, MockAdbdOptions, MockAdbdServer};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_mock_adbd_banner_features() {
    // Default server advertises shell_v2
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut reader, mut writer) = client.split();

    write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();

    let (_, cnxn_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    let banner = Banner::parse(&String::from_utf8_lossy(&cnxn_payload));
    assert!(banner.features.contains(&Feature::Shell2));
    server.shutdown();

    // Server with enable_shell_v2 = false does not advertise shell_v2
    let server_no_v2 = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            enable_shell_v2: false,
        },
    )
    .await
    .unwrap();
    let addr_no_v2 = server_no_v2.local_addr();

    let mut client_no_v2 = TcpStream::connect(addr_no_v2).await.unwrap();
    let (mut reader_no_v2, mut writer_no_v2) = client_no_v2.split();

    write_packet(
        &mut writer_no_v2,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();

    let (_, cnxn_payload_no_v2) = read_packet(&mut reader_no_v2).await.unwrap().unwrap();
    let banner_no_v2 = Banner::parse(&String::from_utf8_lossy(&cnxn_payload_no_v2));
    assert!(!banner_no_v2.features.contains(&Feature::Shell2));
    server_no_v2.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_echo_and_exit() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut reader, mut writer) = client.split();

    write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();
    let _ = read_packet(&mut reader).await.unwrap();

    // Open shell,v2,raw:echo hello
    let host_id = 100u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_id,
        0,
        b"shell,v2,raw:echo hello\0",
    )
    .await
    .unwrap();

    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(okay.command, Command::Okay);
    let mock_id = okay.arg0;

    let mut decoder = ShellDecoder::new();

    // 1st WRTE: stdout packet "hello\n"
    let (wrte1, payload1) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte1.command, Command::Wrte);
    decoder.feed(&payload1);
    let pkt1 = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt1.id, ShellPacketId::Stdout);
    assert_eq!(String::from_utf8_lossy(&pkt1.data), "hello\n");

    // Ack 1st WRTE
    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 2nd WRTE: exit code 0 packet
    let (wrte2, payload2) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte2.command, Command::Wrte);
    decoder.feed(&payload2);
    let pkt2 = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt2.id, ShellPacketId::Exit);
    assert_eq!(pkt2.parse_exit_code().unwrap(), 0);

    // Ack 2nd WRTE
    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // CLSE packet
    let (clse, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse.command, Command::Clse);

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_stderr_and_env_term() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut reader, mut writer) = client.split();

    write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();
    let _ = read_packet(&mut reader).await.unwrap();

    // 1. stderr command
    let host_id = 101u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_id,
        0,
        b"shell,v2,raw:stderr oops\0",
    )
    .await
    .unwrap();

    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;
    let mut decoder = ShellDecoder::new();

    let (_, payload1) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&payload1);
    let pkt1 = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt1.id, ShellPacketId::Stderr);
    assert_eq!(String::from_utf8_lossy(&pkt1.data), "oops\n");

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    let (_, payload2) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&payload2);
    let pkt2 = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt2.id, ShellPacketId::Exit);
    assert_eq!(pkt2.parse_exit_code().unwrap(), 0);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    let (clse, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse.command, Command::Clse);

    // 2. env-term command with TERM=xterm-256color
    let host_id2 = 102u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_id2,
        0,
        b"shell,v2,TERM=xterm-256color,raw:env-term\0",
    )
    .await
    .unwrap();

    let (okay2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id2 = okay2.arg0;

    let (_, payload_term) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&payload_term);
    let pkt_term = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_term.id, ShellPacketId::Stdout);
    assert_eq!(String::from_utf8_lossy(&pkt_term.data), "xterm-256color\n");

    write_packet(&mut writer, Command::Okay, host_id2, mock_id2, &[])
        .await
        .unwrap();

    let (_, payload_exit) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&payload_exit);
    let pkt_exit = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_exit.id, ShellPacketId::Exit);
    assert_eq!(pkt_exit.parse_exit_code().unwrap(), 0);

    write_packet(&mut writer, Command::Okay, host_id2, mock_id2, &[])
        .await
        .unwrap();

    let (clse2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse2.command, Command::Clse);

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_cat_and_pty_winsz() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut reader, mut writer) = client.split();

    write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();
    let _ = read_packet(&mut reader).await.unwrap();

    // Open shell,v2,pty:cat
    let host_id = 103u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_id,
        0,
        b"shell,v2,pty:cat\0",
    )
    .await
    .unwrap();

    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;
    let mut decoder = ShellDecoder::new();

    // 1. Send WindowSizeChange packet
    let winsz_pkt = ShellPacket::window_size_change(WindowSize {
        rows: 24,
        cols: 80,
        x_pixels: 0,
        y_pixels: 0,
    });
    write_packet(
        &mut writer,
        Command::Wrte,
        host_id,
        mock_id,
        &winsz_pkt.encode_to_vec(),
    )
    .await
    .unwrap();

    // Mock acknowledges host WRTE with OKAY
    let (host_ack1, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(host_ack1.command, Command::Okay);

    // Mock replies with WINSZ stdout packet
    let (wrte_winsz, winsz_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_winsz.command, Command::Wrte);
    decoder.feed(&winsz_payload);
    let pkt_winsz = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_winsz.id, ShellPacketId::Stdout);
    assert_eq!(
        String::from_utf8_lossy(&pkt_winsz.data),
        "WINSZ 24x80,0x0\n"
    );

    // Host acknowledges mock WRTE
    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 2. Send Stdin packet "hello stdin"
    let stdin_pkt = ShellPacket::new(ShellPacketId::Stdin, b"hello stdin".to_vec());
    write_packet(
        &mut writer,
        Command::Wrte,
        host_id,
        mock_id,
        &stdin_pkt.encode_to_vec(),
    )
    .await
    .unwrap();

    let (host_ack2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(host_ack2.command, Command::Okay);

    let (_, cat_stdout_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&cat_stdout_payload);
    let pkt_stdout = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_stdout.id, ShellPacketId::Stdout);
    assert_eq!(String::from_utf8_lossy(&pkt_stdout.data), "hello stdin");

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 3. Send CloseStdin packet
    let close_stdin_pkt = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
    write_packet(
        &mut writer,
        Command::Wrte,
        host_id,
        mock_id,
        &close_stdin_pkt.encode_to_vec(),
    )
    .await
    .unwrap();

    let (host_ack3, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(host_ack3.command, Command::Okay);

    // Expect exit code 0 packet
    let (_, exit_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&exit_payload);
    let pkt_exit = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_exit.id, ShellPacketId::Exit);
    assert_eq!(pkt_exit.parse_exit_code().unwrap(), 0);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    let (clse, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse.command, Command::Clse);

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_unsupported_command() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    let mut client = TcpStream::connect(addr).await.unwrap();
    let (mut reader, mut writer) = client.split();

    write_packet(
        &mut writer,
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        b"host::",
    )
    .await
    .unwrap();
    let _ = read_packet(&mut reader).await.unwrap();

    let host_id = 104u32;
    write_packet(
        &mut writer,
        Command::Open,
        host_id,
        0,
        b"shell,v2,raw:unknown_cmd\0",
    )
    .await
    .unwrap();

    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    let mut decoder = ShellDecoder::new();

    let (_, stderr_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&stderr_payload);
    let pkt_stderr = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_stderr.id, ShellPacketId::Stderr);
    assert_eq!(
        String::from_utf8_lossy(&pkt_stderr.data),
        "mock-adbd: unsupported command\n"
    );

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    let (_, exit_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    decoder.feed(&exit_payload);
    let pkt_exit = decoder.pop_packet().unwrap().unwrap();
    assert_eq!(pkt_exit.id, ShellPacketId::Exit);
    assert_eq!(pkt_exit.parse_exit_code().unwrap(), 127);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    let (clse, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse.command, Command::Clse);

    server.shutdown();
}
