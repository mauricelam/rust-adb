//! Client session management and typestate machine.

use crate::server_address::ServerAddress;
use crate::transport::TransportSelector;
use adb_proto::smart_socket::{
    parse_protocol_string, parse_reply, ProtocolString, SmartSocketError, SmartSocketReply,
};
use adb_proto::TransportId;
use adb_proto::MAX_PAYLOAD;
use std::fmt;
use std::marker::PhantomData;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// Trait representing an asynchronous duplex byte stream suitable for ADB client sessions.
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + fmt::Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + fmt::Debug> AsyncStream for T {}

/// Boxed abstract stream type for mockable sessions.
pub type BoxedStream = Box<dyn AsyncStream>;

/// Typestate marker: initial connected state before service or transport is selected.
#[derive(Debug)]
pub struct Fresh;

/// Typestate marker: transport has been selected for a target device.
#[derive(Debug)]
pub struct TransportSelected;

/// Typestate marker: service request was transmitted, awaiting OKAY or FAIL status.
#[derive(Debug)]
pub struct AwaitingStatus;

/// Typestate marker: OKAY status received; session is ready for data exchange or stream consumption.
#[derive(Debug)]
pub struct Ready;

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

/// Represents an active smart-socket session with an ADB server with state machine typestate.
#[derive(Debug)]
pub struct Session<S = BoxedStream, State = Fresh> {
    stream: S,
    read_buf: Vec<u8>,
    config: SessionConfig,
    _state: PhantomData<State>,
}

impl<S, State> Session<S, State> {
    fn transition<NewState>(self) -> Session<S, NewState> {
        Session {
            stream: self.stream,
            read_buf: self.read_buf,
            config: self.config,
            _state: PhantomData,
        }
    }

    /// Maps the underlying stream to another stream type (e.g., boxing it).
    pub fn map_stream<T, F>(self, f: F) -> Session<T, State>
    where
        F: FnOnce(S) -> T,
    {
        Session {
            stream: f(self.stream),
            read_buf: self.read_buf,
            config: self.config,
            _state: PhantomData,
        }
    }

    /// Boxes the underlying stream into `BoxedStream`.
    pub fn boxed(self) -> Session<BoxedStream, State>
    where
        S: AsyncStream + 'static,
    {
        self.map_stream(|s| Box::new(s) as BoxedStream)
    }

    /// Consumes the session and yields the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S: AsyncStream> Session<S, Fresh> {
    /// Creates a new `Session` wrapping an existing stream in `Fresh` state with default configuration.
    pub fn new(stream: S) -> Self {
        Self::new_with_config(stream, SessionConfig::default())
    }

    /// Creates a new `Session` wrapping an existing stream in `Fresh` state with specific configuration.
    pub fn new_with_config(stream: S, config: SessionConfig) -> Self {
        Self {
            stream,
            read_buf: Vec::new(),
            config,
            _state: PhantomData,
        }
    }

    /// Selects a device transport on this connection.
    ///
    /// Ports `switch_socket_transport` from C++ `adb_client.cpp`.
    /// Consumes the session in `Fresh` state and yields `Session<S, TransportSelected>`.
    pub async fn select_transport(
        mut self,
        selector: &TransportSelector,
    ) -> Result<(Session<S, TransportSelected>, Option<TransportId>), SessionError> {
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

        let session_awaiting = self.transition::<AwaitingStatus>();
        let mut session_ready = session_awaiting.read_status().await?;

        let tid = if read_transport {
            let mut buf = [0u8; 8];
            let mut read_bytes = 0;
            while read_bytes < 8 {
                if !session_ready.read_buf.is_empty() {
                    let avail = (8 - read_bytes).min(session_ready.read_buf.len());
                    buf[read_bytes..read_bytes + avail]
                        .copy_from_slice(&session_ready.read_buf[..avail]);
                    session_ready.read_buf.drain(..avail);
                    read_bytes += avail;
                } else {
                    session_ready
                        .stream
                        .read_exact(&mut buf[read_bytes..])
                        .await
                        .map_err(|_| SessionError::ReadTransportIdFailed)?;
                    read_bytes = 8;
                }
            }
            let tid = TransportId::from_ne_bytes(buf).ok_or(SessionError::ReadTransportIdFailed)?;
            Some(tid)
        } else if let TransportSelector::Id(id) = selector {
            Some(*id)
        } else {
            None
        };

        Ok((session_ready.transition::<TransportSelected>(), tid))
    }

    /// Sends a service name directly from `Fresh` state (e.g. for `host:` services).
    pub async fn send_service(
        mut self,
        service: &str,
    ) -> Result<Session<S, AwaitingStatus>, SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        crate::adb_trace!(
            crate::trace::TraceCategory::Services,
            "sending service request: {service}"
        );

        let req_bytes = adb_proto::smart_socket::encode_request(service)
            .map_err(|_| SessionError::BadServiceLength(service.len()))?;

        self.stream
            .write_all(&req_bytes)
            .await
            .map_err(SessionError::WriteFailure)?;

