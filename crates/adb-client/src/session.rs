use crate::server_address::ServerAddress;
use crate::transport::TransportSelector;
use adb_proto::MAX_PAYLOAD;
use std::fmt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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
}

impl Session {
    /// Creates a new `Session` wrapping an existing `TcpStream`.
    pub fn new(stream: TcpStream) -> Self {
        Self { stream }
    }

    /// Connects to the ADB server at the given address.
    ///
    /// Ports `_adb_connect` socket connection phase.
    pub async fn connect(addr: &ServerAddress) -> Result<Self, SessionError> {
        let spec = addr.to_spec();
        let host = addr.host.as_deref().unwrap_or("127.0.0.1");
        let host = if host == "localhost" {
            "127.0.0.1"
        } else {
            host
        };

        let stream = TcpStream::connect((host, addr.port)).await.map_err(|err| {
            SessionError::CannotConnect {
                spec: spec.clone(),
                cause: err.to_string(),
            }
        })?;

        Ok(Self { stream })
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
            self.stream
                .read_exact(&mut buf)
                .await
                .map_err(|_| SessionError::ReadTransportIdFailed)?;
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

    /// Reads smart-socket status response (`OKAY` or `FAIL` + length-prefixed reason).
    ///
    /// Ports `adb_status()` from C++ `adb_client.cpp`.
    pub async fn read_status(&mut self) -> Result<(), SessionError> {
        let mut status_buf = [0u8; 4];
        if let Err(err) = self.stream.read_exact(&mut status_buf).await {
            return Err(SessionError::ReadStatusFailed(err));
        }

        if &status_buf == b"OKAY" {
            return Ok(());
        }

        if &status_buf != b"FAIL" {
            return Err(SessionError::InvalidStatus(status_buf));
        }

        // FAIL case: read 4-hex length
        let mut len_buf = [0u8; 4];
        if let Err(err) = self.stream.read_exact(&mut len_buf).await {
            return Err(SessionError::ReadStatusLengthFailed(err));
        }

        let len_str = std::str::from_utf8(&len_buf).map_err(|_| {
            SessionError::ReadStatusLengthFailed(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid hex length",
            ))
        })?;

        let len = usize::from_str_radix(len_str, 16).map_err(|_| {
            SessionError::ReadStatusLengthFailed(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid hex length",
            ))
        })?;

        let mut reason_buf = vec![0u8; len];
        if len > 0 {
            if let Err(err) = self.stream.read_exact(&mut reason_buf).await {
                return Err(SessionError::ReadStatusMessageFailed(err));
            }
        }

        let reason = String::from_utf8_lossy(&reason_buf).into_owned();
        Err(SessionError::Fail(reason))
    }

    /// Reads a 4-hex-length prefixed string response after `OKAY`.
    ///
    /// Ports `ReadProtocolString` from C++ `adb_io.cpp`.
    pub async fn read_string(&mut self) -> Result<String, SessionError> {
        let mut len_buf = [0u8; 4];
        if let Err(err) = self.stream.read_exact(&mut len_buf).await {
            return Err(SessionError::ReadStatusLengthFailed(err));
        }

        let len_str = std::str::from_utf8(&len_buf).map_err(|_| {
            SessionError::ReadStatusLengthFailed(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid hex length",
            ))
        })?;

        let len = usize::from_str_radix(len_str, 16).map_err(|_| {
            SessionError::ReadStatusLengthFailed(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid hex length",
            ))
        })?;

        let mut payload_buf = vec![0u8; len];
        if len > 0 {
            if let Err(err) = self.stream.read_exact(&mut payload_buf).await {
                return Err(SessionError::ReadStatusMessageFailed(err));
            }
        }

        Ok(String::from_utf8_lossy(&payload_buf).into_owned())
    }

    /// Reads all remaining bytes until socket is closed by server (orderly shutdown).
    pub async fn read_to_close(&mut self) -> Result<Vec<u8>, SessionError> {
        let mut buf = Vec::new();
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
        Self::connect_and_switch_factory(
            addr,
            selector,
            service,
            force_switch,
            &crate::client::DefaultSessionFactory,
        )
        .await
    }

    /// Executing `connect_and_switch` using a custom `SessionFactory`.
    pub async fn connect_and_switch_factory(
        addr: &ServerAddress,
        selector: &TransportSelector,
        service: &str,
        force_switch: bool,
        factory: &dyn crate::client::SessionFactory,
    ) -> Result<Self, SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        let mut session = factory.connect(addr).await?;

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
