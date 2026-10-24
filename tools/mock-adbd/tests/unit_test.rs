use adb_proto::{
    Banner, Command, Feature, ShellDecoder, ShellPacket, ShellPacketId, WindowSize, A_VERSION,
    A_VERSION_MIN, MAX_PAYLOAD,
};
use mock_adbd::{read_packet, write_packet, MockAdbdOptions, MockAdbdServer};
use tokio::net::TcpStream;

#[tokio::test]
async fn test_stale_clse_does_not_abort_next_stream() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
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

        // 1. Open Stream 1 running `echo a`
        let host_id_1 = 100u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_1,
            0,
            b"shell,v2,raw:echo a\0",
        )
        .await
        .unwrap();

        let (okay1, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay1.command, Command::Okay);
        let mock_id_1 = okay1.arg0;

        let mut decoder = ShellDecoder::new();

        let (wrte1, payload1) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte1.command, Command::Wrte);
        decoder.feed(&payload1);
        let pkt1 = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(pkt1.id, ShellPacketId::Stdout);
        assert_eq!(String::from_utf8_lossy(&pkt1.data), "a\n");
        write_packet(&mut writer, Command::Okay, host_id_1, mock_id_1, &[])
            .await
            .unwrap();

        let (wrte2, payload2) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte2.command, Command::Wrte);
        decoder.feed(&payload2);
        let pkt2 = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(pkt2.id, ShellPacketId::Exit);
        assert_eq!(pkt2.parse_exit_code().unwrap(), 0);
        write_packet(&mut writer, Command::Okay, host_id_1, mock_id_1, &[])
            .await
            .unwrap();

        let (clse1, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(clse1.command, Command::Clse);

        // 2. Open Stream 2 running `echo b` and immediately send host's CLSE for Stream 1
        let host_id_2 = 101u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_2,
            0,
            b"shell,v2,raw:echo b\0",
        )
        .await
        .unwrap();

        write_packet(&mut writer, Command::Clse, host_id_1, mock_id_1, &[])
            .await
            .unwrap();

        let (okay2, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay2.command, Command::Okay);
        let mock_id_2 = okay2.arg0;

        let (wrte_b1, payload_b1) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte_b1.command, Command::Wrte);
        decoder.feed(&payload_b1);
        let pkt_b1 = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(pkt_b1.id, ShellPacketId::Stdout);
        assert_eq!(String::from_utf8_lossy(&pkt_b1.data), "b\n");
        write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
            .await
            .unwrap();

        let (wrte_b2, payload_b2) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte_b2.command, Command::Wrte);
        decoder.feed(&payload_b2);
        let pkt_b2 = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(pkt_b2.id, ShellPacketId::Exit);
        assert_eq!(pkt_b2.parse_exit_code().unwrap(), 0);
        write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
            .await
            .unwrap();

        let (clse2, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(clse2.command, Command::Clse);

        // 3. Repeat with `cat` for Stream 3
        let host_id_3 = 102u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_3,
            0,
            b"shell,v2,raw:cat\0",
        )
        .await
        .unwrap();

        write_packet(&mut writer, Command::Clse, host_id_2, mock_id_2, &[])
            .await
            .unwrap();

        let (okay3, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay3.command, Command::Okay);
        let mock_id_3 = okay3.arg0;

        let stdin = ShellPacket::new(ShellPacketId::Stdin, b"hello cat\n".to_vec());
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_3,
            mock_id_3,
            &stdin.encode_to_vec(),
        )
        .await
        .unwrap();

        let (ack_stdin, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(ack_stdin.command, Command::Okay);

        let (wrte_cat1, payload_cat1) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte_cat1.command, Command::Wrte);
        decoder.feed(&payload_cat1);
        let pkt_cat1 = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(pkt_cat1.id, ShellPacketId::Stdout);
        assert_eq!(String::from_utf8_lossy(&pkt_cat1.data), "hello cat\n");
        write_packet(&mut writer, Command::Okay, host_id_3, mock_id_3, &[])
            .await
            .unwrap();

        let close_stdin = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_3,
            mock_id_3,
            &close_stdin.encode_to_vec(),
        )
        .await
        .unwrap();

        let (ack_close, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(ack_close.command, Command::Okay);

        let (wrte_cat2, payload_cat2) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte_cat2.command, Command::Wrte);
        decoder.feed(&payload_cat2);
        let pkt_cat2 = decoder.pop_packet().unwrap().unwrap();
        assert_eq!(pkt_cat2.id, ShellPacketId::Exit);
        assert_eq!(pkt_cat2.parse_exit_code().unwrap(), 0);
        write_packet(&mut writer, Command::Okay, host_id_3, mock_id_3, &[])
            .await
            .unwrap();

        let (clse3, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(clse3.command, Command::Clse);

        server.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_banner_features() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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
        assert!(banner.features().contains(&Feature::Shell2));
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
        assert!(!banner_no_v2.features().contains(&Feature::Shell2));
        server_no_v2.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_echo_and_exit() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_stderr_and_env_term() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_cat_and_pty_winsz() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_shell_v2_unsupported_command() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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
    })
    .await
    .unwrap();
}

