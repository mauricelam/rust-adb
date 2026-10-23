use adb_client::{
    check_server_version, kill_server, AdbClient, ClientEvent, DefaultSessionFactory, LaunchError,
    ServerAddress, ServerLauncher, ADB_SERVER_VERSION,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Copy)]
enum FakeVersionMode {
    Matching,
    Different(u32),
    UnknownHostService,
    Unparsable,
}

#[derive(Clone)]
enum FakeKillMode {
    Okay,
    Fail(String),
}

struct MockServer {
    port: u16,
}

impl MockServer {
    async fn start(version_mode: FakeVersionMode, kill_mode: FakeKillMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let v_mode = version_mode;
                let k_mode = kill_mode.clone();

                tokio::spawn(async move {
                    let mut len_bytes = [0u8; 4];
                    if socket.read_exact(&mut len_bytes).await.is_err() {
                        return;
                    }
                    let Ok(len_str) = std::str::from_utf8(&len_bytes) else {
                        return;
                    };
                    let Ok(len) = usize::from_str_radix(len_str, 16) else {
                        return;
                    };

                    let mut service_bytes = vec![0u8; len];
                    if socket.read_exact(&mut service_bytes).await.is_err() {
                        return;
                    }
                    let service = String::from_utf8_lossy(&service_bytes);

                    if service == "host:version" {
                        match v_mode {
                            FakeVersionMode::Matching => {
                                let resp = format!("OKAY0004{:04x}", ADB_SERVER_VERSION);
                                let _ = socket.write_all(resp.as_bytes()).await;
                            }
                            FakeVersionMode::Different(ver) => {
                                let resp = format!("OKAY0004{:04x}", ver);
                                let _ = socket.write_all(resp.as_bytes()).await;
                            }
                            FakeVersionMode::UnknownHostService => {
                                let reason = "unknown host service";
                                let resp = format!("FAIL{:04x}{}", reason.len(), reason);
                                let _ = socket.write_all(resp.as_bytes()).await;
                            }
                            FakeVersionMode::Unparsable => {
                                let resp = "OKAY0004XXXX";
                                let _ = socket.write_all(resp.as_bytes()).await;
                            }
                        }
                    } else if service == "host:kill" {
                        match k_mode {
                            FakeKillMode::Okay => {
                                let _ = socket.write_all(b"OKAY").await;
                                let _ = socket.shutdown().await;
                            }
                            FakeKillMode::Fail(reason) => {
                                let resp = format!("FAIL{:04x}{}", reason.len(), reason);
                                let _ = socket.write_all(resp.as_bytes()).await;
                            }
                        }
                    }
                });
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

struct MockLauncher {
    launched: Arc<AtomicBool>,
}

impl MockLauncher {
    fn new() -> (Self, Arc<AtomicBool>) {
        let launched = Arc::new(AtomicBool::new(false));
        (
            Self {
                launched: launched.clone(),
            },
            launched,
        )
    }
}

impl ServerLauncher for MockLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a str,
        _one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        let launched = self.launched.clone();
        Box::pin(async move {
            launched.store(true, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test]
async fn test_matching_version() {
    let server = MockServer::start(FakeVersionMode::Matching, FakeKillMode::Okay).await;
    let addr = server.address();
    let session_factory = DefaultSessionFactory;
    let (launcher, launched) = MockLauncher::new();
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_ok());
    assert!(events.is_empty());
    assert!(!launched.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_different_version_mismatch() {
    let server = MockServer::start(FakeVersionMode::Different(40), FakeKillMode::Okay).await;
    let addr = server.address();
    let session_factory = DefaultSessionFactory;
    let (launcher, launched) = MockLauncher::new();
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_ok());

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
    assert_eq!(
        events[0].to_string(),
        "adb server version (40) doesn't match this client (41); killing..."
    );
    assert!(launched.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_version_zero_from_unknown_host_service() {
    let server = MockServer::start(FakeVersionMode::UnknownHostService, FakeKillMode::Okay).await;
    let addr = server.address();
    let session_factory = DefaultSessionFactory;
    let (launcher, launched) = MockLauncher::new();
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_ok());

    assert_eq!(
        events,
        vec![
            ClientEvent::ServerVersionMismatch {
                server: 0,
                client: 41
            },
            ClientEvent::DaemonStarted,
        ]
    );
    assert_eq!(
        events[0].to_string(),
        "adb server version (0) doesn't match this client (41); killing..."
    );
    assert!(launched.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_unparsable_payload() {
    let server = MockServer::start(FakeVersionMode::Unparsable, FakeKillMode::Okay).await;
    let addr = server.address();
    let session_factory = DefaultSessionFactory;
    let (launcher, launched) = MockLauncher::new();
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.to_string(), "cannot parse version string: XXXX");
    assert!(!launched.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_remote_spec_nothing_listening() {
    let addr = ServerAddress {
        host: Some("remote.host.invalid".into()), // is_local() == false
        port: 59997,
    };
    let session_factory = DefaultSessionFactory;
    let (launcher, _launched) = MockLauncher::new();
    let mut events = Vec::new();

    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_err());
    assert_eq!(events, vec![ClientEvent::CannotStartRemote]);
    assert_eq!(
        events[0].to_string(),
        "* cannot start server on remote host"
    );
}

#[tokio::test]
async fn test_kill_when_nothing_listening() {
    // Port 59999 has nothing listening locally
    let addr = ServerAddress {
        host: None,
        port: 59999,
    };
    let session_factory = DefaultSessionFactory;
    let mut events = Vec::new();

    let res = kill_server(&addr, &session_factory, &mut events).await;
    assert!(res.is_ok());
    assert_eq!(events.len(), 1);
    match &events[0] {
        ClientEvent::CannotConnectDaemon { spec, cause } => {
            assert_eq!(spec, "tcp:59999");
            assert!(!cause.is_empty());
        }
        _ => panic!("Expected CannotConnectDaemon event"),
    }
    assert!(events[0]
        .to_string()
        .starts_with("cannot connect to daemon at tcp:59999:"));
}

#[tokio::test]
async fn test_kill_server_fail() {
    let server = MockServer::start(
        FakeVersionMode::Matching,
        FakeKillMode::Fail("cannot kill".into()),
    )
    .await;
    let addr = server.address();
    let session_factory = DefaultSessionFactory;
    let mut events = Vec::new();

    let res = kill_server(&addr, &session_factory, &mut events).await;
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert_eq!(err.to_string(), "cannot kill");
    assert_eq!(events, vec![ClientEvent::ErrorLine("error: cannot kill".into())]);
    assert_eq!(events[0].to_string(), "error: cannot kill");
}

#[tokio::test]
async fn test_adb_client_caching() {
    let server = MockServer::start(FakeVersionMode::Matching, FakeKillMode::Okay).await;
    let (launcher, _launched) = MockLauncher::new();
    let client = AdbClient::builder(server.address())
        .session_factory(DefaultSessionFactory)
        .launcher(launcher)
        .build();

    let outcome1 = client.check_server_version().await;
    assert!(outcome1.result.is_ok());

    let outcome2 = client.check_server_version().await;
    assert!(outcome2.result.is_ok());
    assert!(outcome2.events.is_empty()); // Cached result, no second check or duplicate events
}
