use adb_client::{format_host_command, ServerAddress, Session, SessionError, TransportSelector};
use compat::TestServer;
use std::env;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::timeout;

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

#[test]
fn test_format_host_command_precedence() {
    let cmd = "features";
    assert_eq!(
        format_host_command(cmd, &TransportSelector::Id(42)),
        "host-transport-id:42:features"
    );
    assert_eq!(
        format_host_command(cmd, &TransportSelector::Serial("device123".into())),
        "host-serial:device123:features"
    );
    assert_eq!(
        format_host_command(cmd, &TransportSelector::Usb),
        "host-usb:features"
    );
    assert_eq!(
        format_host_command(cmd, &TransportSelector::Local),
        "host-local:features"
    );
    assert_eq!(
        format_host_command(cmd, &TransportSelector::Any),
        "host:features"
    );

    // Method syntax on TransportSelector
    assert_eq!(
        TransportSelector::Id(100).format_host_command("version"),
        "host-transport-id:100:version"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_reference_server_host_version() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let res = timeout(
        Duration::from_secs(5),
        Session::connect_and_switch(&addr, &TransportSelector::Any, "host:version", false),
    )
    .await
    .expect("test timed out")
    .expect("host:version connection failed");

    let mut session = res;
    let version_payload = session
        .read_string()
        .await
        .expect("failed to read version payload");

    assert_eq!(version_payload.len(), 4);
    assert!(
        version_payload.chars().all(|c| c.is_ascii_hexdigit()),
        "expected hex version string, got: {version_payload}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_reference_server_unknown_host_service() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let service = "host:nonexistent_service_12345";
    let err = timeout(
        Duration::from_secs(5),
        Session::connect_and_switch(&addr, &TransportSelector::Any, service, false),
    )
    .await
    .expect("test timed out")
    .expect_err("unknown host service should fail");

    assert_eq!(
        err.to_string(),
        "unknown host service 'nonexistent_service_12345'"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_reference_server_device_selecting_service_no_device() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let service = "host:transport-any";
    let err = timeout(
        Duration::from_secs(5),
        Session::connect_and_switch(&addr, &TransportSelector::Any, service, false),
    )
    .await
    .expect("test timed out")
    .expect_err("host:transport-any should fail when no devices connected");

    assert_eq!(err.to_string(), "no devices/emulators found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_connect_closed_port_error_text() {
    // Find a port where no server is listening
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = listener.local_addr().unwrap().port();
    drop(listener); // Close port

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: closed_port,
    };

    let err = Session::connect(&addr)
        .await
        .expect_err("connecting to closed port should fail");

    let err_msg = err.to_string();
    assert!(
        err_msg.starts_with(&format!(
            "cannot connect to daemon at tcp:127.0.0.1:{closed_port}:"
        )),
        "Expected error starting with 'cannot connect to daemon at ...', got: '{err_msg}'"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mock_adbd_transport_switch_success() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    // Connect mock adbd to server
    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "adb connect failed: stdout: {}, stderr: {}",
        connect_out.stdout,
        connect_out.stderr
    );

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    // Connect session, select device by serial with host:tport:serial:<addr>
    let mut session = timeout(Duration::from_secs(5), Session::connect(&addr))
        .await
        .expect("test timed out")
        .expect("connect to server failed");

    let transport_id = session
        .select_transport(&TransportSelector::Serial(mock.addr.clone()))
        .await
        .expect("select_transport by serial failed");

    assert!(
        transport_id.is_some(),
        "expected transport ID returned from server"
    );

    // Send service over selected transport
    session
        .send_service("shell:echo hello")
        .await
        .expect("send_service failed");

    session
        .read_status()
        .await
        .expect("read_status after shell service failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_fake_server_invalid_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 64];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let _ = stream.write_all(b"BOGU").await;
        }
    });

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port,
    };

    let mut session = Session::connect(&addr).await.unwrap();
    session.send_service("host:version").await.unwrap();

    let err = session
        .read_status()
        .await
        .expect_err("should fail on invalid status");
    assert_eq!(err.to_string(), "protocol fault (status 42 4f 47 55?!)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_fake_server_closed_before_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 64];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            // Close connection without writing status
            drop(stream);
        }
    });

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port,
    };

    let mut session = Session::connect(&addr).await.unwrap();
    session.send_service("host:version").await.unwrap();

    let err = session
        .read_status()
        .await
        .expect_err("should fail on closed connection");
    assert!(
        err.to_string()
            .starts_with("protocol fault (couldn't read status):"),
        "expected 'protocol fault (couldn't read status): ...', got: '{err}'"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_fake_server_fail_empty_reason() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 64];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let _ = stream.write_all(b"FAIL0000").await;
        }
    });

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port,
    };

    let mut session = Session::connect(&addr).await.unwrap();
    session.send_service("host:version").await.unwrap();

    let err = session
        .read_status()
        .await
        .expect_err("should fail on FAIL status");
    if let SessionError::Fail(reason) = err {
        assert_eq!(reason, "");
    } else {
        panic!("expected SessionError::Fail, got {err:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_bad_service_name_length() {
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: 5037,
    };

    let err_empty = Session::connect_and_switch(&addr, &TransportSelector::Any, "", false)
        .await
        .expect_err("empty service name should fail");
    assert_eq!(err_empty.to_string(), "bad service name length (0)");

    let oversized_service = "a".repeat(adb_proto::MAX_PAYLOAD + 1);
    let err_oversized =
        Session::connect_and_switch(&addr, &TransportSelector::Any, &oversized_service, false)
            .await
            .expect_err("oversized service name should fail");
    assert_eq!(
        err_oversized.to_string(),
        format!("bad service name length ({})", adb_proto::MAX_PAYLOAD + 1)
    );
}
