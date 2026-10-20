use std::fmt;

/// Errors that can occur during smart-socket request encoding or reply parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartSocketError {
    /// Service request length exceeds 0xFFFF (65,535 bytes).
    RequestTooLong(usize),
    /// Invalid reply status header (expected `OKAY` or `FAIL`).
    InvalidStatus([u8; 4]),
    /// Invalid hexadecimal length prefix.
    InvalidHexLength(String),
    /// String payload is not valid UTF-8.
    InvalidUtf8(std::str::Utf8Error),
}

impl fmt::Display for SmartSocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SmartSocketError::RequestTooLong(len) => write!(
                f,
                "service name length ({len} bytes) exceeds maximum 0xFFFF bytes"
            ),
            SmartSocketError::InvalidStatus(status) => write!(
                f,
                "invalid smart-socket status: {:?}",
                String::from_utf8_lossy(status)
            ),
            SmartSocketError::InvalidHexLength(s) => {
                write!(f, "invalid hex length string: {s:?}")
            }
            SmartSocketError::InvalidUtf8(err) => write!(f, "invalid UTF-8 string: {err}"),
        }
    }
}

impl std::error::Error for SmartSocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SmartSocketError::InvalidUtf8(err) => Some(err),
            _ => None,
        }
    }
}

/// The result of parsing a smart-socket reply status (`OKAY` or `FAIL`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartSocketReply {
    /// Buffer is incomplete; more bytes are needed to parse the reply status and reason.
    Incomplete,
    /// Request succeeded (`OKAY`).
    Okay,
    /// Request failed (`FAIL`) with the provided error reason message.
    Fail(String),
}

/// The result of parsing a hex-length-prefixed protocol string payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolString {
    /// Buffer is incomplete; more bytes are needed to parse the length prefix and payload.
    Incomplete,
    /// Protocol string payload was successfully parsed.
    Complete(String),
}

/// Encodes a client-to-server smart-socket service request with a 4-hex-digit length prefix.
///
/// Ports `SendProtocolString` from `original/adb_io.cpp`.
///
/// # Errors
///
/// Returns [`SmartSocketError::RequestTooLong`] if `service.len()` exceeds 65,535 bytes (`0xFFFF`).
pub fn encode_request(service: &str) -> Result<Vec<u8>, SmartSocketError> {
    if service.len() > 0xffff {
        return Err(SmartSocketError::RequestTooLong(service.len()));
    }
    let mut buf = Vec::with_capacity(4 + service.len());
    let len_prefix = format!("{:04x}", service.len());
    buf.extend_from_slice(len_prefix.as_bytes());
    buf.extend_from_slice(service.as_bytes());
    Ok(buf)
}

/// Incrementally parses a smart-socket reply (`OKAY` or `FAIL`) from a byte slice.
///
/// Ports `adb_status()` from `original/client/adb_client.cpp` and `ReadProtocolString()` from `original/adb_io.cpp`.
///
/// Returns `Ok((SmartSocketReply::Incomplete, 0))` if the buffer contains insufficient data.
/// Returns `Ok((reply, consumed_bytes))` upon successfully parsing `OKAY` or `FAIL`.
///
/// # Errors
///
/// Returns [`SmartSocketError::InvalidStatus`] if the first 4 bytes are neither `OKAY` nor `FAIL`.
/// Returns [`SmartSocketError::InvalidHexLength`] if a `FAIL` status is followed by invalid hex digits.
/// Returns [`SmartSocketError::InvalidUtf8`] if the `FAIL` reason message is not valid UTF-8.
pub fn parse_reply(buf: &[u8]) -> Result<(SmartSocketReply, usize), SmartSocketError> {
    if buf.len() < 4 {
        return Ok((SmartSocketReply::Incomplete, 0));
    }

    match &buf[0..4] {
        b"OKAY" => Ok((SmartSocketReply::Okay, 4)),
        b"FAIL" => {
            if buf.len() < 8 {
                return Ok((SmartSocketReply::Incomplete, 0));
            }

            let hex_str = std::str::from_utf8(&buf[4..8]).map_err(|_| {
                SmartSocketError::InvalidHexLength(String::from_utf8_lossy(&buf[4..8]).into_owned())
            })?;

            let reason_len = usize::from_str_radix(hex_str, 16)
                .map_err(|_| SmartSocketError::InvalidHexLength(hex_str.to_string()))?;

            let total_len = 8 + reason_len;
            if buf.len() < total_len {
                return Ok((SmartSocketReply::Incomplete, 0));
            }

            let reason_bytes = &buf[8..total_len];
            let reason = std::str::from_utf8(reason_bytes)
                .map_err(SmartSocketError::InvalidUtf8)?
                .to_string();

            Ok((SmartSocketReply::Fail(reason), total_len))
        }
        status => Err(SmartSocketError::InvalidStatus([
            status[0], status[1], status[2], status[3],
        ])),
    }
}

/// Incrementally parses a 4-hex-digit length-prefixed string from a byte slice.
///
/// Ports `ReadProtocolString` from `original/adb_io.cpp`.
///
/// Returns `Ok((ProtocolString::Incomplete, 0))` if the buffer contains insufficient data.
/// Returns `Ok((ProtocolString::Complete(payload), consumed_bytes))` upon successfully reading the payload.
///
/// # Errors
///
/// Returns [`SmartSocketError::InvalidHexLength`] if the 4-byte length prefix is not valid hex digits.
/// Returns [`SmartSocketError::InvalidUtf8`] if the payload is not valid UTF-8.
pub fn parse_protocol_string(buf: &[u8]) -> Result<(ProtocolString, usize), SmartSocketError> {
    if buf.len() < 4 {
        return Ok((ProtocolString::Incomplete, 0));
    }

    let hex_str = std::str::from_utf8(&buf[0..4]).map_err(|_| {
        SmartSocketError::InvalidHexLength(String::from_utf8_lossy(&buf[0..4]).into_owned())
    })?;

    let payload_len = usize::from_str_radix(hex_str, 16)
        .map_err(|_| SmartSocketError::InvalidHexLength(hex_str.to_string()))?;

    let total_len = 4 + payload_len;
    if buf.len() < total_len {
        return Ok((ProtocolString::Incomplete, 0));
    }

    let payload_bytes = &buf[4..total_len];
    let payload = std::str::from_utf8(payload_bytes)
        .map_err(SmartSocketError::InvalidUtf8)?
        .to_string();

    Ok((ProtocolString::Complete(payload), total_len))
}
