use adb_client::{AdbClient, DefaultSessionFactory, ServerAddress, TrackDevicesMode};
use compat::{Config, TestServer};
use std::env;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio_stream::StreamExt;

fn get_mock_adbd_bin() -> PathBuf {
    if let Ok(bin) = env::var("CARGO_BIN_EXE_mock-adbd") {
        return PathBuf::from(bin);
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_dir).join("../..");
    let bin_name = if cfg!(target_os = "windows") {
        "mock-adbd.exe"
    } else {
        "mock-adbd"
    };
    let bin_path = root.join("target").join("debug").join(bin_name);
    if !bin_path.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "mock-adbd"])
            .status()
            .expect("Failed to build mock-adbd binary");
        assert!(status.success(), "Failed to build mock-adbd");
    }
    bin_path
}

struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start() -> Self {
        let bin = get_mock_adbd_bin();
        let mut child = Command::new(&bin)
            .arg("--listen")
            .arg("127.0.0.1:0")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn mock-adbd binary at {:?}: {}", bin, e));

        let stdout = child.stdout.take().expect("Failed to open stdout");
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("Failed to read first line from mock-adbd stdout");
        let addr = first_line.trim().to_string();

        MockAdbdProcess { child, addr }
    }
}

impl Drop for MockAdbdProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn get_free_closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_connect_and_disconnect_parity_and_effect() {
    let mock = MockAdbdProcess::start();

    let server_ref = TestServer::start();
    let server_rust = TestServer::start();

    let session_factory = DefaultSessionFactory;

