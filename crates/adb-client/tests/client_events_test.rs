use adb_client::{
    AdbClient, ClientError, ClientEvent, LaunchError, ServerAddress, ServerLauncher,
    TransportSelector, ADB_SERVER_VERSION,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpSocket;

struct MockServer {
    port: u16,
}

impl MockServer {
    async fn start_with_port(
        port: u16,
        version: u32,
    ) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        let bind_addr = format!("127.0.0.1:{port}").parse().unwrap();
        socket.bind(bind_addr).unwrap();
        let listener = socket.listen(1024).unwrap();
        let actual_port = listener.local_addr().unwrap().port();

        let (kill_tx, kill_rx) = tokio::sync::oneshot::channel();
        let kill_tx = Arc::new(tokio::sync::Mutex::new(Some(kill_tx)));

        let (shutdown_listener_tx, mut shutdown_listener_rx) =
            tokio::sync::oneshot::channel::<()>();
        let shutdown_listener_tx = Arc::new(tokio::sync::Mutex::new(Some(shutdown_listener_tx)));

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_listener_rx => break,
                    res = listener.accept() => {
                        let Ok((mut socket, _)) = res else {
                            break;
                        };
                        let kill_tx_clone = kill_tx.clone();
                        let shutdown_listener_tx_clone = shutdown_listener_tx.clone();

                        tokio::spawn(async move {
                            loop {
                                let mut len_bytes = [0u8; 4];
                                if socket.read_exact(&mut len_bytes).await.is_err() {
                                    break;
                                }
                                let Ok(len_str) = std::str::from_utf8(&len_bytes) else {
                                    break;
                                };
                                let Ok(len) = usize::from_str_radix(len_str, 16) else {
                                    break;
                                };

                                let mut service_bytes = vec![0u8; len];
                                if socket.read_exact(&mut service_bytes).await.is_err() {
                                    break;
                                }
                                let service = String::from_utf8_lossy(&service_bytes);

                                if service == "host:version" {
                                    let resp = format!("OKAY0004{:04x}", version);
                                    let _ = socket.write_all(resp.as_bytes()).await;
                                    let _ = socket.shutdown().await;
                                    break;
                                } else if service == "host:kill" {
                                    let _ = socket.write_all(b"OKAY").await;
                                    let _ = socket.shutdown().await;
                                    if let Some(tx) = kill_tx_clone.lock().await.take() {
                                        let _ = tx.send(());
                                    }
                                    if let Some(tx) = shutdown_listener_tx_clone.lock().await.take() {
                                        let _ = tx.send(());
                                    }
                                    break;
                                } else if service == "host:devices" || service == "host:devices-l" {
                                    let resp = "OKAY0000";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                } else if service.contains("get-state") {
                                    let resp = "OKAY0006device";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                } else if service.contains("get-serialno") {
                                    let resp = "OKAY0006123456";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                } else if service.contains("get-devpath") {
                                    let resp = "OKAY0007unknown";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                } else if service == "host:features"
                                    || service == "host:host-features"
                                    || service.contains("features")
                                {
                                    let resp = "OKAY000ecmd,shell_v2,ls";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                } else if service.contains("transport-id") || service.contains("tport") {
                                    let resp = "OKAY";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                    if socket.write_all(&1u64.to_ne_bytes()).await.is_err() {
                                        break;
                                    }
                                } else {
                                    let resp = "FAIL000bunknown cmd";
                                    if socket.write_all(resp.as_bytes()).await.is_err() {
                                        break;
                                    }
                                }
                            }
                        });
                    }
                }
            }
        });

        (Self { port: actual_port }, kill_rx)
    }

    async fn start(version: u32) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        Self::start_with_port(0, version).await
    }

    fn address(&self) -> ServerAddress {
        ServerAddress {
            host: None,
            port: self.port,
        }
    }
}

struct DelayedLauncher {
    port: u16,
    launched: Arc<AtomicBool>,
}

impl DelayedLauncher {
    fn new(port: u16) -> (Self, Arc<AtomicBool>) {
        let launched = Arc::new(AtomicBool::new(false));
        (
            Self {
                port,
                launched: launched.clone(),
            },
            launched,
        )
    }
}

impl ServerLauncher for DelayedLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a str,
        _one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        let port = self.port;
        let launched = self.launched.clone();
        Box::pin(async move {
            launched.store(true, Ordering::SeqCst);
            // Start server listening on port with matching version 41
            let _ = MockServer::start_with_port(port, ADB_SERVER_VERSION).await;
            Ok(())
        })
    }
}

