use crate::limits::MAX_PAYLOAD;
use std::fmt;

/// Result of incremental decoding for smart-socket payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded<T> {
    /// Buffer is incomplete; more bytes are needed to parse the value.
    Incomplete,
    /// Value was successfully parsed along with the number of consumed bytes.
    Complete { value: T, consumed: usize },
}

/// The result of parsing a smart-socket reply status (`OKAY` or `FAIL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartSocketReply {
    /// Request succeeded (`OKAY`).
    Okay,
    /// Request failed (`FAIL`) with the provided error reason message.
    Fail(String),
}

/// Errors that can occur when encoding a smart-socket service request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// Service name is empty.
    Empty,
    /// Service request length exceeds allowed maximum (`MAX_PAYLOAD` = 1,048,576 bytes).
    TooLong(usize),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::Empty => write!(f, "service name cannot be empty"),
            RequestError::TooLong(len) => write!(
                f,
                "service name length ({len} bytes) exceeds maximum {MAX_PAYLOAD} bytes"
            ),
        }
    }
}

impl std::error::Error for RequestError {}

/// A validated, non-empty smart-socket service name bounded by [`MAX_PAYLOAD`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceName(String);

impl ServiceName {
    /// Creates a new `ServiceName`, verifying that it is non-empty and does not exceed [`MAX_PAYLOAD`].
    pub fn new(service: impl Into<String>) -> Result<Self, RequestError> {
        let s = service.into();
        if s.is_empty() {
            return Err(RequestError::Empty);
        }
        if s.len() > MAX_PAYLOAD {
            return Err(RequestError::TooLong(s.len()));
        }
        Ok(Self(s))
    }

    /// Returns the service name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the underlying `String`.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::ops::Deref for ServiceName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for ServiceName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<&str> for ServiceName {
    type Error = RequestError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ServiceName {
    type Error = RequestError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Errors that can occur during smart-socket reply or protocol string parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyError {
    /// Invalid reply status header (expected `OKAY` or `FAIL`).
    InvalidStatus([u8; 4]),
    /// Invalid 4-hex-digit length prefix.
    InvalidHexLength(String),
    /// String payload is not valid UTF-8.
    InvalidUtf8(std::str::Utf8Error),
}

impl fmt::Display for ReplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplyError::InvalidStatus(status) => write!(
                f,
                "invalid smart-socket status: {:?}",
                String::from_utf8_lossy(status)
            ),
            ReplyError::InvalidHexLength(s) => {
                write!(f, "invalid hex length string: {s:?}")
            }
            ReplyError::InvalidUtf8(err) => write!(f, "invalid UTF-8 string: {err}"),
        }
    }
}

impl std::error::Error for ReplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ReplyError::InvalidUtf8(err) => Some(err),
            _ => None,
        }
    }
}

/// Encodes an `OKAY` status reply without payload (`OKAY`).
///
/// Ports `SendOkay` from `original/adb_io.cpp`.
pub fn encode_okay() -> &'static [u8] {
    b"OKAY"
}

/// Encodes an `OKAY` status reply followed by a 4-hex-digit length-prefixed payload (`OKAY<len><payload>`).
///
/// Ports `SendOkay` followed by `SendProtocolString` from `original/adb_io.cpp`.
pub fn encode_okay_with_payload(payload: &str) -> Vec<u8> {
    let mut resp = Vec::with_capacity(8 + payload.len());
    resp.extend_from_slice(b"OKAY");
    let len_prefix = format!("{:04x}", payload.len());
    resp.extend_from_slice(len_prefix.as_bytes());
    resp.extend_from_slice(payload.as_bytes());
    resp
}

/// Encodes a `FAIL` status reply followed by a 4-hex-digit length-prefixed reason message (`FAIL<len><reason>`).
///
/// Ports `SendFail` from `original/adb_io.cpp`.
pub fn encode_fail(reason: &str) -> Vec<u8> {
    let mut resp = Vec::with_capacity(8 + reason.len());
    resp.extend_from_slice(b"FAIL");
    let len_prefix = format!("{:04x}", reason.len());
    resp.extend_from_slice(len_prefix.as_bytes());
    resp.extend_from_slice(reason.as_bytes());
    resp
}

