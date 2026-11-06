use std::collections::VecDeque;
use std::fmt;

/// Maximum size of a sync data chunk in bytes (64 KiB).
///
/// Ports `SYNC_DATA_MAX` from `original/file_sync_protocol.h`.
pub const SYNC_DATA_MAX: usize = 64 * 1024;

/// Maximum path length for sync request messages.
///
/// Ports path length check in `original/daemon/file_sync_service.cpp`.
pub const SYNC_PATH_MAX: usize = 1024;

/// Maximum filename length for directory entries (255 bytes).
pub const SYNC_NAME_MAX: usize = 255;

/// Typed identifier enum for sync wire protocol message headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SyncId {
    LstatV1 = u32::from_le_bytes(*b"STAT"),
    StatV2 = u32::from_le_bytes(*b"STA2"),
    LstatV2 = u32::from_le_bytes(*b"LST2"),
    ListV1 = u32::from_le_bytes(*b"LIST"),
    ListV2 = u32::from_le_bytes(*b"LIS2"),
    DentV1 = u32::from_le_bytes(*b"DENT"),
    DentV2 = u32::from_le_bytes(*b"DNT2"),
    SendV1 = u32::from_le_bytes(*b"SEND"),
    SendV2 = u32::from_le_bytes(*b"SND2"),
    RecvV1 = u32::from_le_bytes(*b"RECV"),
    RecvV2 = u32::from_le_bytes(*b"RCV2"),
    Done = u32::from_le_bytes(*b"DONE"),
    Data = u32::from_le_bytes(*b"DATA"),
    Okay = u32::from_le_bytes(*b"OKAY"),
    Fail = u32::from_le_bytes(*b"FAIL"),
    Quit = u32::from_le_bytes(*b"QUIT"),
}

impl SyncId {
    /// Returns the raw u32 wire value in host endianness.
    pub const fn to_u32(self) -> u32 {
        self as u32
    }

    /// Returns the 4-byte little-endian wire representation.
    pub const fn to_le_bytes(self) -> [u8; 4] {
        (self as u32).to_le_bytes()
    }
}

impl TryFrom<u32> for SyncId {
    type Error = u32;

    fn try_from(val: u32) -> Result<Self, Self::Error> {
        match &val.to_le_bytes() {
            b"STAT" => Ok(SyncId::LstatV1),
            b"STA2" => Ok(SyncId::StatV2),
            b"LST2" => Ok(SyncId::LstatV2),
            b"LIST" => Ok(SyncId::ListV1),
            b"LIS2" => Ok(SyncId::ListV2),
            b"DENT" => Ok(SyncId::DentV1),
            b"DNT2" => Ok(SyncId::DentV2),
            b"SEND" => Ok(SyncId::SendV1),
            b"SND2" => Ok(SyncId::SendV2),
            b"RECV" => Ok(SyncId::RecvV1),
            b"RCV2" => Ok(SyncId::RecvV2),
            b"DONE" => Ok(SyncId::Done),
            b"DATA" => Ok(SyncId::Data),
            b"OKAY" => Ok(SyncId::Okay),
            b"FAIL" => Ok(SyncId::Fail),
            b"QUIT" => Ok(SyncId::Quit),
            _ => Err(val),
        }
    }
}

pub const ID_LSTAT_V1: u32 = SyncId::LstatV1.to_u32();
pub const ID_STAT_V2: u32 = SyncId::StatV2.to_u32();
pub const ID_LSTAT_V2: u32 = SyncId::LstatV2.to_u32();
pub const ID_LIST_V1: u32 = SyncId::ListV1.to_u32();
pub const ID_LIST_V2: u32 = SyncId::ListV2.to_u32();
pub const ID_DENT_V1: u32 = SyncId::DentV1.to_u32();
pub const ID_DENT_V2: u32 = SyncId::DentV2.to_u32();
pub const ID_SEND_V1: u32 = SyncId::SendV1.to_u32();
pub const ID_SEND_V2: u32 = SyncId::SendV2.to_u32();
pub const ID_RECV_V1: u32 = SyncId::RecvV1.to_u32();
pub const ID_RECV_V2: u32 = SyncId::RecvV2.to_u32();
pub const ID_DONE: u32 = SyncId::Done.to_u32();
pub const ID_DATA: u32 = SyncId::Data.to_u32();
pub const ID_OKAY: u32 = SyncId::Okay.to_u32();
pub const ID_FAIL: u32 = SyncId::Fail.to_u32();
pub const ID_QUIT: u32 = SyncId::Quit.to_u32();

