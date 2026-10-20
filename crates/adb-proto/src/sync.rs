use std::collections::VecDeque;
use std::fmt;

/// `ID_LSTAT_V1` constant (`MKID('S', 'T', 'A', 'T')`).
///
/// Ports `ID_LSTAT_V1` from `original/file_sync_protocol.h`.
pub const ID_LSTAT_V1: u32 = u32::from_le_bytes(*b"STAT");

/// `ID_LIST_V1` constant (`MKID('L', 'I', 'S', 'T')`).
///
/// Ports `ID_LIST_V1` from `original/file_sync_protocol.h`.
pub const ID_LIST_V1: u32 = u32::from_le_bytes(*b"LIST");

/// `ID_DENT_V1` constant (`MKID('D', 'E', 'N', 'T')`).
///
/// Ports `ID_DENT_V1` from `original/file_sync_protocol.h`.
pub const ID_DENT_V1: u32 = u32::from_le_bytes(*b"DENT");

/// `ID_SEND_V1` constant (`MKID('S', 'E', 'N', 'D')`).
///
/// Ports `ID_SEND_V1` from `original/file_sync_protocol.h`.
pub const ID_SEND_V1: u32 = u32::from_le_bytes(*b"SEND");

/// `ID_RECV_V1` constant (`MKID('R', 'E', 'C', 'V')`).
///
/// Ports `ID_RECV_V1` from `original/file_sync_protocol.h`.
pub const ID_RECV_V1: u32 = u32::from_le_bytes(*b"RECV");

/// `ID_DONE` constant (`MKID('D', 'O', 'N', 'E')`).
///
/// Ports `ID_DONE` from `original/file_sync_protocol.h`.
pub const ID_DONE: u32 = u32::from_le_bytes(*b"DONE");

/// `ID_DATA` constant (`MKID('D', 'A', 'T', 'A')`).
///
/// Ports `ID_DATA` from `original/file_sync_protocol.h`.
pub const ID_DATA: u32 = u32::from_le_bytes(*b"DATA");

/// `ID_OKAY` constant (`MKID('O', 'K', 'A', 'Y')`).
///
/// Ports `ID_OKAY` from `original/file_sync_protocol.h`.
pub const ID_OKAY: u32 = u32::from_le_bytes(*b"OKAY");

/// `ID_FAIL` constant (`MKID('F', 'A', 'I', 'L')`).
///
/// Ports `ID_FAIL` from `original/file_sync_protocol.h`.
pub const ID_FAIL: u32 = u32::from_le_bytes(*b"FAIL");

/// `ID_QUIT` constant (`MKID('Q', 'U', 'I', 'T')`).
///
/// Ports `ID_QUIT` from `original/file_sync_protocol.h`.
pub const ID_QUIT: u32 = u32::from_le_bytes(*b"QUIT");

/// Maximum size of a sync data chunk in bytes (64 KiB).
///
/// Ports `SYNC_DATA_MAX` from `original/file_sync_protocol.h`.
pub const SYNC_DATA_MAX: usize = 64 * 1024;

/// Maximum path length for sync request messages.
///
/// Ports path length check in `original/daemon/file_sync_service.cpp`.
pub const SYNC_PATH_MAX: usize = 1024;

/// Errors that can occur when encoding Sync v1 messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncEncodeError {
    /// Payload exceeds maximum allowed length.
    PayloadTooLarge { length: usize, max: usize },
}

impl fmt::Display for SyncEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncEncodeError::PayloadTooLarge { length, max } => {
                write!(f, "sync payload too large: {length} bytes (max: {max})")
            }
        }
    }
}

impl std::error::Error for SyncEncodeError {}

/// Errors that can occur when decoding Sync v1 messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncDecodeError {
    /// Unknown or unhandled sync ID code.
    UnknownId(u32),
    /// Declared payload length exceeds limit (e.g. `SYNC_DATA_MAX` or `SYNC_PATH_MAX`).
    OversizedLength { length: usize, max: usize },
    /// Remote end returned a `FAIL` message.
    FailMessage(String),
    /// String field is not valid UTF-8.
    InvalidUtf8(std::string::FromUtf8Error),
    /// Payload structure is invalid (e.g. missing comma or bad mode in `SEND` spec).
    InvalidPayload(String),
}

