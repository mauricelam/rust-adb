//! ADB protocol definitions and constants.
//! Ported from `adb.h`, `adb_protocol.h`.

/// Utilities for reading ADB packets from a byte stream.
pub mod packet_reader;
pub use packet_reader::{APacketReader, AddError};

/// ADB shell protocol implementation.
pub mod shell_protocol;
pub use shell_protocol::{ShellId, ShellProtocol};

/// ADB file sync protocol implementation.
pub mod file_sync_protocol;

/// Maximum payload size for protocol version 1.
pub const MAX_PAYLOAD_V1: usize = 4 * 1024;
/// Maximum payload size for the current protocol version.
pub const MAX_PAYLOAD: usize = 1024 * 1024;
/// Initial number of bytes to acknowledge for delayed-ACK.
pub const INITIAL_DELAYED_ACK_BYTES: usize = 32 * 1024 * 1024;

/// SYNC command.
pub const A_SYNC: u32 = 0x434e5953;
/// CNXN (Connection) command.
pub const A_CNXN: u32 = 0x4e584e43;
/// OPEN command.
pub const A_OPEN: u32 = 0x4e45504f;
/// OKAY command.
pub const A_OKAY: u32 = 0x59414b4f;
/// CLSE (Close) command.
pub const A_CLSE: u32 = 0x45534c43;
/// WRTE (Write) command.
pub const A_WRTE: u32 = 0x45545257;
/// AUTH (Authentication) command.
pub const A_AUTH: u32 = 0x48545541;
/// STLS (Secure Transport Layer Security) command.
pub const A_STLS: u32 = 0x534c5453;

/// Version of the STLS protocol.
pub const A_STLS_VERSION: u32 = 1;

/// Minimum supported protocol version.
pub const A_VERSION_MIN: u32 = 0x01000000;
/// Protocol version that skipped checksums.
pub const A_VERSION_SKIP_CHECKSUM: u32 = 0x01000001;
/// Current protocol version.
pub const A_VERSION: u32 = 0x01000001;

/// ADB server version number.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Returns a string representation of an ADB command.
pub fn command_to_string(cmd: u32) -> String {
    match cmd {
        A_SYNC => "SYNC".to_string(),
        A_CNXN => "CNXN".to_string(),
        A_OPEN => "OPEN".to_string(),
        A_OKAY => "OKAY".to_string(),
        A_CLSE => "CLSE".to_string(),
        A_WRTE => "WRTE".to_string(),
        A_AUTH => "AUTH".to_string(),
        A_STLS => "STLS".to_string(),
        _ => format!("{:08x}", cmd),
    }
}

/// Types of transport supported by ADB.
/// Ported from `TransportType` in `adb.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// USB transport.
    Usb,
    /// Local (TCP/Unix) transport.
    Local,
    /// Any available transport.
    Any,
    /// Host-side transport.
    Host,
}

/// Possible states of an ADB connection.
/// Ported from `ConnectionState` in `adb.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ConnectionState {
    /// Any state.
    Any = -1,
    /// Connecting to the device.
    Connecting = 0,
    /// Authorizing the connection.
    Authorizing = 1,
    /// Unauthorized connection.
    Unauthorized = 2,
    /// No permissions to access the device.
    NoPerm = 3,
    /// Transport is detached.
    Detached = 4,
    /// Connection is offline.
    Offline = 5,
    /// Device is in bootloader mode.
    Bootloader = 6,
    /// Device is online.
    Device = 7,
    /// Connection is to a host.
    Host = 8,
    /// Device is in recovery mode.
    Recovery = 9,
    /// Device is in sideload mode.
    Sideload = 10,
    /// Device is in rescue mode.
    Rescue = 11,
}

impl TryFrom<i32> for ConnectionState {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            -1 => Ok(ConnectionState::Any),
            0 => Ok(ConnectionState::Connecting),
            1 => Ok(ConnectionState::Authorizing),
            2 => Ok(ConnectionState::Unauthorized),
            3 => Ok(ConnectionState::NoPerm),
            4 => Ok(ConnectionState::Detached),
            5 => Ok(ConnectionState::Offline),
            6 => Ok(ConnectionState::Bootloader),
            7 => Ok(ConnectionState::Device),
            8 => Ok(ConnectionState::Host),
            9 => Ok(ConnectionState::Recovery),
            10 => Ok(ConnectionState::Sideload),
            11 => Ok(ConnectionState::Rescue),
            _ => Err(value),
        }
    }
}

impl ConnectionState {
    /// Returns true if the connection is in an online state.
    pub fn is_online(&self) -> bool {
        match self {
            ConnectionState::Bootloader
            | ConnectionState::Device
            | ConnectionState::Host
            | ConnectionState::Recovery
            | ConnectionState::Sideload
            | ConnectionState::Rescue => true,
            _ => false,
        }
    }
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConnectionState::Any => "any",
            ConnectionState::Connecting => "connecting",
            ConnectionState::Authorizing => "authorizing",
            ConnectionState::Unauthorized => "unauthorized",
            ConnectionState::NoPerm => "no permissions",
            ConnectionState::Detached => "detached",
            ConnectionState::Offline => "offline",
            ConnectionState::Bootloader => "bootloader",
            ConnectionState::Device => "device",
            ConnectionState::Host => "host",
            ConnectionState::Recovery => "recovery",
            ConnectionState::Sideload => "sideload",
            ConnectionState::Rescue => "rescue",
        };
        write!(f, "{}", s)
    }
}