    let client_rust = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server_rust.port,
    });

    // 1. Initial devices state (should be empty)
    let devices_ref_before = server_ref.run_client(&["devices"]);
    let devices_rust_before = server_rust.run_client(&["devices"]);
    assert_eq!(
        norm(&devices_ref_before.stdout),
        norm(&devices_rust_before.stdout)
    );

    // 2. Connect mock adbd
    let ref_connect_out = server_ref.run_client(&["connect", &mock.addr]);
    assert!(ref_connect_out.success());

    let rust_connect_res = client_rust
        .connect(&mock.addr, &session_factory)
        .await
        .expect("Rust connect failed");

    assert_eq!(
        norm(ref_connect_out.stdout.trim()),
        norm(rust_connect_res.trim())
    );

    // 3. Compare devices effect after connect
    let devices_ref_after = server_ref.run_client(&["devices"]);
    let devices_rust_after = server_rust.run_client(&["devices"]);
    assert_eq!(
        norm(&devices_ref_after.stdout),
        norm(&devices_rust_after.stdout)
    );
    assert!(devices_rust_after.stdout.contains(&mock.addr));

    // 4. Disconnect mock adbd
    let ref_disconnect_out = server_ref.run_client(&["disconnect", &mock.addr]);
    assert!(ref_disconnect_out.success());

    let rust_disconnect_res = client_rust
        .disconnect(Some(&mock.addr), &session_factory)
        .await
        .expect("Rust disconnect failed");

    assert_eq!(
        norm(ref_disconnect_out.stdout.trim()),
        norm(rust_disconnect_res.trim())
    );

    // 5. Compare devices effect after disconnect
    let devices_ref_final = server_ref.run_client(&["devices"]);
    let devices_rust_final = server_rust.run_client(&["devices"]);
    assert_eq!(
        norm(&devices_ref_final.stdout),
        norm(&devices_rust_final.stdout)
    );
    assert!(!devices_rust_final.stdout.contains(&mock.addr));

    // 6. Test disconnect with None (disconnect all)
    let ref_disconnect_all = server_ref.run_client(&["disconnect"]);
    assert!(ref_disconnect_all.success());

    let rust_disconnect_all = client_rust
        .disconnect(None, &session_factory)
        .await
        .expect("Rust disconnect all failed");

    assert_eq!(
        norm(ref_disconnect_all.stdout.trim()),
        norm(rust_disconnect_all.trim())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_connect_closed_port() {
    let closed_port = get_free_closed_port();
    let closed_addr = format!("127.0.0.1:{closed_port}");

    let server_ref = TestServer::start();
    let server_rust = TestServer::start();

    let session_factory = DefaultSessionFactory;

    let client_rust = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server_rust.port,
    });

    let ref_out = server_ref.run_client(&["connect", &closed_addr]);
    let rust_out = client_rust
        .connect(&closed_addr, &session_factory)
        .await
        .expect("connect query to closed port should return server text");

    assert_eq!(norm(ref_out.stdout.trim()), norm(rust_out.trim()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_track_devices_parity() {
    let server = TestServer::start();
    let session_factory = DefaultSessionFactory;

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });

    // Spawn reference client for track-devices
    let config = Config::from_env();
    let mut ref_child = tokio::process::Command::new(&config.client_path)
        .arg("-P")
        .arg(server.port.to_string())
        .arg("track-devices")
        .stdout(Stdio::piped())
        .spawn()
        .expect("Failed to spawn reference track-devices client");

    let mut ref_stdout = ref_child.stdout.take().expect("Failed to open stdout");

    // Spawn Rust track-devices stream
    let rust_stream = client
        .track_devices(TrackDevicesMode::Normal, &session_factory)
        .await
        .expect("Failed to start Rust track-devices stream");
    tokio::pin!(rust_stream);

    // Helper to read next chunk with a timeout
    async fn read_ref_chunk(stdout: &mut tokio::process::ChildStdout) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(3), stdout.read(&mut buf))
            .await
            .expect("Timed out reading from ref client stdout pipe")
            .expect("Read error from ref client stdout");
        buf[..n].to_vec()
    }

    async fn pop_or_read_ref(
        buf: &mut Vec<u8>,
        stdout: &mut tokio::process::ChildStdout,
    ) -> Vec<u8> {
        loop {
            if buf.len() >= 4 {
                if let Ok(len_str) = std::str::from_utf8(&buf[..4]) {
                    if let Ok(payload_len) = usize::from_str_radix(len_str, 16) {
                        let total_len = 4 + payload_len;
                        if buf.len() >= total_len {
                            return buf.drain(..total_len).collect();
                        }
                    }
                }
            }
            let chunk = read_ref_chunk(stdout).await;
            if chunk.is_empty() {
                panic!("Unexpected EOF while reading ref track-devices message");
            }
            buf.extend_from_slice(&chunk);
        }
    }

    async fn pop_or_read_rust<S>(buf: &mut Vec<u8>, stream: &mut S) -> Vec<u8>
    where
        S: tokio_stream::Stream<Item = Result<bytes::Bytes, adb_client::ClientError>> + Unpin,
    {
        loop {
            if buf.len() >= 4 {
                if let Ok(len_str) = std::str::from_utf8(&buf[..4]) {
                    if let Ok(payload_len) = usize::from_str_radix(len_str, 16) {
                        let total_len = 4 + payload_len;
                        if buf.len() >= total_len {
                            return buf.drain(..total_len).collect();
                        }
                    }
                }
            }
            let chunk = tokio::time::timeout(Duration::from_secs(3), stream.next())
                .await
                .expect("Timed out reading Rust stream")
                .expect("Rust stream ended")
                .expect("Rust stream error");
            if chunk.is_empty() {
                panic!("Unexpected EOF while reading Rust track-devices message");
            }
            buf.extend_from_slice(&chunk);
        }
    }

    let mut ref_buf = Vec::new();
    let mut rust_buf = Vec::new();

    fn norm_stream_bytes(bytes: &[u8]) -> Vec<u8> {
        bytes.iter().copied().filter(|&b| b != b'\r').collect()
    }

    // 1. Initial update (when no devices attached, server sends "0000")
    let ref_msg1 = pop_or_read_ref(&mut ref_buf, &mut ref_stdout).await;
    let rust_msg1 = pop_or_read_rust(&mut rust_buf, &mut rust_stream).await;

    assert_eq!(norm_stream_bytes(&ref_msg1), norm_stream_bytes(&rust_msg1));

    // 2. Connect mock adbd
    let mock = MockAdbdProcess::start();
    let connect_res = client
        .connect(&mock.addr, &session_factory)
        .await
        .expect("connect mock failed");
    assert!(connect_res.contains("connected"));

    let ref_msg2 = pop_or_read_ref(&mut ref_buf, &mut ref_stdout).await;
    let rust_msg2 = pop_or_read_rust(&mut rust_buf, &mut rust_stream).await;

    assert_eq!(norm_stream_bytes(&ref_msg2), norm_stream_bytes(&rust_msg2));

    // 3. Disconnect mock adbd
    let disconnect_res = client
        .disconnect(Some(&mock.addr), &session_factory)
        .await
        .expect("disconnect mock failed");
    assert!(disconnect_res.contains("disconnected"));

    let ref_msg3 = pop_or_read_ref(&mut ref_buf, &mut ref_stdout).await;
    let rust_msg3 = pop_or_read_rust(&mut rust_buf, &mut rust_stream).await;

    assert_eq!(norm_stream_bytes(&ref_msg3), norm_stream_bytes(&rust_msg3));

    let _ = ref_child.kill().await;
    let _ = ref_child.wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_track_devices_drop_closes_connection() {
    let server = TestServer::start();
    let session_factory = DefaultSessionFactory;

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });

    let tracker1 = client
        .track_devices(TrackDevicesMode::Normal, &session_factory)
        .await
        .expect("Failed to start tracker1");

    let tracker2 = client
        .track_devices(TrackDevicesMode::Normal, &session_factory)
        .await
        .expect("Failed to start tracker2");
    tokio::pin!(tracker2);

    // Read initial update on tracker2
    let initial_chunk = tokio::time::timeout(Duration::from_secs(3), tracker2.next())
        .await
        .expect("Timed out reading tracker2 initial")
        .expect("tracker2 initial stream end")
        .expect("tracker2 initial error");
    assert_eq!(initial_chunk.as_ref(), b"0000");

    // Drop tracker1
    drop(tracker1);

    // Verify tracker2 continues working when a device connects
    let mock = MockAdbdProcess::start();
    let _ = client.connect(&mock.addr, &session_factory).await;

    let update_chunk = tokio::time::timeout(Duration::from_secs(3), tracker2.next())
        .await
        .expect("Timed out reading tracker2 update after tracker1 dropped")
        .expect("tracker2 stream end")
        .expect("tracker2 error");

    assert!(update_chunk.len() > 4);
    assert!(String::from_utf8_lossy(&update_chunk).contains(&mock.addr));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_usage_errors() {
    let server = TestServer::start();
    let session_factory = DefaultSessionFactory;

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });

    fn extract_error_msg(stderr: &str) -> String {
        let trimmed = stderr.trim();
        if let Some((_prefix, rest)) = trimmed.split_once(": ") {
            rest.to_string()
        } else {
            trimmed.to_string()
        }
    }

    // 1. connect invalid arguments
    let err_conn_empty = client
        .connect_with_args(&[], &session_factory)
        .await
        .expect_err("connect with no args should fail usage");
    assert_eq!(err_conn_empty.to_string(), "usage: adb connect HOST[:PORT]");

    let ref_conn_empty = server.run_client(&["connect"]);
    assert_eq!(ref_conn_empty.exit_code, Some(1));
    assert_eq!(
        extract_error_msg(&ref_conn_empty.stderr),
        err_conn_empty.to_string()
    );

    let err_conn_extra = client
        .connect_with_args(&["host1", "host2"], &session_factory)
        .await
        .expect_err("connect with extra args should fail usage");
    assert_eq!(err_conn_extra.to_string(), "usage: adb connect HOST[:PORT]");

    let ref_conn_extra = server.run_client(&["connect", "host1", "host2"]);
    assert_eq!(ref_conn_extra.exit_code, Some(1));
    assert_eq!(
        extract_error_msg(&ref_conn_extra.stderr),
        err_conn_extra.to_string()
    );

    // 2. disconnect invalid arguments
    let err_disc_extra = client
        .disconnect_with_args(&["host1", "host2"], &session_factory)
        .await
        .expect_err("disconnect with extra args should fail usage");
    assert_eq!(
        err_disc_extra.to_string(),
        "usage: adb disconnect [HOST[:PORT]]"
    );

    let ref_disc_extra = server.run_client(&["disconnect", "host1", "host2"]);
    assert_eq!(ref_disc_extra.exit_code, Some(1));
    assert_eq!(
        extract_error_msg(&ref_disc_extra.stderr),
        err_disc_extra.to_string()
    );

    // 3. track-devices invalid arguments
    let err_track_invalid = match client
        .track_devices_with_args(&["-x"], &session_factory)
        .await
    {
        Err(e) => e,
        Ok(_) => panic!("track-devices with invalid arg should fail usage"),
    };
    assert_eq!(
        err_track_invalid.to_string(),
        "usage: adb track-devices [-l][--proto-text][--proto-binary]"
    );

    let ref_track_invalid = server.run_client(&["track-devices", "-x"]);
    assert_eq!(ref_track_invalid.exit_code, Some(1));
    assert_eq!(
        extract_error_msg(&ref_track_invalid.stderr),
        err_track_invalid.to_string()
    );

    let err_track_multiple = match client
        .track_devices_with_args(&["invalid1", "invalid2"], &session_factory)
        .await
    {
        Err(e) => e,
        Ok(_) => panic!("track-devices with multiple args should fail usage"),
    };
    assert_eq!(
        err_track_multiple.to_string(),
        "usage: adb track-devices [-l][--proto-text][--proto-binary]"
    );

    let ref_track_multiple = server.run_client(&["track-devices", "invalid1", "invalid2"]);
    assert_eq!(ref_track_multiple.exit_code, Some(1));
    assert_eq!(
        extract_error_msg(&ref_track_multiple.stderr),
        err_track_multiple.to_string()
    );
}
