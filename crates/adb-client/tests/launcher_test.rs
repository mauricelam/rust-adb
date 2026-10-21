// Tests for starting the ADB server when none is running (task 3.1d).
//
// Behavior ported from C++:
// - `launch_server` in `original/adb.cpp` lines 813-1120
// - `__adb_check_server_version` in `original/client/adb_client.cpp` lines 272-366

use adb_client::{
    check_server_version, ClientError, ClientEvent, DefaultSessionFactory, ExternalBinaryLauncher,
    ServerAddress, SessionFactory, ADB_SERVER_VERSION,
};
use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn get_free_port() -> u16 {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

struct ServerDropGuard {
    addr: ServerAddress,
}

impl Drop for ServerDropGuard {
    fn drop(&mut self) {
        let host = self.addr.host.as_deref().unwrap_or("127.0.0.1");
        if let Ok(mut stream) = std::net::TcpStream::connect((host, self.addr.port)) {
            let req = "0009host:kill";
            if stream.write_all(req.as_bytes()).is_ok() {
                let mut resp = [0u8; 4];
                let _ = stream.read_exact(&mut resp);
            }
        }
    }
}

// Scenario 1: starting through the launcher makes host:version answer afterwards.
#[tokio::test]
async fn test_launch_server_success() {
    let port = get_free_port();
    let addr = ServerAddress { host: None, port };
    let session_factory = DefaultSessionFactory;
    let ref_bin = compat::Config::from_env().server_bin_path;
    let launcher = ExternalBinaryLauncher::new(ref_bin);
    let mut events = Vec::new();

    let _guard = ServerDropGuard { addr: addr.clone() };

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;

    #[cfg(unix)]
    {
        assert!(res.is_ok(), "Expected start_server to succeed: {:?}", res);

        assert_eq!(
            events,
            vec![
                ClientEvent::DaemonNotRunning {
                    spec: addr.to_spec()
                },
                ClientEvent::DaemonStarted,
            ]
        );
        assert_eq!(
            events[0].to_string(),
            format!("* daemon not running; starting now at {}", addr.to_spec())
        );
        assert_eq!(events[1].to_string(), "* daemon started successfully");

        // Verify host:version answers afterwards
        let mut session = session_factory
            .connect(&addr)
            .await
            .expect("Should connect to launched server");
        session
            .send_service("host:version")
            .await
            .expect("Should send host:version");
        session.read_status().await.expect("Status OKAY expected");
        let ver_str = session
            .read_string()
            .await
            .expect("Version string expected");
        let ver = u32::from_str_radix(&ver_str, 16).expect("Hex version parse expected");
        assert_eq!(ver, ADB_SERVER_VERSION);
    }

    #[cfg(not(unix))]
    {
        assert!(
            matches!(res, Err(ClientError::CannotConnectDaemon)),
            "Expected Unsupported/CannotConnectDaemon on non-unix, got {:?}",
            res
        );
        assert_eq!(
            events,
            vec![
                ClientEvent::DaemonNotRunning {
                    spec: addr.to_spec()
                },
                ClientEvent::FailedToStartDaemon,
            ]
        );
    }
}

// Scenario 2: a second start does nothing and prints no starting now event.
#[tokio::test]
async fn test_second_start_does_nothing() {
    let port = get_free_port();
    let addr = ServerAddress { host: None, port };
    let session_factory = DefaultSessionFactory;
    let ref_bin = compat::Config::from_env().server_bin_path;
    let launcher = ExternalBinaryLauncher::new(ref_bin);

    let _guard = ServerDropGuard { addr: addr.clone() };

    #[cfg(unix)]
    {
        // First start
        let mut events1 = Vec::new();
        let res1 = check_server_version(&addr, &session_factory, &launcher, &mut events1).await;
        assert!(res1.is_ok());
        assert_eq!(events1.len(), 2);

        // Second start
        let mut events2 = Vec::new();
        let res2 = check_server_version(&addr, &session_factory, &launcher, &mut events2).await;
        assert!(res2.is_ok());
        assert!(
            events2.is_empty(),
            "Second check should produce no events, got {:?}",
            events2
        );
    }

    #[cfg(not(unix))]
    {
        let mut events1 = Vec::new();
        let res1 = check_server_version(&addr, &session_factory, &launcher, &mut events1).await;
        assert!(res1.is_err());
    }
}

// Scenario 3: a launcher that fails produces failed to start daemon and the cannot connect to daemon error.
#[tokio::test]
async fn test_launcher_failure() {
    let port = get_free_port();
    let addr = ServerAddress { host: None, port };
    let session_factory = DefaultSessionFactory;
    let launcher = ExternalBinaryLauncher::new(PathBuf::from("/nonexistent/binary/path/to/adb"));
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(
        matches!(err, ClientError::CannotConnectDaemon),
        "Expected CannotConnectDaemon error, got {:?}",
        err
    );
    assert_eq!(err.to_string(), "cannot connect to daemon");

    assert_eq!(
        events,
        vec![
            ClientEvent::DaemonNotRunning {
                spec: addr.to_spec()
            },
            ClientEvent::FailedToStartDaemon,
        ]
    );
    assert_eq!(
        events[0].to_string(),
        format!("* daemon not running; starting now at {}", addr.to_spec())
    );
    assert_eq!(events[1].to_string(), "* failed to start daemon");
}

// Scenario 4: starting after a version mismatch, using a fake server that reports a different version, kills it and starts the real one.
#[tokio::test]
async fn test_version_mismatch_restarts_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let addr = ServerAddress { host: None, port };

    // Spawn mock server returning version 40 (different from 41)
    tokio::spawn(async move {
        // Connection 1: host:version request -> respond with version 40 (0028 hex) and close
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut len_bytes = [0u8; 4];
            if socket.read_exact(&mut len_bytes).await.is_ok() {
                let mut service_bytes = vec![0u8; 12];
                if socket.read_exact(&mut service_bytes).await.is_ok() {
                    let _ = socket.write_all(b"OKAY00040028").await;
                    let _ = socket.shutdown().await;
                }
            }
        }

        // Connection 2: host:kill request
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut len_bytes = [0u8; 4];
            if socket.read_exact(&mut len_bytes).await.is_ok() {
                let mut service_bytes = vec![0u8; 9];
                if socket.read_exact(&mut service_bytes).await.is_ok()
                    && &service_bytes == b"host:kill"
                {
                    let _ = socket.write_all(b"OKAY").await;
                    let _ = socket.shutdown().await;
                }
            }
        }
    });

    let session_factory = DefaultSessionFactory;
    let ref_bin = compat::Config::from_env().server_bin_path;
    let launcher = ExternalBinaryLauncher::new(ref_bin);
    let mut events = Vec::new();

    let _guard = ServerDropGuard { addr: addr.clone() };

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;

    #[cfg(unix)]
    {
        assert!(res.is_ok(), "Expected restart to succeed: {:?}", res);

        assert_eq!(
            events,
            vec![
                ClientEvent::ServerVersionMismatch {
                    server: 40,
                    client: 41
                },
                ClientEvent::DaemonStarted,
            ]
        );

        // Verify real server is running on that port
        let mut session = session_factory
            .connect(&addr)
            .await
            .expect("Should connect to newly launched server");
        session
            .send_service("host:version")
            .await
            .expect("Should send host:version");
        session.read_status().await.expect("Status OKAY expected");
        let ver_str = session
            .read_string()
            .await
            .expect("Version string expected");
        let ver = u32::from_str_radix(&ver_str, 16).expect("Hex version parse expected");
        assert_eq!(ver, ADB_SERVER_VERSION);
    }

    #[cfg(not(unix))]
    {
        assert!(
            matches!(res, Err(ClientError::CannotConnectDaemon)),
            "Expected Unsupported/CannotConnectDaemon on non-unix, got {:?}",
            res
        );
        assert_eq!(
            events,
            vec![
                ClientEvent::ServerVersionMismatch {
                    server: 40,
                    client: 41
                },
                ClientEvent::FailedToStartDaemon,
            ]
        );
    }
}