/// Bounded remote path string/bytes up to [`SYNC_PATH_MAX`] bytes.
/// Preserves arbitrary non-UTF-8 bytes as supported by Android and C++ ADB.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemotePath(Vec<u8>);

impl RemotePath {
    /// Creates a new `RemotePath`, verifying length does not exceed [`SYNC_PATH_MAX`].
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, SyncEncodeError> {
        let b = bytes.into();
        if b.len() > SYNC_PATH_MAX {
            return Err(SyncEncodeError::PayloadTooLarge {
                length: b.len(),
                max: SYNC_PATH_MAX,
            });
        }
        Ok(Self(b))
    }

    /// Returns byte slice representation.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes into inner byte vector.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Returns lossy UTF-8 string representation.
    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
}

impl std::ops::Deref for RemotePath {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for RemotePath {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<&str> for RemotePath {
    type Error = SyncEncodeError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s.as_bytes())
    }
}

impl TryFrom<String> for RemotePath {
    type Error = SyncEncodeError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s.into_bytes())
    }
}

/// Bounded data chunk up to [`SYNC_DATA_MAX`] bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataChunk(Vec<u8>);

impl DataChunk {
    /// Creates a new `DataChunk`, verifying length does not exceed [`SYNC_DATA_MAX`].
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, SyncEncodeError> {
        let b = bytes.into();
        if b.len() > SYNC_DATA_MAX {
            return Err(SyncEncodeError::PayloadTooLarge {
                length: b.len(),
                max: SYNC_DATA_MAX,
            });
        }
        Ok(Self(b))
    }

    /// Returns byte slice of the chunk.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes into inner byte vector.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl std::ops::Deref for DataChunk {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<[u8]> for DataChunk {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Classification of file types represented in POSIX modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    CharacterDevice,
    BlockDevice,
    Fifo,
    Socket,
    Other(u32),
}

/// Strongly typed POSIX file mode newtype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileMode(pub u32);

impl FileMode {
    pub const S_IFMT: u32 = 0o170000;
    pub const S_IFDIR: u32 = 0o040000;
    pub const S_IFCHR: u32 = 0o020000;
    pub const S_IFBLK: u32 = 0o060000;
    pub const S_IFREG: u32 = 0o100000;
    pub const S_IFLNK: u32 = 0o120000;
    pub const S_IFIFO: u32 = 0o010000;
    pub const S_IFSOCK: u32 = 0o140000;

    /// Creates a new `FileMode`.
    pub const fn new(mode: u32) -> Self {
        Self(mode)
    }

    /// Returns the raw mode bits.
    pub const fn mode(self) -> u32 {
        self.0
    }

    /// Returns the categorized file type.
    pub fn file_type(self) -> FileType {
        match self.0 & Self::S_IFMT {
            Self::S_IFREG => FileType::Regular,
            Self::S_IFDIR => FileType::Directory,
            Self::S_IFLNK => FileType::Symlink,
            Self::S_IFCHR => FileType::CharacterDevice,
            Self::S_IFBLK => FileType::BlockDevice,
            Self::S_IFIFO => FileType::Fifo,
            Self::S_IFSOCK => FileType::Socket,
            other => FileType::Other(other),
        }
    }

    pub fn is_dir(self) -> bool {
        self.file_type() == FileType::Directory
    }

    pub fn is_file(self) -> bool {
        self.file_type() == FileType::Regular
    }

    pub fn is_symlink(self) -> bool {
        self.file_type() == FileType::Symlink
    }
}

/// Strongly typed Unix timestamp (seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct UnixTime(pub u64);

impl UnixTime {
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    pub const fn as_secs(self) -> u64 {
        self.0
    }

    pub const fn as_u32(self) -> u32 {
        self.0 as u32
    }

    pub const fn as_i64(self) -> i64 {
        self.0 as i64
    }
}

/// Send transfer flags for Sync v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SendFlags(pub u32);

