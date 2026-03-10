//! ADB file sync protocol definitions.
//! Ported from `file_sync_protocol.h`.

use zerocopy::{FromBytes, Immutable, IntoBytes};

/// Creates a 4-byte protocol ID from four characters.
pub const fn mkid(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

/// LSTAT v1 command ID.
pub const ID_LSTAT_V1: u32 = mkid(b'S', b'T', b'A', b'T');
/// STAT v2 command ID.
pub const ID_STAT_V2: u32 = mkid(b'S', b'T', b'A', b'2');
/// LSTAT v2 command ID.
pub const ID_LSTAT_V2: u32 = mkid(b'L', b'S', b'T', b'2');

/// LIST v1 command ID.
pub const ID_LIST_V1: u32 = mkid(b'L', b'I', b'S', b'T');
/// LIST v2 command ID.
pub const ID_LIST_V2: u32 = mkid(b'L', b'I', b'S', b'2');
/// DENT v1 (directory entry) command ID.
pub const ID_DENT_V1: u32 = mkid(b'D', b'E', b'N', b'T');
/// DENT v2 (directory entry) command ID.
pub const ID_DENT_V2: u32 = mkid(b'D', b'N', b'T', b'2');

/// SEND v1 command ID.
pub const ID_SEND_V1: u32 = mkid(b'S', b'E', b'N', b'D');
/// SEND v2 command ID.
pub const ID_SEND_V2: u32 = mkid(b'S', b'N', b'D', b'2');
/// RECV v1 command ID.
pub const ID_RECV_V1: u32 = mkid(b'R', b'E', b'C', b'V');
/// RECV v2 command ID.
pub const ID_RECV_V2: u32 = mkid(b'R', b'C', b'V', b'2');
/// DONE command ID.
pub const ID_DONE: u32 = mkid(b'D', b'O', b'N', b'E');
/// DATA command ID.
pub const ID_DATA: u32 = mkid(b'D', b'A', b'T', b'A');
/// OKAY command ID.
pub const ID_OKAY: u32 = mkid(b'O', b'K', b'A', b'Y');
/// FAIL command ID.
pub const ID_FAIL: u32 = mkid(b'F', b'A', b'I', b'L');
/// QUIT command ID.
pub const ID_QUIT: u32 = mkid(b'Q', b'U', b'I', b'T');

/// A request to the sync service.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncRequest {
    /// Command ID.
    pub id: u32,           // ID_STAT, et cetera.
    /// Length of the following path string.
    pub path_length: u32,  // <= 1024
}

/// STAT v1 response.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncStatV1 {
    /// Command ID (ID_LSTAT_V1).
    pub id: u32,
    /// File mode.
    pub mode: u32,
    /// File size.
    pub size: u32,
    /// Last modification time.
    pub mtime: u32,
}

/// STAT v2 response.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncStatV2 {
    /// Command ID (ID_STAT_V2 or ID_LSTAT_V2).
    pub id: u32,
    /// Error code (errno).
    pub error: u32,
    /// Device ID.
    pub dev: u64,
    /// Inode number.
    pub ino: u64,
    /// File mode.
    pub mode: u32,
    /// Number of hard links.
    pub nlink: u32,
    /// User ID.
    pub uid: u32,
    /// Group ID.
    pub gid: u32,
    /// File size.
    pub size: u64,
    /// Last access time.
    pub atime: i64,
    /// Last modification time.
    pub mtime: i64,
    /// Last status change time.
    pub ctime: i64,
}

/// Directory entry v1 response.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncDentV1 {
    /// Command ID (ID_DENT_V1).
    pub id: u32,
    /// File mode.
    pub mode: u32,
    /// File size.
    pub size: u32,
    /// Last modification time.
    pub mtime: u32,
    /// Length of the following filename string.
    pub namelen: u32,
}

/// Directory entry v2 response.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncDentV2 {
    /// Command ID (ID_DENT_V2).
    pub id: u32,
    /// Error code (errno).
    pub error: u32,
    /// Device ID.
    pub dev: u64,
    /// Inode number.
    pub ino: u64,
    /// File mode.
    pub mode: u32,
    /// Number of hard links.
    pub nlink: u32,
    /// User ID.
    pub uid: u32,
    /// Group ID.
    pub gid: u32,
    /// File size.
    pub size: u64,
    /// Last access time.
    pub atime: i64,
    /// Last modification time.
    pub mtime: i64,
    /// Last status change time.
    pub ctime: i64,
    /// Length of the following filename string.
    pub namelen: u32,
}

/// No flags.
pub const SYNC_FLAG_NONE: u32 = 0;
/// Use Brotli compression.
pub const SYNC_FLAG_BROTLI: u32 = 1;
/// Use LZ4 compression.
pub const SYNC_FLAG_LZ4: u32 = 2;
/// Use Zstd compression.
pub const SYNC_FLAG_ZSTD: u32 = 4;
/// Perform a dry run (don't write to disk).
pub const SYNC_FLAG_DRY_RUN: u32 = 0x8000_0000;

/// Compression types for SEND/RECV v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionType {
    /// No compression.
    None,
    /// Any supported compression.
    Any,
    /// Brotli compression.
    Brotli,
    /// LZ4 compression.
    LZ4,
    /// Zstd compression.
    Zstd,
}

/// SEND v2 setup message.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncSendV2 {
    /// Command ID (ID_SEND_V2).
    pub id: u32,
    /// File mode.
    pub mode: u32,
    /// Flags (compression, dry run).
    pub flags: u32,
}

/// RECV v2 setup message.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncRecvV2 {
    /// Command ID (ID_RECV_V2).
    pub id: u32,
    /// Flags (compression).
    pub flags: u32,
}

/// Data packet in the sync protocol.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncData {
    /// Command ID (ID_DATA).
    pub id: u32,
    /// Size of the following data block.
    pub size: u32,
}

/// Status response message.
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, FromBytes, IntoBytes, Immutable)]
pub struct SyncStatus {
    /// Command ID (ID_OKAY or ID_FAIL).
    pub id: u32,
    /// Length of the following error message string.
    pub msglen: u32,
}

/// Maximum size of a sync data block.
pub const SYNC_DATA_MAX: usize = 64 * 1024;