impl fmt::Display for SyncDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncDecodeError::UnknownId(id) => write!(
                f,
                "unknown sync packet id: 0x{id:08x} ({:?})",
                String::from_utf8_lossy(&id.to_le_bytes())
            ),
            SyncDecodeError::OversizedLength { length, max } => {
                write!(f, "sync length exceeds max: {length} (max: {max})")
            }
            SyncDecodeError::FailMessage(msg) => write!(f, "sync fail message: {msg}"),
            SyncDecodeError::InvalidUtf8(e) => write!(f, "invalid utf-8 in sync message: {e}"),
            SyncDecodeError::InvalidPayload(msg) => write!(f, "invalid sync payload: {msg}"),
        }
    }
}

impl std::error::Error for SyncDecodeError {}

impl From<std::string::FromUtf8Error> for SyncDecodeError {
    fn from(err: std::string::FromUtf8Error) -> Self {
        SyncDecodeError::InvalidUtf8(err)
    }
}

/// Operating mode for [`SyncDecoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncDecoderMode {
    /// Decode `STAT` as server response (`sync_stat_v1`, 16 bytes).
    #[default]
    Response,
    /// Decode `STAT` as client request (`SyncRequest`, 8 bytes + path).
    Request,
}

/// Represents a Sync v1 protocol message.
///
/// Ports layouts from `original/file_sync_protocol.h` (`SyncRequest`, `sync_stat_v1`, `sync_dent_v1`, `sync_data`, `sync_status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncMessage {
    /// List directory contents request (`ID_LIST_V1`).
    List { path: String },
    /// Directory entry response (`ID_DENT_V1`).
    Dent {
        mode: u32,
        size: u32,
        mtime: u32,
        name: String,
    },
    /// File stat request (`ID_LSTAT_V1`).
    Stat { path: String },
    /// File stat response (`sync_stat_v1`, `ID_LSTAT_V1`).
    StatReply { mode: u32, size: u32, mtime: u32 },
    /// Send file request (`ID_SEND_V1`), with path and mode separated by comma.
    Send { path: String, mode: u32 },
    /// Receive file request (`ID_RECV_V1`).
    Recv { path: String },
    /// File transfer data chunk (`ID_DATA`).
    Data { data: Vec<u8> },
    /// Transfer completion marker (`ID_DONE`), carrying `mtime` on SEND.
    Done { mtime: u32 },
    /// Operation succeeded response (`ID_OKAY`).
    Okay,
    /// Operation failed response (`ID_FAIL`).
    Fail { message: String },
    /// Exit sync mode request (`ID_QUIT`).
    Quit,
}

impl SyncMessage {
    /// Encodes the message into a byte vector matching `file_sync_protocol.h` wire layouts.
    pub fn encode_to_vec(&self) -> Vec<u8> {
        match self {
            SyncMessage::List { path } => encode_req(ID_LIST_V1, path.as_bytes()),
            SyncMessage::Dent {
                mode,
                size,
                mtime,
                name,
            } => {
                let name_bytes = name.as_bytes();
                let mut buf = Vec::with_capacity(20 + name_bytes.len());
                buf.extend_from_slice(&ID_DENT_V1.to_le_bytes());
                buf.extend_from_slice(&mode.to_le_bytes());
                buf.extend_from_slice(&size.to_le_bytes());
                buf.extend_from_slice(&mtime.to_le_bytes());
                buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(name_bytes);
                buf
            }
            SyncMessage::Stat { path } => encode_req(ID_LSTAT_V1, path.as_bytes()),
            SyncMessage::StatReply { mode, size, mtime } => {
                let mut buf = Vec::with_capacity(16);
                buf.extend_from_slice(&ID_LSTAT_V1.to_le_bytes());
                buf.extend_from_slice(&mode.to_le_bytes());
                buf.extend_from_slice(&size.to_le_bytes());
                buf.extend_from_slice(&mtime.to_le_bytes());
                buf
            }
            SyncMessage::Send { path, mode } => {
                let spec = format!("{path},{mode}");
                encode_req(ID_SEND_V1, spec.as_bytes())
            }
            SyncMessage::Recv { path } => encode_req(ID_RECV_V1, path.as_bytes()),
            SyncMessage::Data { data } => {
                let mut buf = Vec::with_capacity(8 + data.len());
                buf.extend_from_slice(&ID_DATA.to_le_bytes());
                buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
                buf.extend_from_slice(data);
                buf
            }
            SyncMessage::Done { mtime } => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&ID_DONE.to_le_bytes());
                buf.extend_from_slice(&mtime.to_le_bytes());
                buf
            }
            SyncMessage::Okay => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&ID_OKAY.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf
            }
            SyncMessage::Fail { message } => {
                let msg_bytes = message.as_bytes();
                let mut buf = Vec::with_capacity(8 + msg_bytes.len());
                buf.extend_from_slice(&ID_FAIL.to_le_bytes());
                buf.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(msg_bytes);
                buf
            }
            SyncMessage::Quit => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&ID_QUIT.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf
            }
        }
    }

    /// Encodes the message to a byte vector, verifying payload limits.
    pub fn encode_to_vec_checked(&self) -> Result<Vec<u8>, SyncEncodeError> {
        if let SyncMessage::Data { data } = self {
            if data.len() > SYNC_DATA_MAX {
                return Err(SyncEncodeError::PayloadTooLarge {
                    length: data.len(),
                    max: SYNC_DATA_MAX,
                });
            }
        }
        Ok(self.encode_to_vec())
    }
}