impl SendFlags {
    pub const NONE: Self = Self(0);
    pub const BROTLI: Self = Self(1 << 0);
    pub const LZ4: Self = Self(1 << 1);
    pub const ZSTD: Self = Self(1 << 2);
    pub const DRY_RUN: Self = Self(1 << 3);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(&self) -> u32 {
        self.0
    }

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Recv transfer flags for Sync v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecvFlags(pub u32);

impl RecvFlags {
    pub const NONE: Self = Self(0);
    pub const BROTLI: Self = Self(1 << 0);
    pub const LZ4: Self = Self(1 << 1);
    pub const ZSTD: Self = Self(1 << 2);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(&self) -> u32 {
        self.0
    }

    pub fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Wire structure for `sync_stat_v1` (16 bytes packed).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncStatV1 {
    pub id: u32,
    pub mode: u32,
    pub size: u32,
    pub mtime: u32,
}

/// Wire structure for `sync_stat_v2` (72 bytes packed).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncStatV2 {
    pub id: u32,
    pub error: u32,
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
}

/// Wire structure for `sync_dent_v1` (20 bytes packed header, followed by `namelen` bytes of name).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncDentV1 {
    pub id: u32,
    pub mode: u32,
    pub size: u32,
    pub mtime: u32,
    pub namelen: u32,
}

/// Wire structure for `sync_dent_v2` (76 bytes packed header, followed by `namelen` bytes of name).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncDentV2 {
    pub id: u32,
    pub error: u32,
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub namelen: u32,
}

/// Wire structure for `sync_send_v2` (12 bytes packed).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncSendV2 {
    pub id: u32,
    pub mode: u32,
    pub flags: u32,
}

/// Wire structure for `sync_recv_v2` (8 bytes packed).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncRecvV2 {
    pub id: u32,
    pub flags: u32,
}

/// Errors that can occur when encoding Sync messages.
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

/// Errors that can occur when decoding Sync messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncDecodeError {
    /// Unknown or unhandled sync ID code.
    UnknownId(u32),
    /// Declared payload length exceeds limit (e.g. `SYNC_DATA_MAX` or `SYNC_PATH_MAX`).
    OversizedLength { length: usize, max: usize },
    /// Remote end returned a `FAIL` message.
    FailMessage(String),
    /// Payload structure is invalid.
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
            SyncDecodeError::InvalidPayload(msg) => write!(f, "invalid sync payload: {msg}"),
        }
    }
}

impl std::error::Error for SyncDecodeError {}

/// Strongly typed client-to-server Sync requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncRequest {
    List(RemotePath),
    Stat(RemotePath),
    Send { path: RemotePath, mode: FileMode },
    Recv(RemotePath),
    Data(DataChunk),
    Done { mtime: UnixTime },
    Quit,
}

impl SyncRequest {
    /// Encodes request into bytes. Infallible because bounded types guarantee size limits.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            SyncRequest::List(path) => encode_id_payload(SyncId::ListV1, path.as_bytes()),
            SyncRequest::Stat(path) => encode_id_payload(SyncId::LstatV1, path.as_bytes()),
            SyncRequest::Send { path, mode } => {
                let spec = format!("{},{}", path.to_string_lossy(), mode.mode());
                encode_id_payload(SyncId::SendV1, spec.as_bytes())
            }
            SyncRequest::Recv(path) => encode_id_payload(SyncId::RecvV1, path.as_bytes()),
            SyncRequest::Data(chunk) => encode_id_payload(SyncId::Data, chunk.as_bytes()),
            SyncRequest::Done { mtime } => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&SyncId::Done.to_le_bytes());
                buf.extend_from_slice(&mtime.as_u32().to_le_bytes());
                buf
            }
            SyncRequest::Quit => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&SyncId::Quit.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf
            }
        }
    }
}

/// Strongly typed server-to-client Sync responses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncResponse {
    Dent {
        mode: FileMode,
        size: u32,
        mtime: UnixTime,
        name: Vec<u8>,
    },
    StatReply {
        mode: FileMode,
        size: u32,
        mtime: UnixTime,
    },
    Data(DataChunk),
    Done {
        mtime: UnixTime,
    },
    Okay,
    Fail {
        message: String,
    },
}