        Ok(self.transition::<AwaitingStatus>())
    }
}

impl<S: AsyncStream> Session<S, TransportSelected> {
    /// Sends a service request to the selected device transport.
    pub async fn send_service(
        mut self,
        service: &str,
    ) -> Result<Session<S, AwaitingStatus>, SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        crate::adb_trace!(
            crate::trace::TraceCategory::Services,
            "sending service request to selected transport: {service}"
        );

        let req_bytes = adb_proto::smart_socket::encode_request(service)
            .map_err(|_| SessionError::BadServiceLength(service.len()))?;

        self.stream
            .write_all(&req_bytes)
            .await
            .map_err(SessionError::WriteFailure)?;

        Ok(self.transition::<AwaitingStatus>())
    }
}

impl<S: AsyncStream> Session<S, AwaitingStatus> {
    /// Reads smart-socket status response (`OKAY` or `FAIL` + length-prefixed reason), subject to handshake timeout.
    pub async fn read_status(mut self) -> Result<Session<S, Ready>, SessionError> {
        let timeout_res =
            tokio::time::timeout(self.config.handshake_timeout, self.read_status_internal()).await;
        match timeout_res {
            Ok(Ok(())) => Ok(self.transition::<Ready>()),
            Ok(Err(err)) => Err(err),
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
}

impl<S: AsyncStream> Session<S, Ready> {
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

    /// Reads a second status response (e.g. for forward commands after transport selection).
    pub async fn read_second_status(self) -> Result<Session<S, Ready>, SessionError> {
        let awaiting = self.transition::<AwaitingStatus>();
        awaiting.read_status().await
    }

    /// Reads all remaining bytes until socket is closed by server (orderly shutdown).
    pub async fn read_to_close(mut self) -> Result<Vec<u8>, SessionError> {
        let mut buf = std::mem::take(&mut self.read_buf);
        self.stream
            .read_to_end(&mut buf)
            .await
            .map_err(SessionError::ReadStatusFailed)?;
        Ok(buf)
    }

    /// Consumes the session and yields the underlying stream.
    ///
    /// Available only in the `Ready` typestate.
    pub fn into_stream(self) -> S {
        self.stream
    }
}

impl Session<TcpStream, Fresh> {
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

        crate::adb_trace!(
            crate::trace::TraceCategory::Sockets,
            "connecting to daemon at {spec}"
        );

        let mut last_err = None;
        for i in 0..5 {
            let conn_res = tokio::time::timeout(
                config.connect_timeout,
                TcpStream::connect((host, addr.port)),
            )
            .await;

            match conn_res {
                Ok(Ok(stream)) => {
                    return Ok(Self {
                        stream,
                        read_buf: Vec::new(),
                        config,
                        _state: PhantomData,
                    });
                }
                Ok(Err(err)) => {
                    let retryable = matches!(
                        err.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionRefused
                            | std::io::ErrorKind::ConnectionAborted
                    );
                    last_err = Some(SessionError::CannotConnect {
                        spec: spec.clone(),
                        cause: err.to_string(),
                    });
                    if retryable && i < 4 {
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    } else {
                        break;
                    }
                }
                Err(_) => {
                    return Err(SessionError::Timeout);
                }
            }
        }

        Err(last_err.unwrap_or(SessionError::CannotConnect {
            spec,
            cause: "connection failed".to_string(),
        }))
    }

    /// Convenience helper executing full `_adb_connect` flow:
    /// validate service length, connect to daemon, switch transport (if non-host or force), send service, read status.
    pub async fn connect_and_switch(
        addr: &ServerAddress,
        selector: &TransportSelector,
        service: &str,
        force_switch: bool,
    ) -> Result<Session<TcpStream, Ready>, SessionError> {
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
    ) -> Result<Session<TcpStream, Ready>, SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        let session = Self::connect_with_config(addr, config).await?;

        if !service.starts_with("host") || force_switch {
            let (session, _) = session.select_transport(selector).await?;
            let awaiting = session.send_service(service).await?;
            awaiting.read_status().await
        } else {
            let awaiting = session.send_service(service).await?;
            awaiting.read_status().await
        }
    }
}

impl Session<BoxedStream, Fresh> {
    /// Opens an end-to-end ready session using any `SessionFactory` and `TransportSelector`.
    pub async fn open(
        factory: &dyn crate::client::SessionFactory,
        addr: &ServerAddress,
        selector: &TransportSelector,
        service: &str,
    ) -> Result<Session<BoxedStream, Ready>, SessionError> {
        if service.is_empty() || service.len() > MAX_PAYLOAD {
            return Err(SessionError::BadServiceLength(service.len()));
        }

        let session = factory.connect(addr).await?;
        if !service.starts_with("host") {
            let (session, _) = session.select_transport(selector).await?;
            let awaiting = session.send_service(service).await?;
            awaiting.read_status().await
        } else {
            let awaiting = session.send_service(service).await?;
            awaiting.read_status().await
        }
    }
}