fn encode_req(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Incremental decoder for Sync v1 messages streaming over a socket.
#[derive(Debug)]
pub struct SyncDecoder {
    buffer: VecDeque<u8>,
    mode: SyncDecoderMode,
}

impl SyncDecoder {
    /// Constructs a new [`SyncDecoder`] with the specified [`SyncDecoderMode`].
    pub fn new(mode: SyncDecoderMode) -> Self {
        Self {
            buffer: VecDeque::new(),
            mode,
        }
    }

    /// Sets the operating mode of the decoder.
    pub fn set_mode(&mut self, mode: SyncDecoderMode) {
        self.mode = mode;
    }

    /// Returns the current operating mode.
    pub fn mode(&self) -> SyncDecoderMode {
        self.mode
    }

    /// Feeds incoming byte data into the decoder buffer.
    pub fn feed(&mut self, chunk: &[u8]) {
        self.buffer.extend(chunk);
    }

    /// Clears the decoder buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// Attempts to parse the next complete [`SyncMessage`].
    ///
    /// Returns `Ok(Some(msg))` on successful decoding.
    /// Returns `Ok(None)` if more bytes are required.
    /// Returns `Err(SyncDecodeError)` on unknown packet IDs, oversized payloads, invalid UTF-8, or `FAIL` messages.
    pub fn pop_message(&mut self) -> Result<Option<SyncMessage>, SyncDecodeError> {
        if self.buffer.len() < 8 {
            return Ok(None);
        }

        let mut id_bytes = [0u8; 4];
        let mut param_bytes = [0u8; 4];

        for (i, b) in self.buffer.iter().take(4).enumerate() {
            id_bytes[i] = *b;
        }
        for (i, b) in self.buffer.iter().skip(4).take(4).enumerate() {
            param_bytes[i] = *b;
        }

        let id = u32::from_le_bytes(id_bytes);
        let param = u32::from_le_bytes(param_bytes);

        match id {
            ID_LIST_V1 => {
                let path_len = param as usize;
                if path_len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: path_len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + path_len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let payload = self.pop_bytes(path_len);
                let path = String::from_utf8(payload)?;
                Ok(Some(SyncMessage::List { path }))
            }
            ID_DENT_V1 => {
                if self.buffer.len() < 20 {
                    return Ok(None);
                }
                let mut mode_b = [0u8; 4];
                let mut size_b = [0u8; 4];
                let mut mtime_b = [0u8; 4];
                let mut namelen_b = [0u8; 4];

                for (i, b) in self.buffer.iter().skip(4).take(4).enumerate() {
                    mode_b[i] = *b;
                }
                for (i, b) in self.buffer.iter().skip(8).take(4).enumerate() {
                    size_b[i] = *b;
                }
                for (i, b) in self.buffer.iter().skip(12).take(4).enumerate() {
                    mtime_b[i] = *b;
                }
                for (i, b) in self.buffer.iter().skip(16).take(4).enumerate() {
                    namelen_b[i] = *b;
                }

                let namelen = u32::from_le_bytes(namelen_b) as usize;
                if namelen > SYNC_PATH_MAX {
                    self.buffer.drain(..20);
                    return Err(SyncDecodeError::OversizedLength {
                        length: namelen,
                        max: SYNC_PATH_MAX,
                    });
                }

                if self.buffer.len() < 20 + namelen {
                    return Ok(None);
                }

                let mode = u32::from_le_bytes(mode_b);
                let size = u32::from_le_bytes(size_b);
                let mtime = u32::from_le_bytes(mtime_b);

                self.buffer.drain(..20);
                let name_bytes = self.pop_bytes(namelen);
                let name = String::from_utf8(name_bytes)?;

                Ok(Some(SyncMessage::Dent {
                    mode,
                    size,
                    mtime,
                    name,
                }))
            }
            ID_LSTAT_V1 => match self.mode {
                SyncDecoderMode::Request => {
                    let path_len = param as usize;
                    if path_len > SYNC_PATH_MAX {
                        self.buffer.drain(..8);
                        return Err(SyncDecodeError::OversizedLength {
                            length: path_len,
                            max: SYNC_PATH_MAX,
                        });
                    }
                    if self.buffer.len() < 8 + path_len {
                        return Ok(None);
                    }
                    self.buffer.drain(..8);
                    let payload = self.pop_bytes(path_len);
                    let path = String::from_utf8(payload)?;
                    Ok(Some(SyncMessage::Stat { path }))
                }
                SyncDecoderMode::Response => {
                    if self.buffer.len() < 16 {
                        return Ok(None);
                    }
                    let mut mode_b = [0u8; 4];
                    let mut size_b = [0u8; 4];
                    let mut mtime_b = [0u8; 4];

                    for (i, b) in self.buffer.iter().skip(4).take(4).enumerate() {
                        mode_b[i] = *b;
                    }
                    for (i, b) in self.buffer.iter().skip(8).take(4).enumerate() {
                        size_b[i] = *b;
                    }
                    for (i, b) in self.buffer.iter().skip(12).take(4).enumerate() {
                        mtime_b[i] = *b;
                    }

                    self.buffer.drain(..16);
                    Ok(Some(SyncMessage::StatReply {
                        mode: u32::from_le_bytes(mode_b),
                        size: u32::from_le_bytes(size_b),
                        mtime: u32::from_le_bytes(mtime_b),
                    }))
                }
            },
            ID_SEND_V1 => {
                let spec_len = param as usize;
                if spec_len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: spec_len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + spec_len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let payload = self.pop_bytes(spec_len);
                let spec = String::from_utf8(payload)?;

                let (path, mode_str) = spec.rsplit_once(',').ok_or_else(|| {
                    SyncDecodeError::InvalidPayload("missing comma in SEND spec".to_string())
                })?;

                let mode = parse_mode(mode_str)?;
                Ok(Some(SyncMessage::Send {
                    path: path.to_string(),
                    mode,
                }))
            }
            ID_RECV_V1 => {
                let path_len = param as usize;
                if path_len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: path_len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + path_len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let payload = self.pop_bytes(path_len);
                let path = String::from_utf8(payload)?;
                Ok(Some(SyncMessage::Recv { path }))
            }
            ID_DATA => {
                let size = param as usize;
                if size > SYNC_DATA_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: size,
                        max: SYNC_DATA_MAX,
                    });
                }
                if self.buffer.len() < 8 + size {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let data = self.pop_bytes(size);
                Ok(Some(SyncMessage::Data { data }))
            }
            ID_DONE => {
                self.buffer.drain(..8);
                Ok(Some(SyncMessage::Done { mtime: param }))
            }
            ID_OKAY => {
                self.buffer.drain(..8);
                Ok(Some(SyncMessage::Okay))
            }
            ID_FAIL => {
                let msglen = param as usize;
                if msglen > SYNC_DATA_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: msglen,
                        max: SYNC_DATA_MAX,
                    });
                }
                if self.buffer.len() < 8 + msglen {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let payload = self.pop_bytes(msglen);
                let message = String::from_utf8(payload)?;
                Err(SyncDecodeError::FailMessage(message))
            }
            ID_QUIT => {
                self.buffer.drain(..8);
                Ok(Some(SyncMessage::Quit))
            }
            unknown => {
                self.buffer.drain(..4);
                Err(SyncDecodeError::UnknownId(unknown))
            }
        }
    }

    fn pop_bytes(&mut self, len: usize) -> Vec<u8> {
        let mut vec = vec![0u8; len];
        for b in vec.iter_mut() {
            *b = self.buffer.pop_front().unwrap();
        }
        vec
    }
}

fn parse_mode(s: &str) -> Result<u32, SyncDecodeError> {
    let res = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else if s.len() > 1 && s.starts_with('0') {
        u32::from_str_radix(&s[1..], 8)
    } else {
        s.parse::<u32>()
    };

    res.map_err(|e| {
        SyncDecodeError::InvalidPayload(format!("invalid mode integer string '{s}': {e}"))
    })
}
