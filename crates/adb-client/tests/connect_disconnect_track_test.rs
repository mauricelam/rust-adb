use adb_client::{AdbClient, ClientError, ServerAddress, TrackDevicesMode};
use compat::TestServer;
use std::env;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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

fn run_client_isolated(server: &TestServer, args: &[&str]) -> compat::TestOutput {
    temp_env::with_var_unset("ANDROID_SERIAL", || server.run_client(args))
}

#[tokio::test]
async fn test_usage_errors() {
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: 5037,
    };
    let client = AdbClient::new(addr);

    // connect usage errors
    let err = adb_cli::parse_and_exec_connect(&client, &[])
        .await
        .result
        .unwrap_err();
    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "usage: adb connect HOST[:PORT]")
    );

    let err = adb_cli::parse_and_exec_connect(&client, &["a", "b"])
        .await
        .result
        .unwrap_err();
    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "usage: adb connect HOST[:PORT]")
    );

    // disconnect usage errors
    let err = adb_cli::parse_and_exec_disconnect(&client, &["a", "b"])
        .await
        .result
        .unwrap_err();
    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "usage: adb disconnect [HOST[:PORT]]")
    );

    // track-devices usage errors
    let err = adb_cli::parse_and_exec_track_devices(&client, &["-l", "--proto-text"])
        .await
        .result
        .err()
        .unwrap();
    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "usage: adb track-devices [-l][--proto-text][--proto-binary]")
    );

    let err = adb_cli::parse_and_exec_track_devices(&client, &["--unknown"])
        .await
        .result
        .err()
        .unwrap();
    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "usage: adb track-devices [-l][--proto-text][--proto-binary]")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_differential_connect_disconnect_and_closed_port() {
    let mock = MockAdbdProcess::start();

    // 1. Rust client against server1
    let server1 = TestServer::start();
    let addr1 = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server1.port,
    };
    let client1 = AdbClient::new(addr1);

    // 2. Reference client against server2
    let server2 = TestServer::start();

    // Verify initial devices on both servers
    let dev1_before = client1.devices(false).await.result.unwrap();
    let dev2_before = run_client_isolated(&server2, &["devices"]);
    assert_eq!(norm(&dev1_before), norm(&dev2_before.stdout));

    // Connect mock address
    let rust_connect_res = client1.connect(&mock.addr).await.result;
    let ref_connect_res = run_client_isolated(&server2, &["connect", &mock.addr]);

    assert!(rust_connect_res.is_ok());
    assert!(ref_connect_res.success());
    assert_eq!(
        norm(&rust_connect_res.unwrap()),
        norm(ref_connect_res.stdout.trim())
    );

    // Verify devices after connect
    let dev1_after = client1.devices(false).await.result.unwrap();
    let dev2_after = run_client_isolated(&server2, &["devices"]);
    assert_eq!(norm(&dev1_after), norm(&dev2_after.stdout));
    assert!(dev1_after.contains(&mock.addr));

    // Disconnect mock address
    let rust_disc_res = client1.disconnect(Some(&mock.addr)).await.result;
    let ref_disc_res = run_client_isolated(&server2, &["disconnect", &mock.addr]);

    assert!(rust_disc_res.is_ok());
    assert!(ref_disc_res.success());
    assert_eq!(
        norm(&rust_disc_res.unwrap()),
        norm(ref_disc_res.stdout.trim())
    );

    // Verify devices after disconnect
    let dev1_final = client1.devices(false).await.result.unwrap();
    let dev2_final = run_client_isolated(&server2, &["devices"]);
    assert_eq!(norm(&dev1_final), norm(&dev2_final.stdout));
    assert!(!dev1_final.contains(&mock.addr));

    // Test disconnect all (disconnect with None)
    let rust_disc_all = client1.disconnect(None).await.result;
    let ref_disc_all = run_client_isolated(&server2, &["disconnect"]);
    assert!(rust_disc_all.is_ok());
    assert!(ref_disc_all.success());
    assert_eq!(
        norm(&rust_disc_all.unwrap()),
        norm(ref_disc_all.stdout.trim())
    );

    // Test connection to closed port
    let closed_port = "127.0.0.1:1";
    let rust_closed_res = client1.connect(closed_port).await.result;
    let ref_closed_res = run_client_isolated(&server2, &["connect", closed_port]);

    assert!(rust_closed_res.is_ok());
    assert!(ref_closed_res.success());
    assert_eq!(
        norm(&rust_closed_res.unwrap()),
        norm(ref_closed_res.stdout.trim())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_differential_track_devices() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };
    let client = AdbClient::new(addr);

    let stream = client
        .track_devices(TrackDevicesMode::Normal)
        .await
        .result
        .unwrap();
    tokio::pin!(stream);

    // Initial update on connection (empty list "0000")
    let initial_chunk = tokio::time::timeout(std::time::Duration::from_secs(10), stream.next())
        .await
        .expect("Timeout waiting for initial track-devices update")
        .expect("Stream closed unexpectedly")
        .expect("Stream error");

    assert_eq!(initial_chunk, &b"0000"[..]);

    // Connect mock adbd
    let mock = MockAdbdProcess::start();
    let conn_res = run_client_isolated(&server, &["connect", &mock.addr]);
    assert!(conn_res.success());

    // Second update should report mock device
    let connect_chunk = tokio::time::timeout(std::time::Duration::from_secs(10), stream.next())
        .await
        .expect("Timeout waiting for connect track-devices update")
        .expect("Stream closed unexpectedly")
        .expect("Stream error");

    let chunk_str = String::from_utf8_lossy(&connect_chunk);
    assert!(chunk_str.contains(&mock.addr));

    // Disconnect mock adbd
    let disc_res = run_client_isolated(&server, &["disconnect", &mock.addr]);
    assert!(disc_res.success());

    // Third update should report disconnect/offline or empty list
    let disconnect_chunk = tokio::time::timeout(std::time::Duration::from_secs(10), stream.next())
        .await
        .expect("Timeout waiting for disconnect track-devices update")
        .expect("Stream closed unexpectedly")
        .expect("Stream error");

    let disc_str = String::from_utf8_lossy(&disconnect_chunk);
    assert!(disc_str.contains("offline") || disc_str.contains("0000"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_track_devices_drop_closes_connection() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };
    let client = AdbClient::new(addr);

    // Stream 1 in inner scope so it is dropped after reading initial update
    {
        let stream1 = client
            .track_devices(TrackDevicesMode::Normal)
            .await
            .result
            .unwrap();
        tokio::pin!(stream1);
        let _init1 = tokio::time::timeout(std::time::Duration::from_secs(10), stream1.next())
            .await
            .expect("Timeout on init stream 1")
            .expect("Stream 1 closed")
            .expect("Stream 1 error");
    }

    // Stream 2 stays active
    let stream2 = client
        .track_devices(TrackDevicesMode::Normal)
        .await
        .result
        .unwrap();
    tokio::pin!(stream2);

    let _init2 = tokio::time::timeout(std::time::Duration::from_secs(10), stream2.next())
        .await
        .expect("Timeout on init stream 2")
        .expect("Stream 2 closed")
        .expect("Stream 2 error");

    // Connect mock adbd
    let mock = MockAdbdProcess::start();
    let conn_res = run_client_isolated(&server, &["connect", &mock.addr]);
    assert!(conn_res.success());

    // Stream 2 gets update
    let connect_chunk = tokio::time::timeout(std::time::Duration::from_secs(10), stream2.next())
        .await
        .expect("Timeout on connect stream 2")
        .expect("Stream 2 closed")
        .expect("Stream 2 error");

    let chunk_str = String::from_utf8_lossy(&connect_chunk);
    assert!(chunk_str.contains(&mock.addr));
}
