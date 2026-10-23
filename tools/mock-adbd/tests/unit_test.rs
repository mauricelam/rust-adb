use adb_proto::{
    Banner, Command, Feature, ShellDecoder, ShellPacket, ShellPacketId, WindowSize, A_VERSION,
    A_VERSION_MIN, MAX_PAYLOAD,
};
use mock_adbd::{read_packet, write_packet, MockAdbdOptions, MockAdbdServer};
use std::process::Command as ProcessCommand;
use std::time::Duration;
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
            ..Default::default()
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

// ------------------- Integration Harness Tests with Reference ADB -------------------

fn find_reference_adb() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("ADB_REFERENCE") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let root = std::path::PathBuf::from(manifest_dir).join("../..");
    let path = if cfg!(target_os = "windows") {
        root.join("binaries/windows/adb.exe")
    } else if cfg!(target_os = "macos") {
        root.join("binaries/mac/adb")
    } else {
        root.join("binaries/linux/adb")
    };
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn get_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

async fn run_harness_test_suite(options: MockAdbdOptions) {
    let adb_bin = match find_reference_adb() {
        Some(b) => b,
        None => {
            eprintln!("Skipping harness test: ADB_REFERENCE binary not found");
            return;
        }
    };

    let server = MockAdbdServer::bind_with_options("127.0.0.1:0", options)
        .await
        .unwrap();
    let mock_addr = server.local_addr();

    // Populate initial files
    server.fs().add_dir("/data", 0o040755, 0);
    server.fs().add_file(
        "/data/existing.txt",
        b"existing file contents".to_vec(),
        0o100644,
        1000,
    );

    let sizes = [0, 1, 65535, 65536, 307200];
    for &sz in &sizes {
        let path = format!("/data/file_{sz}.bin");
        let data = vec![(sz % 251) as u8; sz];
        server.fs().add_file(&path, data, 0o100644, 2000);
    }

    let server_port = get_free_port();
    let temp_home = tempfile::tempdir().unwrap();

    // Start reference ADB server
    let mut server_child = ProcessCommand::new(&adb_bin)
        .args(["-P", &server_port.to_string(), "server", "nodaemon"])
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .env("ANDROID_ADB_HOME", temp_home.path())
        .env("ANDROID_USER_HOME", temp_home.path())
        .env("ADB_MDNS_AUTO_CONNECT", "0")
        .env("ADB_MDNS_OPENSCREEN", "0")
        .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
        .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Failed to start reference ADB server");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Connect to mock-adbd
    let conn_output = ProcessCommand::new(&adb_bin)
        .args([
            "-P",
            &server_port.to_string(),
            "connect",
            &mock_addr.to_string(),
        ])
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .output()
        .expect("Failed to run adb connect");
    eprintln!(
        "[HARNESS] adb connect result: status={}, stdout={}, stderr={}",
        conn_output.status,
        String::from_utf8_lossy(&conn_output.stdout),
        String::from_utf8_lossy(&conn_output.stderr)
    );

    assert!(
        conn_output.status.success(),
        "adb connect failed: {}",
        String::from_utf8_lossy(&conn_output.stderr)
    );

    let serial = mock_addr.to_string();

    // Wait for device to be online in 'adb devices' output
    let start_wait = std::time::Instant::now();
    loop {
        let devices_output = ProcessCommand::new(&adb_bin)
            .args(["-P", &server_port.to_string(), "devices"])
            .env("HOME", temp_home.path())
            .env("USERPROFILE", temp_home.path())
            .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
            .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&devices_output.stdout);
        let device_line = format!("{serial}\tdevice");
        if stdout.contains(&device_line) {
            break;
        }

        if start_wait.elapsed() > Duration::from_secs(5) {
            panic!(
                "Device {serial} did not reach 'device' state within 5 seconds. 'adb devices' output:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&devices_output.stderr)
            );
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // 1. adb ls /data
    let ls_output = ProcessCommand::new(&adb_bin)
        .args(["-P", &server_port.to_string(), "-s", &serial, "ls", "/data"])
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
        .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
        .output()
        .unwrap();

    assert!(
        ls_output.status.success(),
        "adb ls failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&ls_output.stdout),
        String::from_utf8_lossy(&ls_output.stderr)
    );
    let ls_stdout = String::from_utf8_lossy(&ls_output.stdout);
    assert!(
        ls_stdout.contains("existing.txt"),
        "ls stdout missing existing.txt: {ls_stdout}"
    );

    // 2. adb pull for each size
    for &sz in &sizes {
        let remote_path = format!("/data/file_{sz}.bin");
        let local_file = temp_home.path().join(format!("out_{sz}.bin"));

        let pull_output = ProcessCommand::new(&adb_bin)
            .args([
                "-P",
                &server_port.to_string(),
                "-s",
                &serial,
                "pull",
                &remote_path,
                local_file.to_str().unwrap(),
            ])
            .env("HOME", temp_home.path())
            .env("USERPROFILE", temp_home.path())
            .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
            .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
            .output()
            .unwrap();

        assert!(
            pull_output.status.success(),
            "adb pull {remote_path} failed: {}",
            String::from_utf8_lossy(&pull_output.stderr)
        );

        let pulled_bytes = std::fs::read(&local_file).unwrap();
        let expected_bytes = vec![(sz % 251) as u8; sz];
        assert_eq!(
            pulled_bytes, expected_bytes,
            "Pulled bytes mismatch for size {sz}"
        );
    }

    // 3. adb push for each size
    for &sz in &sizes {
        let local_file = temp_home.path().join(format!("in_{sz}.bin"));
        let push_bytes = vec![((sz + 5) % 251) as u8; sz];
        std::fs::write(&local_file, &push_bytes).unwrap();

        let remote_path = format!("/data/pushed_{sz}.bin");

        let push_output = ProcessCommand::new(&adb_bin)
            .args([
                "-P",
                &server_port.to_string(),
                "-s",
                &serial,
                "push",
                local_file.to_str().unwrap(),
                &remote_path,
            ])
            .env("HOME", temp_home.path())
            .env("USERPROFILE", temp_home.path())
            .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
            .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
            .output()
            .unwrap();

        assert!(
            push_output.status.success(),
            "adb push {remote_path} failed: {}",
            String::from_utf8_lossy(&push_output.stderr)
        );

        let stored_bytes = server
            .fs()
            .get_file(&remote_path)
            .expect("Pushed file missing in mock fs");
        assert_eq!(
            stored_bytes, push_bytes,
            "Pushed bytes mismatch for size {sz}"
        );
    }

    // 4. adb pull missing file -> fails
    let pull_missing = ProcessCommand::new(&adb_bin)
        .args([
            "-P",
            &server_port.to_string(),
            "-s",
            &serial,
            "pull",
            "/data/nonexistent.file",
            temp_home.path().join("missing.out").to_str().unwrap(),
        ])
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
        .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
        .output()
        .unwrap();

    assert!(
        !pull_missing.status.success(),
        "Pulling missing file should have failed"
    );

    // 5. adb push into missing directory -> fails
    let local_file = temp_home.path().join("fail_in.bin");
    std::fs::write(&local_file, b"data").unwrap();

    let push_missing_dir = ProcessCommand::new(&adb_bin)
        .args([
            "-P",
            &server_port.to_string(),
            "-s",
            &serial,
            "push",
            local_file.to_str().unwrap(),
            "/nonexistent_dir/nested/file.bin",
        ])
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .env("ANDROID_ADB_SERVER_PORT", server_port.to_string())
        .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{server_port}"))
        .output()
        .unwrap();

    assert!(
        !push_missing_dir.status.success(),
        "Pushing into missing directory should have failed"
    );

    // Cleanup reference ADB server
    let _ = ProcessCommand::new(&adb_bin)
        .args(["-P", &server_port.to_string(), "kill-server"])
        .env("HOME", temp_home.path())
        .env("USERPROFILE", temp_home.path())
        .output();

    let _ = server_child.kill();
    let _ = server_child.wait();

    server.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mock_adbd_sync_harness_v2_features_enabled() {
    run_harness_test_suite(MockAdbdOptions::default()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mock_adbd_sync_harness_v1_fallback() {
    run_harness_test_suite(MockAdbdOptions {
        enable_stat_v2: false,
        enable_ls_v2: false,
        enable_sendrecv_v2: false,
        ..Default::default()
    })
    .await;
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

// ------------------- New C14 Tests -------------------

#[tokio::test]
async fn test_mock_adbd_concurrent_cat_streams() {
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
}

#[tokio::test]
async fn test_mock_adbd_open_during_pending_okay() {
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
}

#[tokio::test]
async fn test_mock_adbd_host_clse_middle() {
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
}

#[tokio::test]
async fn test_mock_adbd_version_min_and_checksums() {
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
}

#[tokio::test]
async fn test_mock_adbd_small_max_payload_split() {
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
}

#[tokio::test]
async fn test_mock_adbd_shutdown_cleanup() {
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
}

// ------------------- Sync Service Packet-Level Tests -------------------

#[tokio::test]
async fn test_mock_adbd_sync_lstat_v1_and_stat_v2() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    server
        .fs()
        .add_file("/data/test.bin", b"hello".to_vec(), 0o100644, 12345);

    let mut client = TcpStream::connect(server.local_addr()).await.unwrap();
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

    // 1. STAT v1
    let host_id = 700u32;
    write_packet(&mut writer, Command::Open, host_id, 0, b"sync:\0")
        .await
        .unwrap();
    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    let mut req = Vec::new();
    req.extend_from_slice(&adb_proto::ID_LSTAT_V1.to_le_bytes());
    req.extend_from_slice(&(14u32.to_le_bytes()));
    req.extend_from_slice(b"/data/test.bin");
    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &req)
        .await
        .unwrap();

    let (ack, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack.command, Command::Okay);

    let (wrte, payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte.command, Command::Wrte);
    assert_eq!(payload.len(), 16);
    let stat_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    let mode = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    let size = u32::from_le_bytes(payload[8..12].try_into().unwrap());
    let mtime = u32::from_le_bytes(payload[12..16].try_into().unwrap());
    assert_eq!(stat_id, adb_proto::ID_LSTAT_V1);
    assert_eq!(mode & 0o777, 0o644);
    assert_eq!(size, 5);
    assert_eq!(mtime, 12345);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 2. STAT v2 for missing file
    let mut req_v2 = Vec::new();
    req_v2.extend_from_slice(&adb_proto::ID_STAT_V2.to_le_bytes());
    req_v2.extend_from_slice(&(17u32.to_le_bytes()));
    req_v2.extend_from_slice(b"/data/missing.bin");
    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &req_v2)
        .await
        .unwrap();

    let (ack2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack2.command, Command::Okay);

    let (wrte2, payload2) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte2.command, Command::Wrte);
    assert_eq!(payload2.len(), 72);
    let stat2_id = u32::from_le_bytes(payload2[0..4].try_into().unwrap());
    let error = u32::from_le_bytes(payload2[4..8].try_into().unwrap());
    assert_eq!(stat2_id, adb_proto::ID_STAT_V2);
    assert_eq!(error, 2); // ENOENT wire value = 2

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_sync_lstat_v2_recv_v2_send_v2() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    server.fs().add_dir("/data", 0o040755, 0);

    let mut client = TcpStream::connect(server.local_addr()).await.unwrap();
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

    let host_id = 704u32;
    write_packet(&mut writer, Command::Open, host_id, 0, b"sync:\0")
        .await
        .unwrap();
    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    // 1. LSTAT_V2 /data
    let mut req_lstat = Vec::new();
    let path_lstat = b"/data";
    req_lstat.extend_from_slice(&adb_proto::ID_LSTAT_V2.to_le_bytes());
    req_lstat.extend_from_slice(&(path_lstat.len() as u32).to_le_bytes());
    req_lstat.extend_from_slice(path_lstat);

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &req_lstat)
        .await
        .unwrap();
    let (ack, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack.command, Command::Okay);

    let (wrte, payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte.command, Command::Wrte);
    assert_eq!(payload.len(), 72);
    let resp_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    let resp_err = u32::from_le_bytes(payload[4..8].try_into().unwrap());
    assert_eq!(resp_id, adb_proto::ID_LSTAT_V2);
    assert_eq!(resp_err, 0);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 2. SEND_V2 /data/v2_file.bin
    let mut send_v2_req = Vec::new();
    let path_send = b"/data/v2_file.bin";
    send_v2_req.extend_from_slice(&adb_proto::ID_SEND_V2.to_le_bytes());
    send_v2_req.extend_from_slice(&(path_send.len() as u32).to_le_bytes());
    send_v2_req.extend_from_slice(path_send);

    // Setup block: ID_SEND_V2 (4 bytes), mode 0o100644 (4 bytes), flags 0 (4 bytes)
    send_v2_req.extend_from_slice(&adb_proto::ID_SEND_V2.to_le_bytes());
    send_v2_req.extend_from_slice(&(0o100644u32.to_le_bytes()));
    send_v2_req.extend_from_slice(&(0u32.to_le_bytes()));

    // DATA
    let data_bytes = b"v2 file payload";
    send_v2_req.extend_from_slice(&adb_proto::ID_DATA.to_le_bytes());
    send_v2_req.extend_from_slice(&(data_bytes.len() as u32).to_le_bytes());
    send_v2_req.extend_from_slice(data_bytes);

    // DONE mtime = 9999
    send_v2_req.extend_from_slice(&adb_proto::ID_DONE.to_le_bytes());
    send_v2_req.extend_from_slice(&(9999u32.to_le_bytes()));

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &send_v2_req)
        .await
        .unwrap();
    let (ack_s2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack_s2.command, Command::Okay);

    let (wrte_s2, payload_s2) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_s2.command, Command::Wrte);
    let ok_id = u32::from_le_bytes(payload_s2[0..4].try_into().unwrap());
    assert_eq!(ok_id, adb_proto::ID_OKAY);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    assert_eq!(
        server.fs().get_file("/data/v2_file.bin"),
        Some(b"v2 file payload".to_vec())
    );

    // 3. RECV_V2 /data/v2_file.bin
    let mut recv_v2_req = Vec::new();
    recv_v2_req.extend_from_slice(&adb_proto::ID_RECV_V2.to_le_bytes());
    recv_v2_req.extend_from_slice(&(path_send.len() as u32).to_le_bytes());
    recv_v2_req.extend_from_slice(path_send);

    // Setup block: ID_RECV_V2 (4 bytes), flags 0 (4 bytes)
    recv_v2_req.extend_from_slice(&adb_proto::ID_RECV_V2.to_le_bytes());
    recv_v2_req.extend_from_slice(&(0u32.to_le_bytes()));

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &recv_v2_req)
        .await
        .unwrap();
    let (ack_r2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack_r2.command, Command::Okay);

    // Read DATA
    let (wrte_r2_data, payload_r2_data) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_r2_data.command, Command::Wrte);
    let r2_data_id = u32::from_le_bytes(payload_r2_data[0..4].try_into().unwrap());
    let r2_data_len = u32::from_le_bytes(payload_r2_data[4..8].try_into().unwrap()) as usize;
    assert_eq!(r2_data_id, adb_proto::ID_DATA);
    assert_eq!(&payload_r2_data[8..8 + r2_data_len], b"v2 file payload");

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // Read DONE
    let (wrte_r2_done, payload_r2_done) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_r2_done.command, Command::Wrte);
    let r2_done_id = u32::from_le_bytes(payload_r2_done[0..4].try_into().unwrap());
    assert_eq!(r2_done_id, adb_proto::ID_DONE);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_sync_truncated_request_handling() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();

    let mut client = TcpStream::connect(server.local_addr()).await.unwrap();
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

    let host_id = 705u32;
    write_packet(&mut writer, Command::Open, host_id, 0, b"sync:\0")
        .await
        .unwrap();
    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    // Send truncated header (less than 8 bytes) inside a WRTE
    let trunc_req = vec![0u8; 4]; // Only 4 bytes
    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &trunc_req)
        .await
        .unwrap();

    let (ack, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack.command, Command::Okay);

    // Now abruptly close client stream without panic on mock server side
    drop(client);
    tokio::time::sleep(Duration::from_millis(50)).await;

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_sync_list_v1_and_v2() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    server
        .fs()
        .add_file("/data/a.txt", b"a".to_vec(), 0o100644, 100);
    server
        .fs()
        .add_file("/data/b.txt", b"bb".to_vec(), 0o100644, 200);

    let mut client = TcpStream::connect(server.local_addr()).await.unwrap();
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

    let host_id = 701u32;
    write_packet(&mut writer, Command::Open, host_id, 0, b"sync:\0")
        .await
        .unwrap();
    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    // LIST_V1 /data
    let mut req = Vec::new();
    req.extend_from_slice(&adb_proto::ID_LIST_V1.to_le_bytes());
    req.extend_from_slice(&(5u32.to_le_bytes()));
    req.extend_from_slice(b"/data");
    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &req)
        .await
        .unwrap();

    let (ack, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack.command, Command::Okay);

    // Read responses until DONE
    let mut entries = Vec::new();
    loop {
        let (wrte, payload) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte.command, Command::Wrte);
        let id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        if id == adb_proto::ID_DONE {
            write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
                .await
                .unwrap();
            break;
        }
        assert_eq!(id, adb_proto::ID_DENT_V1);
        let namelen = u32::from_le_bytes(payload[16..20].try_into().unwrap()) as usize;
        let name = String::from_utf8(payload[20..20 + namelen].to_vec()).unwrap();
        entries.push(name);
        write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
            .await
            .unwrap();
    }

    assert_eq!(entries, vec!["a.txt", "b.txt"]);

    // LIST_V2 /data
    let mut req_v2 = Vec::new();
    req_v2.extend_from_slice(&adb_proto::ID_LIST_V2.to_le_bytes());
    req_v2.extend_from_slice(&(5u32.to_le_bytes()));
    req_v2.extend_from_slice(b"/data");
    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &req_v2)
        .await
        .unwrap();

    let (ack_v2, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack_v2.command, Command::Okay);

    let mut entries_v2 = Vec::new();
    loop {
        let (wrte, payload) = read_packet(&mut reader).await.unwrap().unwrap();
        assert_eq!(wrte.command, Command::Wrte);
        let id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        if id == adb_proto::ID_DONE {
            write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
                .await
                .unwrap();
            break;
        }
        assert_eq!(id, adb_proto::ID_DENT_V2);
        let namelen = u32::from_le_bytes(payload[72..76].try_into().unwrap()) as usize;
        let name = String::from_utf8(payload[76..76 + namelen].to_vec()).unwrap();
        entries_v2.push(name);
        write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
            .await
            .unwrap();
    }

    assert_eq!(entries_v2, vec!["a.txt", "b.txt"]);

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_sync_recv_and_send() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    server.fs().add_dir("/data", 0o040755, 0);

    let mut client = TcpStream::connect(server.local_addr()).await.unwrap();
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

    let host_id = 702u32;
    write_packet(&mut writer, Command::Open, host_id, 0, b"sync:\0")
        .await
        .unwrap();
    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    // 1. SEND_V1 /data/file1.txt,0644
    let mut send_req = Vec::new();
    let spec = b"/data/file1.txt,0644";
    send_req.extend_from_slice(&adb_proto::ID_SEND_V1.to_le_bytes());
    send_req.extend_from_slice(&(spec.len() as u32).to_le_bytes());
    send_req.extend_from_slice(spec);

    // DATA chunk "hello world"
    let file_data = b"hello world";
    send_req.extend_from_slice(&adb_proto::ID_DATA.to_le_bytes());
    send_req.extend_from_slice(&(file_data.len() as u32).to_le_bytes());
    send_req.extend_from_slice(file_data);

    // DONE chunk mtime = 54321
    send_req.extend_from_slice(&adb_proto::ID_DONE.to_le_bytes());
    send_req.extend_from_slice(&(54321u32.to_le_bytes()));

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &send_req)
        .await
        .unwrap();

    let (ack, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack.command, Command::Okay);

    let (wrte, payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte.command, Command::Wrte);
    let reply_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    assert_eq!(reply_id, adb_proto::ID_OKAY);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    assert_eq!(
        server.fs().get_file("/data/file1.txt"),
        Some(b"hello world".to_vec())
    );

    // 2. RECV_V1 /data/file1.txt
    let mut recv_req = Vec::new();
    let path = b"/data/file1.txt";
    recv_req.extend_from_slice(&adb_proto::ID_RECV_V1.to_le_bytes());
    recv_req.extend_from_slice(&(path.len() as u32).to_le_bytes());
    recv_req.extend_from_slice(path);

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &recv_req)
        .await
        .unwrap();

    let (ack_r, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack_r.command, Command::Okay);

    // Read DATA chunk
    let (wrte_data, data_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_data.command, Command::Wrte);
    let data_id = u32::from_le_bytes(data_payload[0..4].try_into().unwrap());
    let data_size = u32::from_le_bytes(data_payload[4..8].try_into().unwrap()) as usize;
    assert_eq!(data_id, adb_proto::ID_DATA);
    assert_eq!(&data_payload[8..8 + data_size], b"hello world");

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // Read DONE chunk
    let (wrte_done, done_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_done.command, Command::Wrte);
    let done_id = u32::from_le_bytes(done_payload[0..4].try_into().unwrap());
    assert_eq!(done_id, adb_proto::ID_DONE);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 3. SEND into missing directory fails with FAIL
    let mut bad_send = Vec::new();
    let bad_spec = b"/missing_dir/file.txt,0644";
    bad_send.extend_from_slice(&adb_proto::ID_SEND_V1.to_le_bytes());
    bad_send.extend_from_slice(&(bad_spec.len() as u32).to_le_bytes());
    bad_send.extend_from_slice(bad_spec);
    bad_send.extend_from_slice(&adb_proto::ID_DATA.to_le_bytes());
    bad_send.extend_from_slice(&(4u32.to_le_bytes()));
    bad_send.extend_from_slice(b"test");
    bad_send.extend_from_slice(&adb_proto::ID_DONE.to_le_bytes());
    bad_send.extend_from_slice(&(100u32.to_le_bytes()));

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &bad_send)
        .await
        .unwrap();

    let (ack_b, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack_b.command, Command::Okay);

    let (wrte_fail, fail_payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte_fail.command, Command::Wrte);
    let fail_id = u32::from_le_bytes(fail_payload[0..4].try_into().unwrap());
    assert_eq!(fail_id, adb_proto::ID_FAIL);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // 4. QUIT
    let mut quit_req = Vec::new();
    quit_req.extend_from_slice(&adb_proto::ID_QUIT.to_le_bytes());
    quit_req.extend_from_slice(&0u32.to_le_bytes());

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &quit_req)
        .await
        .unwrap();

    let (ack_q, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack_q.command, Command::Okay);

    let (clse, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse.command, Command::Clse);

    server.shutdown();
}

