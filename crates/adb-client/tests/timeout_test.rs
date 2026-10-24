use adb_client::{
    check_server_version, AdbClient, ClientErrorKind, DefaultSessionFactory,
    ExternalBinaryLauncher, ServerAddress, SessionConfig, SessionError, SessionFactory,
    TrackDevicesMode,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_smart_socket_server_never_replies_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = ServerAddress { host: None, port };

    // Spawn server that accepts connection but never writes anything back
    tokio::spawn(async move {
        while let Ok((_socket, _)) = listener.accept().await {
            // Keep connection open without writing
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });

    let config = SessionConfig {
        connect_timeout: Duration::from_millis(100),
        handshake_timeout: Duration::from_millis(100),
    };

    let session_factory = DefaultSessionFactory::with_config(config);
    let mut session = session_factory.connect(&addr).await.unwrap();

    session.send_service("host:version").await.unwrap();
    let err = session.read_status().await.unwrap_err();
    match err {
        SessionError::Timeout => {}
        other => panic!("Expected SessionError::Timeout, got {:?}", other),
    }

    // Check check_server_version with timeout
    let launcher = ExternalBinaryLauncher::new("/nonexistent");
    let mut events = Vec::new();
    let client_err = check_server_version(&addr, &session_factory, &launcher, &mut events)
        .await
        .unwrap_err();

    assert_eq!(client_err.kind(), ClientErrorKind::Timeout);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_launcher_ack_timeout_kills_child_and_fails() {
    use adb_client::{LaunchConfig, LaunchError, ServerLauncher};

    // Rebuild stream_helper example binary
    let status = std::process::Command::new("cargo")
        .args(["build", "--example", "stream_helper", "-p", "adb-client"])
        .status()
        .expect("Failed to run cargo build --example stream_helper");
    assert!(status.success());

    let helper_bin = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/examples/stream_helper");

    let launch_config = LaunchConfig {
        ack_timeout: Duration::from_millis(200),
    };

    let launcher = ExternalBinaryLauncher::with_config(&helper_bin, launch_config);
    let res = launcher.launch("tcp:5037 --no-ack", None).await;

    assert!(res.is_err());
    let err = res.unwrap_err();
    match err {
        LaunchError::Timeout(msg) => {
            assert!(msg.contains("could not read ok from ADB Server: timed out"));
        }
        other => panic!("Expected LaunchError::Timeout, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_connect_to_non_routable_address_times_out() {
    // TEST-NET-1 non-routable IP address
    let addr = ServerAddress {
        host: Some("192.0.2.1".to_string()),
        port: 5037,
    };

    let config = SessionConfig {
        connect_timeout: Duration::from_millis(200),
        handshake_timeout: Duration::from_millis(200),
    };

    let session_factory = DefaultSessionFactory::with_config(config);
    let start = std::time::Instant::now();
    let res = session_factory.connect(&addr).await;
    let elapsed = start.elapsed();

    assert!(res.is_err());
    assert!(
        elapsed < Duration::from_secs(3),
        "Connection attempt took too long: {:?}",
        elapsed
    );

    let err = res.unwrap_err();
    match err {
        SessionError::Timeout | SessionError::CannotConnect { .. } => {}
        other => panic!("Expected Timeout or CannotConnect, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_streaming_reads_are_unbounded_by_handshake_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = ServerAddress { host: None, port };

    // Server responds OKAY to track-devices, then waits 300ms before sending data
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0u8; 128];
            if let Ok(n) = socket.read(&mut buf).await {
                let service = String::from_utf8_lossy(&buf[..n]);
                if service.contains("version") {
                    let _ = socket.write_all(b"OKAY00040029").await;
                } else if service.contains("track-devices") {
                    let _ = socket.write_all(b"OKAY").await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let _ = socket.write_all(b"0004test").await;
                }
            }
        }
    });

    let session_config = SessionConfig {
        connect_timeout: Duration::from_secs(5),
        handshake_timeout: Duration::from_millis(100),
    };

    let client = AdbClient::builder(addr)
        .session_config(session_config)
        .build();

    let outcome = client.track_devices(TrackDevicesMode::Normal).await;
    assert!(outcome.result.is_ok());

    use tokio_stream::StreamExt;
    let mut stream = outcome.result.unwrap();

    let start = std::time::Instant::now();
    let item = stream.next().await;
    let elapsed = start.elapsed();

    assert!(item.is_some());
    let bytes = item.unwrap().unwrap();
    assert_eq!(&bytes[..], b"0004test");
    assert!(elapsed >= Duration::from_millis(250));
}
