//! Socket specification definitions for ADB.
//!
//! Ported from socket spec implementations in `original/adb_utils.cpp` and `original/sockets.cpp`.

use std::fmt;
use std::str::FromStr;

/// Parsed socket specification for forward, reverse, or listener specifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketSpec {
    /// TCP socket with optional host (None = localhost) and port.
    Tcp { host: Option<String>, port: u16 },
    /// Unix abstract domain socket (`localabstract:<name>`).
    LocalAbstract(String),
    /// Unix reserved domain socket (`localreserved:<name>`).
    LocalReserved(String),
    /// Unix filesystem domain socket (`localfilesystem:<path>` or `local:<path>`).
    LocalFilesystem(String),
    /// Device raw byte streaming (`dev-raw:<device>`).
    DevRaw(String),
    /// Device char streaming (`dev:<device>`).
    Dev(String),
    /// JDWP process connection (`jdwp:<pid>`).
    Jdwp(u32),
    /// Accept spec (`accept:<path>`).
    Accept(String),
    /// Vsock connection (`vsock:<cid>:<port>`).
    Vsock { cid: u32, port: u32 },
    /// Other unparsed or passthrough specification.
    Other(String),
}

/// Error type when parsing a [`SocketSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseSocketSpecError {
    InvalidPort(String),
    InvalidSpec(String),
}

impl fmt::Display for ParseSocketSpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPort(p) => write!(f, "invalid port: {p}"),
            Self::InvalidSpec(s) => write!(f, "invalid socket specification: {s}"),
        }
    }
}

impl std::error::Error for ParseSocketSpecError {}

impl SocketSpec {
    /// Returns true if this is a TCP socket spec.
    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp { .. })
    }

    /// Returns the port if this is a TCP socket spec.
    pub fn tcp_port(&self) -> Option<u16> {
        match self {
            Self::Tcp { port, .. } => Some(*port),
            _ => None,
        }
    }
}

impl FromStr for SocketSpec {
    type Err = ParseSocketSpecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(rest) = s.strip_prefix("tcp:") {
            if let Some((host, port_str)) = rest.rsplit_once(':') {
                let port = port_str
                    .parse::<u16>()
                    .map_err(|_| ParseSocketSpecError::InvalidPort(port_str.to_string()))?;
                let host_val = if host.is_empty() {
                    None
                } else {
                    Some(host.to_string())
                };
                Ok(Self::Tcp {
                    host: host_val,
                    port,
                })
            } else {
                let port = rest
                    .parse::<u16>()
                    .map_err(|_| ParseSocketSpecError::InvalidPort(rest.to_string()))?;
                Ok(Self::Tcp { host: None, port })
            }
        } else if let Some(name) = s.strip_prefix("localabstract:") {
            Ok(Self::LocalAbstract(name.to_string()))
        } else if let Some(name) = s.strip_prefix("localreserved:") {
            Ok(Self::LocalReserved(name.to_string()))
        } else if let Some(path) = s.strip_prefix("localfilesystem:") {
            Ok(Self::LocalFilesystem(path.to_string()))
        } else if let Some(path) = s.strip_prefix("local:") {
            Ok(Self::LocalFilesystem(path.to_string()))
        } else if let Some(dev) = s.strip_prefix("dev-raw:") {
            Ok(Self::DevRaw(dev.to_string()))
        } else if let Some(dev) = s.strip_prefix("dev:") {
            Ok(Self::Dev(dev.to_string()))
        } else if let Some(pid_str) = s.strip_prefix("jdwp:") {
            let pid = pid_str
                .parse::<u32>()
                .map_err(|_| ParseSocketSpecError::InvalidSpec(s.to_string()))?;
            Ok(Self::Jdwp(pid))
        } else if let Some(path) = s.strip_prefix("accept:") {
            Ok(Self::Accept(path.to_string()))
        } else if let Some(rest) = s.strip_prefix("vsock:") {
            if let Some((cid_str, port_str)) = rest.split_once(':') {
                let cid = cid_str
                    .parse::<u32>()
                    .map_err(|_| ParseSocketSpecError::InvalidSpec(s.to_string()))?;
                let port = port_str
                    .parse::<u32>()
                    .map_err(|_| ParseSocketSpecError::InvalidPort(port_str.to_string()))?;
                Ok(Self::Vsock { cid, port })
            } else {
                Err(ParseSocketSpecError::InvalidSpec(s.to_string()))
            }
        } else {
            Ok(Self::Other(s.to_string()))
        }
    }
}

impl fmt::Display for SocketSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp { host, port } => match host {
                Some(h) => write!(f, "tcp:{h}:{port}"),
                None => write!(f, "tcp:{port}"),
            },
            Self::LocalAbstract(name) => write!(f, "localabstract:{name}"),
            Self::LocalReserved(name) => write!(f, "localreserved:{name}"),
            Self::LocalFilesystem(path) => write!(f, "localfilesystem:{path}"),
            Self::DevRaw(dev) => write!(f, "dev-raw:{dev}"),
            Self::Dev(dev) => write!(f, "dev:{dev}"),
            Self::Jdwp(pid) => write!(f, "jdwp:{pid}"),
            Self::Accept(path) => write!(f, "accept:{path}"),
            Self::Vsock { cid, port } => write!(f, "vsock:{cid}:{port}"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}
