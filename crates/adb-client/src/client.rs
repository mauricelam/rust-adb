use crate::server_address::ServerAddress;
use crate::session::{Session, SessionError};
use std::future::Future;
use std::pin::Pin;

/// Expected ADB server version matching C++ `ADB_SERVER_VERSION` in `original/adb.h`.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Events reported by client operations (e.g. version checks, server startup, kill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    ServerVersionMismatch { server: u32, client: u32 },
    DaemonNotRunning { spec: String },
    CannotStartRemote,
    CannotConnectDaemon { spec: String, cause: String },
    Error(String),
    FailedToStartDaemon,
    DaemonStarted,
}

impl std::fmt::Display for ClientEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientEvent::ServerVersionMismatch { server, client } => {
                write!(
                    f,
                    "adb server version ({server}) doesn't match this client ({client}); killing..."
                )
            }
            ClientEvent::DaemonNotRunning { spec } => {
                write!(f, "* daemon not running; starting now at {spec}")
            }
            ClientEvent::CannotStartRemote => {
                write!(f, "* cannot start server on remote host")
            }
            ClientEvent::CannotConnectDaemon { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            ClientEvent::Error(msg) => {
                if msg.starts_with("error:") || msg.starts_with("cannot connect") {
                    write!(f, "{msg}")
                } else {
                    write!(f, "error: {msg}")
                }
            }
            ClientEvent::FailedToStartDaemon => {
                write!(f, "* failed to start daemon")
            }
            ClientEvent::DaemonStarted => {
                write!(f, "* daemon started successfully")
            }
        }
    }
}

