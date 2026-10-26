use adb_client::{
    check_server_version, ClientError, ClientEvent, DefaultSessionFactory, ExternalBinaryLauncher,
    ServerAddress,
};
#[cfg(unix)]
use adb_client::{kill_server, SessionFactory, ADB_SERVER_VERSION};

use std::env;
use std::net::TcpListener;
use std::path::PathBuf;
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn get_reference_adb_path() -> PathBuf {
    if let Ok(path) = env::var("ADB_SERVER_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(path) = env::var("ADB_REFERENCE") {
        return PathBuf::from(path);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to locate repo root");

    #[cfg(target_os = "windows")]
    let binary = repo_root.join("binaries/windows/adb.exe");
    #[cfg(target_os = "macos")]
    let binary = repo_root.join("binaries/mac/adb");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let binary = repo_root.join("binaries/linux/adb");

    binary
}

fn get_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

#[cfg(unix)]
struct KillOnDrop {
    addr: ServerAddress,
}

#[cfg(unix)]
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let addr = self.addr.clone();
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                let session_factory = DefaultSessionFactory::new();
                let mut events = Vec::new();
                let _ = kill_server(&addr, &session_factory, &mut events).await;
            });
        })
        .join();
    }
}

#[cfg(unix)]
struct MockServer {
    port: u16,
}

#[cfg(unix)]
impl MockServer {
    async fn start_with_version(version: u32) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            let mut listener = Some(listener);
            while let Some(l) = listener.as_ref() {
                let Ok((mut socket, _)) = l.accept().await else {
                    break;
                };

                let mut len_bytes = [0u8; 4];
                if socket.read_exact(&mut len_bytes).await.is_err() {
                    continue;
                }
                let Ok(len_str) = std::str::from_utf8(&len_bytes) else {
                    continue;
                };
                let Ok(len) = usize::from_str_radix(len_str, 16) else {
                    continue;
                };

                let mut service_bytes = vec![0u8; len];
                if socket.read_exact(&mut service_bytes).await.is_err() {
                    continue;
                }
                let service = String::from_utf8_lossy(&service_bytes);

                if service == "host:version" {
                    let resp = format!("OKAY0004{:04x}", version);
                    let _ = socket.write_all(resp.as_bytes()).await;
                } else if service == "host:kill" {
                    let _ = socket.write_all(b"OKAY").await;
                    let _ = socket.shutdown().await;
                    // Explicitly drop listener so port is released immediately
                    drop(listener.take());
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    break;
                }
            }
        });

        Self { port }
    }

    fn address(&self) -> ServerAddress {
        ServerAddress {
            host: None,
            port: self.port,
        }
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_1_and_2_start_server_when_none_running_and_second_start_does_nothing() {
    let port = get_free_port();
    let addr = ServerAddress { host: None, port };
    let _guard = KillOnDrop { addr: addr.clone() };

    let ref_adb = get_reference_adb_path();
    assert!(ref_adb.exists(), "Reference ADB not found at {:?}", ref_adb);

    let session_factory = DefaultSessionFactory::new();
    let launcher = ExternalBinaryLauncher::new(&ref_adb);
    let mut events = Vec::new();

    // 1. Starting when none is running
    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_ok(), "check_server_version failed: {:?}", res);

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

    // Verify host:version answers with expected version
    let mut session = session_factory.connect(&addr).await.unwrap();
    session.send_service("host:version").await.unwrap();
    session.read_status().await.unwrap();
    let ver_str = session.read_string().await.unwrap();
    let ver = u32::from_str_radix(&ver_str, 16).unwrap();
    assert_eq!(ver, ADB_SERVER_VERSION);

    // 2. Second start does nothing and prints no events
    let mut second_events = Vec::new();
    let second_res =
        check_server_version(&addr, &session_factory, &launcher, &mut second_events).await;
    assert!(second_res.is_ok());
    assert!(second_events.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_3_launcher_failure_produces_failed_to_start_daemon() {
    let port = get_free_port();
    let addr = ServerAddress { host: None, port };

    let session_factory = DefaultSessionFactory::new();
    let launcher = ExternalBinaryLauncher::new("/nonexistent/path/to/adb");
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_err());

    match res.unwrap_err() {
        ClientError::CannotConnectDaemon { .. } => {}
        err => panic!("Expected ClientError::CannotConnectDaemon, got {:?}", err),
    }

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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_4_version_mismatch_kills_fake_server_and_starts_real_one() {
    let fake_server = MockServer::start_with_version(40).await;
    let addr = fake_server.address();
    let _guard = KillOnDrop { addr: addr.clone() };

    let ref_adb = get_reference_adb_path();
    assert!(ref_adb.exists(), "Reference ADB not found at {:?}", ref_adb);

    let session_factory = DefaultSessionFactory::new();
    let launcher = ExternalBinaryLauncher::new(&ref_adb);
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_ok(), "check_server_version failed: {:?}", res);

    assert_eq!(
        events,
        vec![
            ClientEvent::ServerVersionMismatch {
                server: 40,
                client: 41,
            },
            ClientEvent::DaemonStarted,
        ]
    );

    assert_eq!(
        events[0].to_string(),
        "adb server version (40) doesn't match this client (41); killing..."
    );
    assert_eq!(events[1].to_string(), "* daemon started successfully");

    // Verify the newly started server answers with 41
    let mut session = session_factory.connect(&addr).await.unwrap();
    session.send_service("host:version").await.unwrap();
    session.read_status().await.unwrap();
    let ver_str = session.read_string().await.unwrap();
    let ver = u32::from_str_radix(&ver_str, 16).unwrap();
    assert_eq!(ver, ADB_SERVER_VERSION);
}

#[cfg(windows)]
#[tokio::test]
async fn test_windows_launcher_spawns_process() {
    use adb_client::ServerLauncher;

    let port = get_free_port();
    let addr = ServerAddress { host: None, port };
    let ref_adb = get_reference_adb_path();
    let launcher = ExternalBinaryLauncher::new(&ref_adb);
    let res = launcher.launch(&addr.to_spec(), None).await;
    // Process launch attempt on non-existent binary or real binary
    if ref_adb.exists() {
        assert!(res.is_ok());
    } else {
        assert!(res.is_err());
    }
}
