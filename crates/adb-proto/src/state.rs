//! Connection states for ADB transports.
//!
//! Ported from `original/adb.h` and `original/transport.cpp`.

use std::fmt;
use std::str::FromStr;

/// Transport connection states matching upstream ADB's `ConnectionState`.
///
/// Ported from `enum ConnectionState` in `original/adb.h`:
/// ```cpp
/// enum ConnectionState {
///     kCsAny = -1,
///     kCsConnecting = 0,
///     kCsAuthorizing,
///     kCsUnauthorized,
///     kCsNoPerm,
///     kCsDetached,
///     kCsOffline,
///     kCsBootloader,
///     kCsDevice,
///     kCsHost,
///     kCsRecovery,
///     kCsSideload,
///     kCsRescue,
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConnectionState {
    /// A peer has been detected (device/host) but no comm has started yet.
    #[default]
    Offline,
    /// Haven't received a response from the device yet.
    Connecting,
    /// Authorizing with keys from ADB_VENDOR_KEYS or default keys.
    Authorizing,
    /// Keys exhausted, fell back to user prompt dialog on device.
    Unauthorized,
    /// Insufficient permissions to communicate with the device.
    NoPerm,
    /// USB device detached from the adb server.
    Detached,
    /// Device running fastboot OS or userspace fastboot.
    Bootloader,
    /// Device running Android OS (adbd).
    Device,
    /// What a device sees from its end of a Transport (adb host).
    Host,
    /// Device with bootloader loaded but no ROM OS loaded.
    Recovery,
    /// Device running Android OS Sideload mode (minadbd).
    Sideload,
    /// Device running Android OS Rescue mode.
    Rescue,
}

/// Error type when parsing a [`ConnectionState`] from a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseConnectionStateError(pub String);

impl fmt::Display for ParseConnectionStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown connection state '{}'", self.0)
    }
}

impl std::error::Error for ParseConnectionStateError {}

impl ConnectionState {
    /// Returns the string representation matching upstream C++ `to_string(ConnectionState)`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Connecting => "connecting",
            Self::Authorizing => "authorizing",
            Self::Unauthorized => "unauthorized",
            Self::NoPerm => "no permissions",
            Self::Detached => "detached",
            Self::Bootloader => "bootloader",
            Self::Device => "device",
            Self::Host => "host",
            Self::Recovery => "recovery",
            Self::Sideload => "sideload",
            Self::Rescue => "rescue",
        }
    }

    /// Checks if the connection is considered online for device operations.
    ///
    /// Ported from `ConnectionStateIsOnline` in `original/adb.h`.
    pub fn is_online(&self) -> bool {
        matches!(
            self,
            Self::Bootloader
                | Self::Device
                | Self::Host
                | Self::Recovery
                | Self::Sideload
                | Self::Rescue
        )
    }

    /// Parses a service type string (from CNXN banner, e.g. "device", "recovery", "sideload", "bootloader", "rescue")
    /// into the corresponding `ConnectionState`.
    ///
    /// Ported from `original/adb.cpp`:
    /// ```cpp
    /// if (type == "bootloader") {
    ///     t->SetConnectionState(kCsBootloader);
    /// } else if (type == "device") { ... }
    /// ```
    pub fn from_service_type(service_type: &str) -> Self {
        match service_type {
            "bootloader" => Self::Bootloader,
            "device" => Self::Device,
            "recovery" => Self::Recovery,
            "sideload" => Self::Sideload,
            "rescue" => Self::Rescue,
            _ => Self::Host,
        }
    }
}

impl FromStr for ConnectionState {
    type Err = ParseConnectionStateError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "offline" => Ok(Self::Offline),
            "connecting" => Ok(Self::Connecting),
            "authorizing" => Ok(Self::Authorizing),
            "unauthorized" => Ok(Self::Unauthorized),
            "no permissions" | "noperm" => Ok(Self::NoPerm),
            "detached" => Ok(Self::Detached),
            "bootloader" => Ok(Self::Bootloader),
            "device" => Ok(Self::Device),
            "host" => Ok(Self::Host),
            "recovery" => Ok(Self::Recovery),
            "sideload" => Ok(Self::Sideload),
            "rescue" => Ok(Self::Rescue),
            other => Err(ParseConnectionStateError(other.to_string())),
        }
    }
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
