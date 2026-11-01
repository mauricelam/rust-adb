//! Transport type definitions.
//!
//! Ported from `enum TransportType` in `original/adb.h`.

use std::fmt;

/// Transport type of a connected device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransportType {
    /// Connected via USB.
    Usb,
    /// Connected via TCP/Local network (e.g. emulator, adb connect).
    Local,
    /// Any transport type (used in target queries).
    #[default]
    Any,
    /// Host-side server transport.
    Host,
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usb => write!(f, "usb"),
            Self::Local => write!(f, "local"),
            Self::Any => write!(f, "any"),
            Self::Host => write!(f, "host"),
        }
    }
}