/// Encodes a client-to-server smart-socket service request with a 4-hex-digit length prefix.
///
/// Ports `SendProtocolString` from `original/adb_io.cpp`.
///
/// # Errors
///
/// Returns [`RequestError`] if `service` is invalid (e.g. empty or length exceeds [`MAX_PAYLOAD`]).
pub fn encode_request(service: impl AsRef<str>) -> Result<Vec<u8>, RequestError> {
    let service = service.as_ref();
    if service.is_empty() {
        return Err(RequestError::Empty);
    }
    if service.len() > MAX_PAYLOAD {
        return Err(RequestError::TooLong(service.len()));
    }
    let mut buf = Vec::with_capacity(4 + service.len());
    let len_prefix = format!("{:04x}", service.len());
    buf.extend_from_slice(len_prefix.as_bytes());
    buf.extend_from_slice(service.as_bytes());
    Ok(buf)
}

/// Parses a 4-byte ASCII hexadecimal length prefix strictly matching C++ `unhex`.
/// Rejects leading signs (`+`, `-`) or any character outside `0-9`, `a-f`, `A-F`.
pub fn parse_hex_len4(bytes: &[u8; 4]) -> Result<usize, ReplyError> {
    let mut n = 0usize;
    for &b in bytes {
        let val = match b {
            b'0'..=b'9' => (b - b'0') as usize,
            b'a'..=b'f' => (b - b'a' + 10) as usize,
            b'A'..=b'F' => (b - b'A' + 10) as usize,
            _ => {
                return Err(ReplyError::InvalidHexLength(
                    String::from_utf8_lossy(bytes).into_owned(),
                ));
            }
        };
        n = (n << 4) | val;
    }
    Ok(n)
}

/// Incrementally parses a smart-socket reply (`OKAY` or `FAIL`) from a byte slice.
///
/// Ports `adb_status()` from `original/client/adb_client.cpp` and `ReadProtocolString()` from `original/adb_io.cpp`.
///
/// Returns `Ok(Decoded::Incomplete)` if the buffer contains insufficient data.
/// Returns `Ok(Decoded::Complete { value, consumed })` upon successfully parsing `OKAY` or `FAIL`.
///
/// # Errors
///
/// Returns [`ReplyError::InvalidStatus`] if the first 4 bytes are neither `OKAY` nor `FAIL`.
/// Returns [`ReplyError::InvalidHexLength`] if a `FAIL` status is followed by invalid hex digits.
/// Returns [`ReplyError::InvalidUtf8`] if the `FAIL` reason message is not valid UTF-8.
pub fn parse_reply(buf: &[u8]) -> Result<Decoded<SmartSocketReply>, ReplyError> {
    if buf.len() < 4 {
        return Ok(Decoded::Incomplete);
    }

    match &buf[0..4] {
        b"OKAY" => Ok(Decoded::Complete {
            value: SmartSocketReply::Okay,
            consumed: 4,
        }),
        b"FAIL" => {
            if buf.len() < 8 {
                return Ok(Decoded::Incomplete);
            }

            let hex_bytes: &[u8; 4] = buf[4..8].try_into().unwrap();
            let reason_len = parse_hex_len4(hex_bytes)?;

            let total_len = 8 + reason_len;
            if buf.len() < total_len {
                return Ok(Decoded::Incomplete);
            }

            let reason_bytes = &buf[8..total_len];
            let reason = std::str::from_utf8(reason_bytes)
                .map_err(ReplyError::InvalidUtf8)?
                .to_string();

            Ok(Decoded::Complete {
                value: SmartSocketReply::Fail(reason),
                consumed: total_len,
            })
        }
        status => Err(ReplyError::InvalidStatus([
            status[0], status[1], status[2], status[3],
        ])),
    }
}

/// Incrementally parses a 4-hex-digit length-prefixed string from a byte slice.
///
/// Ports `ReadProtocolString` from `original/adb_io.cpp`.
///
/// Returns `Ok(Decoded::Incomplete)` if the buffer contains insufficient data.
/// Returns `Ok(Decoded::Complete { value: payload, consumed })` upon successfully reading the payload.
///
/// # Errors
///
/// Returns [`ReplyError::InvalidHexLength`] if the 4-byte length prefix is not valid hex digits.
/// Returns [`ReplyError::InvalidUtf8`] if the payload is not valid UTF-8.
pub fn parse_protocol_string(buf: &[u8]) -> Result<Decoded<String>, ReplyError> {
    if buf.len() < 4 {
        return Ok(Decoded::Incomplete);
    }

    let hex_bytes: &[u8; 4] = buf[0..4].try_into().unwrap();
    let payload_len = parse_hex_len4(hex_bytes)?;

    let total_len = 4 + payload_len;
    if buf.len() < total_len {
        return Ok(Decoded::Incomplete);
    }

    let payload_bytes = &buf[4..total_len];
    let payload = std::str::from_utf8(payload_bytes)
        .map_err(ReplyError::InvalidUtf8)?
        .to_string();

    Ok(Decoded::Complete {
        value: payload,
        consumed: total_len,
    })
}
