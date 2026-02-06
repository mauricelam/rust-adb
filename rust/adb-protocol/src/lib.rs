/*
 * Copyright (C) 2007 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

pub mod packet_reader;
pub use packet_reader::{APacketReader, AddError};

pub mod shell_protocol;
pub use shell_protocol::{ShellId, ShellProtocol};

pub const MAX_PAYLOAD_V1: usize = 4 * 1024;
pub const MAX_PAYLOAD: usize = 1024 * 1024;
pub const INITIAL_DELAYED_ACK_BYTES: usize = 32 * 1024 * 1024;

pub const A_SYNC: u32 = 0x434e5953;
pub const A_CNXN: u32 = 0x4e584e43;
pub const A_OPEN: u32 = 0x4e45504f;
pub const A_OKAY: u32 = 0x59414b4f;
pub const A_CLSE: u32 = 0x45534c43;
pub const A_WRTE: u32 = 0x45545257;
pub const A_AUTH: u32 = 0x48545541;
pub const A_STLS: u32 = 0x534c5453;

pub const A_VERSION_MIN: u32 = 0x01000000;
pub const A_VERSION_SKIP_CHECKSUM: u32 = 0x01000001;
pub const A_VERSION: u32 = 0x01000001;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    Usb,
    Local,
    Any,
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ConnectionState {
    Any = -1,
    Connecting = 0,
    Authorizing = 1,
    Unauthorized = 2,
    NoPerm = 3,
    Detached = 4,
    Offline = 5,
    Bootloader = 6,
    Device = 7,
    Host = 8,
    Recovery = 9,
    Sideload = 10,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_to_string() {
        assert_eq!(command_to_string(A_SYNC), "SYNC");
        assert_eq!(command_to_string(0x12345678), "12345678");
    }

    #[test]
    fn test_connection_state_is_online() {
        assert!(ConnectionState::Device.is_online());
        assert!(!ConnectionState::Offline.is_online());
        assert!(!ConnectionState::Connecting.is_online());
    }

    #[test]
    fn test_connection_state_display() {
        assert_eq!(ConnectionState::Offline.to_string(), "offline");
        assert_eq!(ConnectionState::Device.to_string(), "device");
        assert_eq!(ConnectionState::Any.to_string(), "any");
    }
}
