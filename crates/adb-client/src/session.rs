use crate::server_address::ServerAddress;
use crate::transport::TransportSelector;
use adb_proto::smart_socket::{
    parse_protocol_string, parse_reply, ProtocolString, SmartSocketError, SmartSocketReply,
};
use adb_proto::MAX_PAYLOAD;
use std::fmt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Configuration for smart-socket sessions (timeouts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    pub connect_timeout: std::time::Duration,
    pub handshake_timeout: std::time::Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            connect_timeout: std::time::Duration::from_secs(10),
            handshake_timeout: std::time::Duration::from_secs(30),
        }
    }
}

/// Errors that can occur during an ADB smart-socket session.
#[derive(Debug)]
pub enum SessionError {
    /// Service name length error (empty or > MAX_PAYLOAD)
    BadServiceLength(usize),
    /// Connection to daemon failed
    CannotConnect { spec: String, cause: String },
    /// Write failure during connection
    WriteFailure(std::io::Error),
    /// Failed to read transport id from server
    ReadTransportIdFailed,
    /// Couldn't read status (0..3 bytes before EOF or IO error)
    ReadStatusFailed(std::io::Error),
    /// Couldn't read status length prefix for FAIL
    ReadStatusLengthFailed(std::io::Error),
    /// Couldn't read status message for FAIL
    ReadStatusMessageFailed(std::io::Error),
    /// Neither OKAY nor FAIL status bytes received
    InvalidStatus([u8; 4]),
    /// Server returned FAIL status with a reason message
    Fail(String),
    /// Operation timed out
    Timeout,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::BadServiceLength(len) => {
                write!(f, "bad service name length ({len})")
            }
            SessionError::CannotConnect { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            SessionError::WriteFailure(err) => {
                write!(f, "write failure during connection: {err}")
            }
            SessionError::ReadTransportIdFailed => {
                write!(f, "failed to read transport id from server")
            }
            SessionError::ReadStatusFailed(err) => {
                write!(f, "protocol fault (couldn't read status): {err}")
            }
            SessionError::ReadStatusLengthFailed(err) => {
                write!(f, "protocol fault (couldn't read status length): {err}")
            }
            SessionError::ReadStatusMessageFailed(err) => {
                write!(f, "protocol fault (couldn't read status message): {err}")
            }
            SessionError::InvalidStatus(status) => {
                write!(
                    f,
                    "protocol fault (status {:02x} {:02x} {:02x} {:02x}?!)",
                    status[0], status[1], status[2], status[3]
                )
            }
            SessionError::Fail(reason) => {
                write!(f, "{reason}")
            }
            SessionError::Timeout => {
                write!(f, "timeout")
            }
        }
    }
}

impl Clone for SessionError {
    fn clone(&self) -> Self {
        match self {
            SessionError::BadServiceLength(len) => SessionError::BadServiceLength(*len),
            SessionError::CannotConnect { spec, cause } => SessionError::CannotConnect {
                spec: spec.clone(),
                cause: cause.clone(),
            },
            SessionError::WriteFailure(err) => {
                SessionError::WriteFailure(std::io::Error::new(err.kind(), err.to_string()))
            }
            SessionError::ReadTransportIdFailed => SessionError::ReadTransportIdFailed,
            SessionError::ReadStatusFailed(err) => {
                SessionError::ReadStatusFailed(std::io::Error::new(err.kind(), err.to_string()))
            }
            SessionError::ReadStatusLengthFailed(err) => SessionError::ReadStatusLengthFailed(
                std::io::Error::new(err.kind(), err.to_string()),
            ),
            SessionError::ReadStatusMessageFailed(err) => SessionError::ReadStatusMessageFailed(
                std::io::Error::new(err.kind(), err.to_string()),
            ),
            SessionError::InvalidStatus(status) => SessionError::InvalidStatus(*status),
            SessionError::Fail(reason) => SessionError::Fail(reason.clone()),
            SessionError::Timeout => SessionError::Timeout,
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SessionError::WriteFailure(err)
            | SessionError::ReadStatusFailed(err)
            | SessionError::ReadStatusLengthFailed(err)
            | SessionError::ReadStatusMessageFailed(err) => Some(err),
            _ => None,
        }
    }
}

/// Represents an active smart-socket session with an ADB server.
#[derive(Debug)]
pub struct Session {
    stream: TcpStream,
    read_buf: Vec<u8>,
    config: SessionConfig,
}

impl Session {
    /// Creates a new `Session` wrapping an existing `TcpStream` with default configuration.
    pub fn new(stream: TcpStream) -> Self {
        Self::new_with_config(stream, SessionConfig::default())
    }