#[tokio::test]
async fn test_mock_adbd_sync_oversized_path_and_truncation() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();

    let mut client = TcpStream::connect(server.local_addr()).await.unwrap();
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

    let host_id = 703u32;
    write_packet(&mut writer, Command::Open, host_id, 0, b"sync:\0")
        .await
        .unwrap();
    let (okay, _) = read_packet(&mut reader).await.unwrap().unwrap();
    let mock_id = okay.arg0;

    // Oversized path (1025 bytes)
    let mut req = Vec::new();
    req.extend_from_slice(&adb_proto::ID_LSTAT_V1.to_le_bytes());
    req.extend_from_slice(&(1025u32.to_le_bytes()));

    write_packet(&mut writer, Command::Wrte, host_id, mock_id, &req)
        .await
        .unwrap();

    let (ack, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(ack.command, Command::Okay);

    // Mock sends FAIL write
    let (wrte, payload) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(wrte.command, Command::Wrte);
    let fail_id = u32::from_le_bytes(payload[0..4].try_into().unwrap());
    assert_eq!(fail_id, adb_proto::ID_FAIL);

    write_packet(&mut writer, Command::Okay, host_id, mock_id, &[])
        .await
        .unwrap();

    // Expect CLSE
    let (clse, _) = read_packet(&mut reader).await.unwrap().unwrap();
    assert_eq!(clse.command, Command::Clse);

    server.shutdown();
}