impl SyncResponse {
    /// Encodes response into bytes. Infallible because bounded types guarantee size limits.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            SyncResponse::Dent {
                mode,
                size,
                mtime,
                name,
            } => {
                let mut buf = Vec::with_capacity(20 + name.len());
                buf.extend_from_slice(&SyncId::DentV1.to_le_bytes());
                buf.extend_from_slice(&mode.mode().to_le_bytes());
                buf.extend_from_slice(&size.to_le_bytes());
                buf.extend_from_slice(&mtime.as_u32().to_le_bytes());
                buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
                buf.extend_from_slice(name);
                buf
            }
            SyncResponse::StatReply { mode, size, mtime } => {
                let mut buf = Vec::with_capacity(16);
                buf.extend_from_slice(&SyncId::LstatV1.to_le_bytes());
                buf.extend_from_slice(&mode.mode().to_le_bytes());
                buf.extend_from_slice(&size.to_le_bytes());
                buf.extend_from_slice(&mtime.as_u32().to_le_bytes());
                buf
            }
            SyncResponse::Data(chunk) => encode_id_payload(SyncId::Data, chunk.as_bytes()),
            SyncResponse::Done { mtime } => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&SyncId::Done.to_le_bytes());
                buf.extend_from_slice(&mtime.as_u32().to_le_bytes());
                buf
            }
            SyncResponse::Okay => {
                let mut buf = Vec::with_capacity(8);
                buf.extend_from_slice(&SyncId::Okay.to_le_bytes());
                buf.extend_from_slice(&0u32.to_le_bytes());
                buf
            }
            SyncResponse::Fail { message } => {
                let b = message.as_bytes();
                let mut buf = Vec::with_capacity(8 + b.len());
                buf.extend_from_slice(&SyncId::Fail.to_le_bytes());
                buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
                buf.extend_from_slice(b);
                buf
            }
        }
    }
}