    /// Creates a new `Session` wrapping an existing `TcpStream` with specific configuration.
    pub fn new_with_config(stream: TcpStream, config: SessionConfig) -> Self {
        Self {
            stream,
            read_buf: Vec::new(),
            config,
        }
    }

    /// Connects to the ADB server at the given address using default configuration.
    pub async fn connect(addr: &ServerAddress) -> Result<Self, SessionError> {
        Self::connect_with_config(addr, SessionConfig::default()).await
    }

    /// Connects to the ADB server at the given address using specified `SessionConfig`.
    ///
    /// Ports `_adb_connect` socket connection phase with connect timeout.
    pub async fn connect_with_config(
        addr: &ServerAddress,
        config: SessionConfig,
    ) -> Result<Self, SessionError> {
        let spec = addr.to_spec();
        let host = addr.host.as_deref().unwrap_or("127.0.0.1");
        let host = if host == "localhost" {
            "127.0.0.1"
        } else {
            host
        };

        let conn_res = tokio::time::timeout(
            config.connect_timeout,
            TcpStream::connect((host, addr.port)),
        )
        .await;

        match conn_res {
            Ok(Ok(stream)) => Ok(Self {
                stream,
                read_buf: Vec::new(),
                config,
            }),
            Ok(Err(err)) => Err(SessionError::CannotConnect {
                spec,
                cause: err.to_string(),
            }),
            Err(_) => Err(SessionError::Timeout),
        }
    }

    /// Selects a device transport on this connection.
    ///
    /// Ports `switch_socket_transport` from C++ `adb_client.cpp`.
    ///
    /// Returns `Ok(Some(transport_id))` if transport switch succeeded and a transport ID was read/set,
    /// or `Ok(None)` for host transport.
    pub async fn select_transport(
        &mut self,
        selector: &TransportSelector,
    ) -> Result<Option<u64>, SessionError> {
        let (service, read_transport) = match selector {
            TransportSelector::Id(id) => (format!("host:transport-id:{id}"), false),
            TransportSelector::Serial(s) => (format!("host:tport:serial:{s}"), true),
            TransportSelector::Usb => ("host:tport:usb".to_string(), true),
            TransportSelector::Local => ("host:tport:local".to_string(), true),
            TransportSelector::Any => ("host:tport:any".to_string(), true),
        };

        let req_bytes = adb_proto::smart_socket::encode_request(&service)
            .map_err(|_| SessionError::BadServiceLength(service.len()))?;

        self.stream
            .write_all(&req_bytes)
            .await
            .map_err(SessionError::WriteFailure)?;

        self.read_status().await?;

        if read_transport {
            let mut buf = [0u8; 8];
            let mut read_bytes = 0;
            while read_bytes < 8 {
                if !self.read_buf.is_empty() {
                    let avail = (8 - read_bytes).min(self.read_buf.len());
                    buf[read_bytes..read_bytes + avail].copy_from_slice(&self.read_buf[..avail]);
                    self.read_buf.drain(..avail);
                    read_bytes += avail;
                } else {
                    self.stream
                        .read_exact(&mut buf[read_bytes..])
                        .await
                        .map_err(|_| SessionError::ReadTransportIdFailed)?;
                    read_bytes = 8;
                }
            }
            let id = u64::from_ne_bytes(buf);
            Ok(Some(id))
        } else if let TransportSelector::Id(id) = selector {
            Ok(Some(*id))
        } else {
            Ok(None)
        }
    }

    /// Sends a service name over the connection.
    ///
    /// Validates service name length (1..=MAX_PAYLOAD).
    pub async fn send_service(&mut self, service: &str) -> Result<(), SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        let req_bytes = adb_proto::smart_socket::encode_request(service)
            .map_err(|_| SessionError::BadServiceLength(service.len()))?;

        self.stream
            .write_all(&req_bytes)
            .await
            .map_err(SessionError::WriteFailure)?;

