use super::error::ClientError;

/// Events reported by client operations (e.g. version checks, server startup, kill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    ServerVersionMismatch { server: u32, client: u32 },
    DaemonNotRunning { spec: String },
    CannotStartRemote,
    CannotConnectDaemon { spec: String, cause: String },
    ErrorLine(String),
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
            ClientEvent::ErrorLine(msg) => {
                write!(f, "{msg}")
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

/// Result of an `AdbClient` operation paired with events emitted during version check or daemon management.
#[derive(Debug)]
pub struct Outcome<T, E = ClientError> {
    pub result: Result<T, E>,
    pub events: Vec<ClientEvent>,
}