struct FailingLauncher;

impl ServerLauncher for FailingLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a str,
        _one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move {
            Err(LaunchError::Unsupported(
                "failed to launch server".to_string(),
            ))
        })
    }
}

enum QueryOp {
    Devices,
    GetState,
    GetSerialNo,
    GetDevPath,
    TransportId,
    Features,
    HostFeatures,
}

async fn execute_op(client: &AdbClient, op: QueryOp) -> adb_client::Outcome<String> {
    let selector = TransportSelector::Any;
    match op {
        QueryOp::Devices => client.devices(false).await,
        QueryOp::GetState => client.get_state(&selector).await,
        QueryOp::GetSerialNo => client.get_serialno(&selector).await,
        QueryOp::GetDevPath => client.get_devpath(&selector).await,
        QueryOp::TransportId => client.transport_id(&selector).await,
        QueryOp::Features => client.features(&selector).await,
        QueryOp::HostFeatures => client.host_features().await,
    }
}

fn all_ops() -> Vec<QueryOp> {
    vec![
        QueryOp::Devices,
        QueryOp::GetState,
        QueryOp::GetSerialNo,
        QueryOp::GetDevPath,
        QueryOp::TransportId,
        QueryOp::Features,
        QueryOp::HostFeatures,
    ]
}

#[tokio::test]
async fn test_nothing_listening_starts_daemon_and_queries() {
    for op in all_ops() {
        // Reserve an unused local port
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);

        let addr = ServerAddress { host: None, port };
        let (launcher, launched) = DelayedLauncher::new(port);
        let client = AdbClient::builder(addr.clone()).launcher(launcher).build();

        let outcome = execute_op(&client, op).await;
        assert!(
            outcome.result.is_ok(),
            "Operation failed: {:?}",
            outcome.result
        );
        assert!(launched.load(Ordering::SeqCst));

        let expected_spec = addr.to_spec();
        assert_eq!(
            outcome.events,
            vec![
                ClientEvent::DaemonNotRunning {
                    spec: expected_spec
                },
                ClientEvent::DaemonStarted,
            ]
        );
    }
}

#[tokio::test]
async fn test_wrong_server_version_restarts_daemon() {
    for op in all_ops() {
        // Start server with version 40
        let (server, _kill_rx) = MockServer::start(40).await;
        let addr = server.address();
        let (launcher, launched) = DelayedLauncher::new(addr.port);

        let client = AdbClient::builder(addr.clone()).launcher(launcher).build();

        let outcome = execute_op(&client, op).await;
        assert!(
            outcome.result.is_ok(),
            "Operation failed: {:?}",
            outcome.result
        );
        assert!(launched.load(Ordering::SeqCst));

        assert_eq!(
            outcome.events,
            vec![
                ClientEvent::ServerVersionMismatch {
                    server: 40,
                    client: 41
                },
                ClientEvent::DaemonStarted,
            ]
        );
    }
}

#[tokio::test]
async fn test_launcher_failure_preserves_events() {
    for op in all_ops() {
        // Reserve an unused local port
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);

        let addr = ServerAddress { host: None, port };
        let client = AdbClient::builder(addr.clone())
            .launcher(FailingLauncher)
            .build();

        let outcome = execute_op(&client, op).await;
        assert!(
            matches!(outcome.result, Err(ClientError::CannotConnectDaemon)),
            "Expected CannotConnectDaemon, got {:?}",
            outcome.result
        );

        let expected_spec = addr.to_spec();
        assert_eq!(
            outcome.events,
            vec![
                ClientEvent::DaemonNotRunning {
                    spec: expected_spec
                },
                ClientEvent::FailedToStartDaemon,
            ]
        );
    }
}

#[tokio::test]
async fn test_two_calls_emit_version_check_events_only_once() {
    // Reserve unused port
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);

    let addr = ServerAddress { host: None, port };
    let (launcher, _launched) = DelayedLauncher::new(port);
    let client = AdbClient::builder(addr.clone()).launcher(launcher).build();

    // Call 1
    let outcome1 = client.devices(false).await;
    assert!(outcome1.result.is_ok());
    assert_eq!(
        outcome1.events,
        vec![
            ClientEvent::DaemonNotRunning {
                spec: addr.to_spec()
            },
            ClientEvent::DaemonStarted,
        ]
    );

    // Call 2
    let outcome2 = client.devices(false).await;
    assert!(outcome2.result.is_ok());
    assert!(
        outcome2.events.is_empty(),
        "Second call on same client instance should emit empty events, got {:?}",
        outcome2.events
    );
}