        Ok(())
    }

    /// Reads smart-socket status response (`OKAY` or `FAIL` + length-prefixed reason), subject to handshake timeout.
    pub async fn read_status(&mut self) -> Result<(), SessionError> {
        let res =
            tokio::time::timeout(self.config.handshake_timeout, self.read_status_internal()).await;
        match res {
            Ok(r) => r,
            Err(_) => Err(SessionError::Timeout),
        }
    }

    async fn read_status_internal(&mut self) -> Result<(), SessionError> {
        loop {
            match parse_reply(&self.read_buf) {
                Ok((SmartSocketReply::Okay, consumed)) => {
                    self.read_buf.drain(..consumed);
                    return Ok(());
                }
                Ok((SmartSocketReply::Fail(reason), consumed)) => {
                    self.read_buf.drain(..consumed);
                    return Err(SessionError::Fail(reason));
                }
                Ok((SmartSocketReply::Incomplete, _)) => {
                    let mut byte = [0u8; 1];
                    if let Err(err) = self.stream.read_exact(&mut byte).await {
                        if self.read_buf.is_empty() {
                            return Err(SessionError::ReadStatusFailed(err));
                        } else if self.read_buf.starts_with(b"FAIL") && self.read_buf.len() < 8 {
                            return Err(SessionError::ReadStatusLengthFailed(err));
                        } else if self.read_buf.starts_with(b"FAIL") {
                            return Err(SessionError::ReadStatusMessageFailed(err));
                        } else {
                            return Err(SessionError::ReadStatusFailed(err));
                        }
                    }
                    self.read_buf.push(byte[0]);
                }
                Err(SmartSocketError::InvalidStatus(status)) => {
                    self.read_buf.clear();
                    return Err(SessionError::InvalidStatus(status));
                }
                Err(SmartSocketError::InvalidHexLength(_)) => {
                    self.read_buf.clear();
                    return Err(SessionError::ReadStatusLengthFailed(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid hex length",
                    )));
                }
                Err(SmartSocketError::InvalidUtf8(err)) => {
                    self.read_buf.clear();
                    return Err(SessionError::ReadStatusMessageFailed(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        err,
                    )));
                }
                Err(SmartSocketError::RequestTooLong(_)) => unreachable!(),
            }
        }
    }

    /// Reads a 4-hex-length prefixed string response after `OKAY`, subject to handshake timeout.
    pub async fn read_string(&mut self) -> Result<String, SessionError> {
        let res =
            tokio::time::timeout(self.config.handshake_timeout, self.read_string_internal()).await;
        match res {
            Ok(r) => r,
            Err(_) => Err(SessionError::Timeout),
        }
    }

    async fn read_string_internal(&mut self) -> Result<String, SessionError> {
        loop {
            match parse_protocol_string(&self.read_buf) {
                Ok((ProtocolString::Complete(payload), consumed)) => {
                    self.read_buf.drain(..consumed);
                    return Ok(payload);
                }
                Ok((ProtocolString::Incomplete, _)) => {
                    let mut byte = [0u8; 1];
                    if let Err(err) = self.stream.read_exact(&mut byte).await {
                        if self.read_buf.len() < 4 {
                            return Err(SessionError::ReadStatusLengthFailed(err));
                        } else {
                            return Err(SessionError::ReadStatusMessageFailed(err));
                        }
                    }
                    self.read_buf.push(byte[0]);
                }
                Err(SmartSocketError::InvalidHexLength(_)) => {
                    self.read_buf.clear();
                    return Err(SessionError::ReadStatusLengthFailed(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid hex length",
                    )));
                }
                Err(SmartSocketError::InvalidUtf8(err)) => {
                    self.read_buf.clear();
                    return Err(SessionError::ReadStatusMessageFailed(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        err,
                    )));
                }
                Err(_) => unreachable!(),
            }
        }
    }

    /// Reads all remaining bytes until socket is closed by server (orderly shutdown).
    pub async fn read_to_close(&mut self) -> Result<Vec<u8>, SessionError> {
        let mut buf = std::mem::take(&mut self.read_buf);
        self.stream
            .read_to_end(&mut buf)
            .await
            .map_err(SessionError::ReadStatusFailed)?;
        Ok(buf)
    }

    /// Convenience helper executing full `_adb_connect` flow:
    /// validate service length, connect to daemon, switch transport (if non-host or force), send service, read status.
    pub async fn connect_and_switch(
        addr: &ServerAddress,
        selector: &TransportSelector,
        service: &str,
        force_switch: bool,
    ) -> Result<Self, SessionError> {
        Self::connect_and_switch_with_config(
            addr,
            selector,
            service,
            force_switch,
            SessionConfig::default(),
        )
        .await
    }

    /// Convenience helper executing full `_adb_connect` flow with specified `SessionConfig`.
    pub async fn connect_and_switch_with_config(
        addr: &ServerAddress,
        selector: &TransportSelector,
        service: &str,
        force_switch: bool,
        config: SessionConfig,
    ) -> Result<Self, SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        let mut session = Self::connect_with_config(addr, config).await?;

        if !service.starts_with("host") || force_switch {
            session.select_transport(selector).await?;
        }

        session.send_service(service).await?;
        session.read_status().await?;

        Ok(session)
    }

    /// Consumes this session and returns the underlying `TcpStream`.
    pub fn into_inner(self) -> TcpStream {
        self.stream
    }

    /// Returns a mutable reference to the underlying `TcpStream`.
    pub fn stream_mut(&mut self) -> &mut TcpStream {
        &mut self.stream
    }

    /// Returns a reference to the underlying `TcpStream`.
    pub fn stream_ref(&self) -> &TcpStream {
        &self.stream
    }
}