/// Errors that can occur during high-level client operations.
#[derive(Debug)]
pub enum ClientError {
    Session(SessionError),
    CannotConnect { spec: String, cause: String },
    CannotConnectDaemon,
    CannotParseVersionString(String),
    KillFailed(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Session(err) => write!(f, "{err}"),
            ClientError::CannotConnect { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            ClientError::CannotConnectDaemon => {
                write!(f, "cannot connect to daemon")
            }
            ClientError::CannotParseVersionString(ver) => {
                write!(f, "cannot parse version string: {ver}")
            }
            ClientError::KillFailed(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Session(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SessionError> for ClientError {
    fn from(err: SessionError) -> Self {
        ClientError::Session(err)
    }
}

/// Trait for creating sessions to an ADB server address.
pub trait SessionFactory: Send + Sync {
    fn connect<'a>(
        &'a self,
        addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<Session, SessionError>> + Send + 'a>>;
}

/// Default implementation of `SessionFactory` using `Session::connect`.
pub struct DefaultSessionFactory;

impl SessionFactory for DefaultSessionFactory {
    fn connect<'a>(
        &'a self,
        addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<Session, SessionError>> + Send + 'a>> {
        Box::pin(async move { Session::connect(addr).await })
    }
}

/// Trait for launching an ADB server process.
pub trait ServerLauncher: Send + Sync {
    fn launch_server<'a>(
        &'a self,
        addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + 'a>>;
}

/// Kills the running ADB server at `addr`.
///
/// Ports `adb_kill_server` from C++ `adb_client.cpp`.
pub async fn kill_server(
    addr: &ServerAddress,
    session_factory: &dyn SessionFactory,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    let mut session = match session_factory.connect(addr).await {
        Ok(s) => s,
        Err(err) => {
            let cause = match &err {
                SessionError::CannotConnect { cause, .. } => cause.clone(),
                _ => err.to_string(),
            };
            events.push(ClientEvent::CannotConnectDaemon {
                spec: addr.to_spec(),
                cause,
            });
            return Ok(());
        }
    };

    if let Err(err) = session.send_service("host:kill").await {
        let msg = format!("write failure during connection: {err}");
        events.push(ClientEvent::Error(msg.clone()));
        return Err(ClientError::KillFailed(msg));
    }

    match session.read_status().await {
        Ok(()) => {
            let _ = session.read_to_close().await;
            Ok(())
        }
        Err(SessionError::Fail(reason)) => {
            events.push(ClientEvent::Error(reason.clone()));
            Err(ClientError::KillFailed(reason))
        }
        Err(_) => {
            let msg = "failed to read response from server".to_string();
            events.push(ClientEvent::Error(msg.clone()));
            Err(ClientError::KillFailed(msg))
        }
    }
}

/// Checks that the running server version matches `ADB_SERVER_VERSION`.
///
/// Ports `__adb_check_server_version` from C++ `adb_client.cpp`.
pub async fn check_server_version(
    addr: &ServerAddress,
    session_factory: &dyn SessionFactory,
    launcher: &dyn ServerLauncher,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    let connect_res = session_factory.connect(addr).await;
    let local = addr.is_local();

    let mut session = match connect_res {
        Err(SessionError::CannotConnect { cause, .. }) => {
            if !local {
                events.push(ClientEvent::CannotStartRemote);
                return Err(SessionError::CannotConnect {
                    spec: addr.to_spec(),
                    cause,
                }
                .into());
            } else {
                events.push(ClientEvent::DaemonNotRunning {
                    spec: addr.to_spec(),
                });
                return start_server(addr, launcher, events).await;
            }
        }
        Err(err) => return Err(err.into()),
        Ok(s) => s,
    };

    if let Err(err) = session.send_service("host:version").await {
        return Err(err.into());
    }

    let version = match session.read_status().await {
        Ok(()) => {
            let version_string = session.read_string().await?;
            let _ = session.read_to_close().await;

            match u32::from_str_radix(&version_string, 16) {
                Ok(v) => v,
                Err(_) => {
                    return Err(ClientError::CannotParseVersionString(version_string));
                }
            }
        }
        Err(SessionError::Fail(reason)) if reason == "unknown host service" => 0,
        Err(err) => return Err(err.into()),
    };

    if version != ADB_SERVER_VERSION {
        events.push(ClientEvent::ServerVersionMismatch {
            server: version,
            client: ADB_SERVER_VERSION,
        });

        kill_server(addr, session_factory, events).await?;
        return start_server(addr, launcher, events).await;
    }

    Ok(())
}

async fn start_server(
    addr: &ServerAddress,
    launcher: &dyn ServerLauncher,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    match launcher.launch_server(addr).await {
        Ok(()) => {
            events.push(ClientEvent::DaemonStarted);
            Ok(())
        }
        Err(err) => {
            events.push(ClientEvent::FailedToStartDaemon);
            Err(err)
        }
    }
}

/// Represents an ADB client instance associated with a server address.
pub struct AdbClient {
    addr: ServerAddress,
    version_checked: tokio::sync::OnceCell<Result<(), String>>,
}

impl AdbClient {
    /// Creates a new `AdbClient` targeting `addr`.
    pub fn new(addr: ServerAddress) -> Self {
        Self {
            addr,
            version_checked: tokio::sync::OnceCell::new(),
        }
    }

    /// Returns a reference to the server address.
    pub fn addr(&self) -> &ServerAddress {
        &self.addr
    }

    /// Checks the server version once per `AdbClient` instance.
    pub async fn check_server_version(
        &self,
        session_factory: &dyn SessionFactory,
        launcher: &dyn ServerLauncher,
        events: &mut Vec<ClientEvent>,
    ) -> Result<(), ClientError> {
        let res: &Result<(), String> = self
            .version_checked
            .get_or_init(|| async {
                let mut check_events = Vec::new();
                let res =
                    check_server_version(&self.addr, session_factory, launcher, &mut check_events)
                        .await;
                events.extend(check_events);
                match res {
                    Ok(()) => Ok(()),
                    Err(e) => Err(e.to_string()),
                }
            })
            .await;

        match res {
            Ok(()) => Ok(()),
            Err(msg) => Err(ClientError::KillFailed(msg.clone())),
        }
    }

    /// Kills the ADB server for this client instance.
    pub async fn kill_server(
        &self,
        session_factory: &dyn SessionFactory,
        events: &mut Vec<ClientEvent>,
    ) -> Result<(), ClientError> {
        kill_server(&self.addr, session_factory, events).await
    }
}