fn encode_id_payload(id: SyncId, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

/// Incremental decoder for sync request messages.
#[derive(Debug, Default)]
pub struct SyncRequestDecoder {
    buffer: VecDeque<u8>,
}

impl SyncRequestDecoder {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.buffer.extend(chunk);
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn pop_request(&mut self) -> Result<Option<SyncRequest>, SyncDecodeError> {
        if self.buffer.len() < 8 {
            return Ok(None);
        }

        let mut id_b = [0u8; 4];
        let mut param_b = [0u8; 4];
        for (i, b) in self.buffer.iter().take(4).enumerate() {
            id_b[i] = *b;
        }
        for (i, b) in self.buffer.iter().skip(4).take(4).enumerate() {
            param_b[i] = *b;
        }

        let raw_id = u32::from_le_bytes(id_b);
        let param = u32::from_le_bytes(param_b);

        let id = match SyncId::try_from(raw_id) {
            Ok(id) => id,
            Err(unknown) => {
                self.buffer.drain(..4);
                return Err(SyncDecodeError::UnknownId(unknown));
            }
        };

        match id {
            SyncId::ListV1 => {
                let len = param as usize;
                if len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let p = self.pop_bytes(len);
                let path = RemotePath::new(p).map_err(|e| match e {
                    SyncEncodeError::PayloadTooLarge { length, max } => {
                        SyncDecodeError::OversizedLength { length, max }
                    }
                })?;
                Ok(Some(SyncRequest::List(path)))
            }
            SyncId::LstatV1 => {
                let len = param as usize;
                if len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let p = self.pop_bytes(len);
                let path = RemotePath::new(p).map_err(|e| match e {
                    SyncEncodeError::PayloadTooLarge { length, max } => {
                        SyncDecodeError::OversizedLength { length, max }
                    }
                })?;
                Ok(Some(SyncRequest::Stat(path)))
            }
            SyncId::SendV1 => {
                let len = param as usize;
                if len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let payload = self.pop_bytes(len);
                let comma_idx = payload.iter().rposition(|&b| b == b',').ok_or_else(|| {
                    SyncDecodeError::InvalidPayload("missing comma in SEND spec".to_string())
                })?;
                let path_bytes = &payload[..comma_idx];
                let mode_str = std::str::from_utf8(&payload[comma_idx + 1..])
                    .map_err(|e| SyncDecodeError::InvalidPayload(e.to_string()))?;
                let mode_val = parse_mode(mode_str)?;
                let path = RemotePath::new(path_bytes).map_err(|e| match e {
                    SyncEncodeError::PayloadTooLarge { length, max } => {
                        SyncDecodeError::OversizedLength { length, max }
                    }
                })?;
                Ok(Some(SyncRequest::Send {
                    path,
                    mode: FileMode::new(mode_val),
                }))
            }
            SyncId::RecvV1 => {
                let len = param as usize;
                if len > SYNC_PATH_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: len,
                        max: SYNC_PATH_MAX,
                    });
                }
                if self.buffer.len() < 8 + len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let p = self.pop_bytes(len);
                let path = RemotePath::new(p).map_err(|e| match e {
                    SyncEncodeError::PayloadTooLarge { length, max } => {
                        SyncDecodeError::OversizedLength { length, max }
                    }
                })?;
                Ok(Some(SyncRequest::Recv(path)))
            }
            SyncId::Data => {
                let len = param as usize;
                if len > SYNC_DATA_MAX {
                    self.buffer.drain(..8);
                    return Err(SyncDecodeError::OversizedLength {
                        length: len,
                        max: SYNC_DATA_MAX,
                    });
                }
                if self.buffer.len() < 8 + len {
                    return Ok(None);
                }
                self.buffer.drain(..8);
                let d = self.pop_bytes(len);
                let chunk = DataChunk::new(d).map_err(|e| match e {
                    SyncEncodeError::PayloadTooLarge { length, max } => {
                        SyncDecodeError::OversizedLength { length, max }
                    }
                })?;
                Ok(Some(SyncRequest::Data(chunk)))
            }
            SyncId::Done => {
                self.buffer.drain(..8);
                Ok(Some(SyncRequest::Done {
                    mtime: UnixTime::from_secs(param as u64),
                }))
            }
            SyncId::Quit => {
                self.buffer.drain(..8);
                Ok(Some(SyncRequest::Quit))
            }
            _ => {
                self.buffer.drain(..4);
                Err(SyncDecodeError::UnknownId(raw_id))
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

/// Incremental decoder for sync response messages.
#[derive(Debug, Default)]
pub struct SyncResponseDecoder {
    buffer: VecDeque<u8>,
}

impl SyncResponseDecoder {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        self.buffer.extend(chunk);
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn pop_response(&mut self) -> Result<Option<SyncResponse>, SyncDecodeError> {
        if self.buffer.len() < 8 {
            return Ok(None);
        }

        let mut id_b = [0u8; 4];
        let mut param_b = [0u8; 4];
        for (i, b) in self.buffer.iter().take(4).enumerate() {
            id_b[i] = *b;
        }
        for (i, b) in self.buffer.iter().skip(4).take(4).enumerate() {
            param_b[i] = *b;
        }

        let raw_id = u32::from_le_bytes(id_b);
        let param = u32::from_le_bytes(param_b);

        let id = match SyncId::try_from(raw_id) {
            Ok(id) => id,
            Err(unknown) => {
                self.buffer.drain(..4);
                return Err(SyncDecodeError::UnknownId(unknown));
            }
        };

        match id {
            SyncId::DentV1 => {
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
                let name = self.pop_bytes(namelen);

                Ok(Some(SyncResponse::Dent {
                    mode: FileMode::new(mode),
                    size,
                    mtime: UnixTime::from_secs(mtime as u64),
                    name,
                }))
            }
            SyncId::LstatV1 => {
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
                Ok(Some(SyncResponse::StatReply {
                    mode: FileMode::new(u32::from_le_bytes(mode_b)),
                    size: u32::from_le_bytes(size_b),
                    mtime: UnixTime::from_secs(u32::from_le_bytes(mtime_b) as u64),
                }))
            }
            SyncId::Data => {
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
                let chunk = DataChunk::new(data).map_err(|e| match e {
                    SyncEncodeError::PayloadTooLarge { length, max } => {
                        SyncDecodeError::OversizedLength { length, max }
                    }
                })?;
                Ok(Some(SyncResponse::Data(chunk)))
            }
            SyncId::Done => {
                self.buffer.drain(..8);
                Ok(Some(SyncResponse::Done {
                    mtime: UnixTime::from_secs(param as u64),
                }))
            }
            SyncId::Okay => {
                self.buffer.drain(..8);
                Ok(Some(SyncResponse::Okay))
            }
            SyncId::Fail => {
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
                let message = String::from_utf8_lossy(&payload).into_owned();
                Err(SyncDecodeError::FailMessage(message))
            }
            _ => {
                self.buffer.drain(..4);
                Err(SyncDecodeError::UnknownId(raw_id))
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