// ------------------- New C14 Tests -------------------

#[tokio::test]
async fn test_mock_adbd_concurrent_cat_streams() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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

        // Stream 1
        let host_id_1 = 201u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_1,
            0,
            b"shell,v2,raw:cat\0",
        )
        .await
        .unwrap();

        let (okay1, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay1.command, Command::Okay);
        let mock_id_1 = okay1.arg0;

        // Stream 2
        let host_id_2 = 202u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_2,
            0,
            b"shell,v2,raw:cat\0",
        )
        .await
        .unwrap();

        let (okay2, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay2.command, Command::Okay);
        let mock_id_2 = okay2.arg0;

        // Send stdin to Stream 1
        let stdin1 = ShellPacket::new(ShellPacketId::Stdin, b"hello stream1".to_vec());
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_1,
            mock_id_1,
            &stdin1.encode_to_vec(),
        )
        .await
        .unwrap();

        // Send stdin to Stream 2
        let stdin2 = ShellPacket::new(ShellPacketId::Stdin, b"hello stream2".to_vec());
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_2,
            mock_id_2,
            &stdin2.encode_to_vec(),
        )
        .await
        .unwrap();

        let mut decoder1 = ShellDecoder::new();
        let mut decoder2 = ShellDecoder::new();
        let mut got_stream1_stdout = false;
        let mut got_stream2_stdout = false;

        while !got_stream1_stdout || !got_stream2_stdout {
            let (hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
            if hdr.command == Command::Okay {
                // Acknowledge of host's WRTE
                continue;
            }
            assert_eq!(hdr.command, Command::Wrte);
            if hdr.arg1 == host_id_1 {
                decoder1.feed(&payload);
                let pkt = decoder1.pop_packet().unwrap().unwrap();
                assert_eq!(pkt.id, ShellPacketId::Stdout);
                assert_eq!(String::from_utf8_lossy(&pkt.data), "hello stream1");
                got_stream1_stdout = true;
                write_packet(&mut writer, Command::Okay, host_id_1, mock_id_1, &[])
                    .await
                    .unwrap();
            } else if hdr.arg1 == host_id_2 {
                decoder2.feed(&payload);
                let pkt = decoder2.pop_packet().unwrap().unwrap();
                assert_eq!(pkt.id, ShellPacketId::Stdout);
                assert_eq!(String::from_utf8_lossy(&pkt.data), "hello stream2");
                got_stream2_stdout = true;
                write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
                    .await
                    .unwrap();
            }
        }

        // Close Stream 1
        let close1 = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_1,
            mock_id_1,
            &close1.encode_to_vec(),
        )
        .await
        .unwrap();

        // Read until Stream 1 closes
        loop {
            let (hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
            if hdr.command == Command::Okay {
                continue;
            }
            if hdr.command == Command::Wrte && hdr.arg1 == host_id_1 {
                decoder1.feed(&payload);
                let pkt = decoder1.pop_packet().unwrap().unwrap();
                assert_eq!(pkt.id, ShellPacketId::Exit);
                assert_eq!(pkt.parse_exit_code().unwrap(), 0);
                write_packet(&mut writer, Command::Okay, host_id_1, mock_id_1, &[])
                    .await
                    .unwrap();
            } else if hdr.command == Command::Clse && hdr.arg1 == host_id_1 {
                break;
            }
        }

        // Close Stream 2
        let close2 = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_2,
            mock_id_2,
            &close2.encode_to_vec(),
        )
        .await
        .unwrap();

        loop {
            let (hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
            if hdr.command == Command::Okay {
                continue;
            }
            if hdr.command == Command::Wrte && hdr.arg1 == host_id_2 {
                decoder2.feed(&payload);
                let pkt = decoder2.pop_packet().unwrap().unwrap();
                assert_eq!(pkt.id, ShellPacketId::Exit);
                assert_eq!(pkt.parse_exit_code().unwrap(), 0);
                write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
                    .await
                    .unwrap();
            } else if hdr.command == Command::Clse && hdr.arg1 == host_id_2 {
                break;
            }
        }

        server.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_open_during_pending_okay() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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

        // Stream 1 OPEN
        let host_id_1 = 301u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_1,
            0,
            b"shell:echo stream1\0",
        )
        .await
        .unwrap();

        let (okay1, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay1.command, Command::Okay);
        let mock_id_1 = okay1.arg0;

        // Mock sends WRTE for Stream 1
        let (wrte1, payload1) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte1.command, Command::Wrte);
        assert_eq!(wrte1.arg1, host_id_1);
        assert_eq!(&payload1, b"stream1\n");

        // Do NOT send OKAY for Stream 1 yet. Instead send OPEN for Stream 2!
        let host_id_2 = 302u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_2,
            0,
            b"shell:echo stream2\0",
        )
        .await
        .unwrap();

        // Stream 2 should still get OKAY and WRTE
        let (okay2, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay2.command, Command::Okay);
        let mock_id_2 = okay2.arg0;

        let (wrte2, payload2) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte2.command, Command::Wrte);
        assert_eq!(wrte2.arg1, host_id_2);
        assert_eq!(&payload2, b"stream2\n");

        // Now send OKAY for Stream 1 and Stream 2
        write_packet(&mut writer, Command::Okay, host_id_1, mock_id_1, &[])
            .await
            .unwrap();
        write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
            .await
            .unwrap();

        // Both streams should receive CLSE
        let mut clse1_received = false;
        let mut clse2_received = false;

        while !clse1_received || !clse2_received {
            let (hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
            assert_eq!(hdr.command, Command::Clse);
            if hdr.arg1 == host_id_1 {
                clse1_received = true;
            } else if hdr.arg1 == host_id_2 {
                clse2_received = true;
            }
        }

        server.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_host_clse_middle() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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

        // Stream 1 & 2
        let host_id_1 = 401u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_1,
            0,
            b"shell,v2,raw:cat\0",
        )
        .await
        .unwrap();
        let (okay1, _) = read_packet(&mut reader).await.unwrap().unwrap();
        let mock_id_1 = okay1.arg0;

        let host_id_2 = 402u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id_2,
            0,
            b"shell,v2,raw:cat\0",
        )
        .await
        .unwrap();
        let (okay2, _) = read_packet(&mut reader).await.unwrap().unwrap();
        let mock_id_2 = okay2.arg0;

        // Send CLSE for Stream 1 in the middle
        write_packet(&mut writer, Command::Clse, host_id_1, mock_id_1, &[])
            .await
            .unwrap();

        // Stream 2 should continue working without issue
        let stdin2 = ShellPacket::new(ShellPacketId::Stdin, b"still active".to_vec());
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_2,
            mock_id_2,
            &stdin2.encode_to_vec(),
        )
        .await
        .unwrap();

        let mut decoder2 = ShellDecoder::new();
        let mut got_stdout2 = false;

        while !got_stdout2 {
            let (hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
            if hdr.command == Command::Okay {
                continue;
            }
            assert_eq!(hdr.command, Command::Wrte);
            assert_eq!(hdr.arg1, host_id_2);
            decoder2.feed(&payload);
            let pkt = decoder2.pop_packet().unwrap().unwrap();
            assert_eq!(pkt.id, ShellPacketId::Stdout);
            assert_eq!(String::from_utf8_lossy(&pkt.data), "still active");
            got_stdout2 = true;
            write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
                .await
                .unwrap();
        }

        // Close Stream 2
        let close2 = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
        write_packet(
            &mut writer,
            Command::Wrte,
            host_id_2,
            mock_id_2,
            &close2.encode_to_vec(),
        )
        .await
        .unwrap();

        loop {
            let (hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
            if hdr.command == Command::Okay {
                continue;
            }
            if hdr.command == Command::Wrte && hdr.arg1 == host_id_2 {
                decoder2.feed(&payload);
                let pkt = decoder2.pop_packet().unwrap().unwrap();
                assert_eq!(pkt.id, ShellPacketId::Exit);
                assert_eq!(pkt.parse_exit_code().unwrap(), 0);
                write_packet(&mut writer, Command::Okay, host_id_2, mock_id_2, &[])
                    .await
                    .unwrap();
            } else if hdr.command == Command::Clse && hdr.arg1 == host_id_2 {
                break;
            }
        }

        server.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_version_min_and_checksums() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr();

        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut reader, mut writer) = client.split();

        // Send CNXN with A_VERSION_MIN (0x01000000)
        write_packet(
            &mut writer,
            Command::Cnxn,
            A_VERSION_MIN,
            MAX_PAYLOAD as u32,
            b"host::",
        )
        .await
        .unwrap();

        let (cnxn_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(cnxn_hdr.command, Command::Cnxn);
        assert_eq!(cnxn_hdr.arg0, A_VERSION_MIN);

        // Open shell:echo hello
        let host_id = 501u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id,
            0,
            b"shell:echo hello\0",
        )
        .await
        .unwrap();

        // Verify all returned packets have correct checksums calculated for payload
        let (okay_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(okay_hdr.command, Command::Okay);
        assert_eq!(okay_hdr.data_crc32, 0); // empty payload checksum is 0

        let (wrte_hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte_hdr.command, Command::Wrte);
        let expected_crc: u32 = payload.iter().map(|&b| b as u32).sum();
        assert_eq!(wrte_hdr.data_crc32, expected_crc);

        // Ack
        write_packet(&mut writer, Command::Okay, host_id, okay_hdr.arg0, &[])
            .await
            .unwrap();

        let (clse_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(clse_hdr.command, Command::Clse);
        assert_eq!(clse_hdr.data_crc32, 0);

        server.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_small_max_payload_split() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
        let addr = server.local_addr();

        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut reader, mut writer) = client.split();

        // Send CNXN with max payload limit = 16 bytes
        let small_payload_limit = 16u32;
        write_packet(
            &mut writer,
            Command::Cnxn,
            A_VERSION,
            small_payload_limit,
            b"host::",
        )
        .await
        .unwrap();

        let (cnxn_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(cnxn_hdr.command, Command::Cnxn);
        assert_eq!(cnxn_hdr.arg1, small_payload_limit);

        // Open shell:echo 12345678901234567890 (output: "12345678901234567890\n", 21 bytes)
        let host_id = 601u32;
        write_packet(
            &mut writer,
            Command::Open,
            host_id,
            0,
            b"shell:echo 12345678901234567890\0",
        )
        .await
        .unwrap();

        let (okay_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
        let mock_id = okay_hdr.arg0;

        let mut combined_output = Vec::new();

        loop {
            let (hdr, payload) = read_packet(&mut reader).await.unwrap().unwrap();
            if hdr.command == Command::Wrte {
                assert!(
                    hdr.data_length <= small_payload_limit,
                    "WRTE data length {} exceeded limit {}",
                    hdr.data_length,
                    small_payload_limit
                );
                combined_output.extend_from_slice(&payload);
                write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
                    .await
                    .unwrap();
            } else if hdr.command == Command::Clse {
                break;
            }
        }

        assert_eq!(
            String::from_utf8_lossy(&combined_output),
            "12345678901234567890\n"
        );

        server.shutdown();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn test_mock_adbd_shutdown_cleanup() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
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

        let (cnxn_hdr, _) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(cnxn_hdr.command, Command::Cnxn);

        // Shutdown server
        server.shutdown();

        // Existing client read should result in EOF / connection reset
        let res = read_packet(&mut reader).await;
        assert!(
            res.is_err() || res.unwrap().is_none(),
            "Expected connection closed on shutdown"
        );

        // New connection attempt should fail
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let new_conn = TcpStream::connect(addr).await;
        if let Ok(mut stream) = new_conn {
            let res = read_packet(&mut stream).await;
            assert!(res.is_err() || res.unwrap().is_none());
        }
    })
    .await
    .unwrap();
}
