use crate::session::SessionError;

/// Classification of client errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientErrorKind {
    Server,
    Connection,
    Protocol,
    Usage,
    Version,
    Launch,
    Timeout,
}

/// Errors that can occur during high-level client operations.
#[derive(Debug, Clone)]
pub enum ClientError {
    Session(SessionError),
    CannotConnect { spec: String, cause: String },
    CannotConnectDaemon { source: Option<LaunchError> },
    CannotParseVersionString(String),
    KillFailed(String),
    FailedToCheckVersion(String),
    QueryFailed(String),
    InvalidUsage(String),
    Timeout,
}

impl ClientError {
    /// Returns the classification category of this client error.
    pub fn kind(&self) -> ClientErrorKind {
        match self {
            ClientError::Session(err) => match err {
                SessionError::CannotConnect { .. } => ClientErrorKind::Connection,
                SessionError::Fail(_) => ClientErrorKind::Server,
                SessionError::Timeout => ClientErrorKind::Timeout,
                SessionError::BadServiceLength(_)
                | SessionError::WriteFailure(_)
                | SessionError::ReadTransportIdFailed
                | SessionError::ReadStatusFailed(_)
                | SessionError::ReadStatusLengthFailed(_)
                | SessionError::ReadStatusMessageFailed(_)
                | SessionError::InvalidStatus(_) => ClientErrorKind::Protocol,
            },
            ClientError::CannotConnect { .. } => ClientErrorKind::Connection,
            ClientError::CannotConnectDaemon { source } => {
                if let Some(LaunchError::Timeout(_)) = source {
                    ClientErrorKind::Timeout
                } else if source.is_some() {
                    ClientErrorKind::Launch
                } else {
                    ClientErrorKind::Connection
                }
            }
            ClientError::CannotParseVersionString(_) => ClientErrorKind::Version,
            ClientError::KillFailed(_) => ClientErrorKind::Server,
            ClientError::FailedToCheckVersion(_) => ClientErrorKind::Server,
            ClientError::QueryFailed(_) => ClientErrorKind::Server,
            ClientError::InvalidUsage(_) => ClientErrorKind::Usage,
            ClientError::Timeout => ClientErrorKind::Timeout,
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Session(err) => write!(f, "{err}"),
            ClientError::CannotConnect { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            ClientError::CannotConnectDaemon { .. } => {
                write!(f, "cannot connect to daemon")
            }
            ClientError::CannotParseVersionString(ver) => {
                write!(f, "cannot parse version string: {ver}")
            }
            ClientError::KillFailed(msg) => write!(f, "{msg}"),
            ClientError::FailedToCheckVersion(err) => {
                write!(f, "failed to check server version: {err}")
            }
            ClientError::QueryFailed(msg) => write!(f, "{msg}"),
            ClientError::InvalidUsage(msg) => write!(f, "{msg}"),
            ClientError::Timeout => write!(f, "timeout"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Session(err) => Some(err),
            ClientError::CannotConnectDaemon { source: Some(err) } => Some(err),
            _ => None,
        }
    }
}

impl From<SessionError> for ClientError {
    fn from(err: SessionError) -> Self {
        ClientError::Session(err)
    }
}

/// Errors that can occur during ADB server process launching.
#[derive(Debug)]
pub enum LaunchError {
    Io(std::io::Error),
    FailedToReadAck(String),
    Unsupported(String),
    Timeout(String),
}

impl Clone for LaunchError {
    fn clone(&self) -> Self {
        match self {
            LaunchError::Io(err) => {
                LaunchError::Io(std::io::Error::new(err.kind(), err.to_string()))
            }
            LaunchError::FailedToReadAck(msg) => LaunchError::FailedToReadAck(msg.clone()),
            LaunchError::Unsupported(msg) => LaunchError::Unsupported(msg.clone()),
            LaunchError::Timeout(msg) => LaunchError::Timeout(msg.clone()),
        }
    }
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Io(err) => write!(f, "IO error: {err}"),
            LaunchError::FailedToReadAck(msg) => write!(f, "{msg}"),
            LaunchError::Unsupported(msg) => write!(f, "{msg}"),
            LaunchError::Timeout(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LaunchError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for LaunchError {
    fn from(err: std::io::Error) -> Self {
        LaunchError::Io(err)
    }
}
