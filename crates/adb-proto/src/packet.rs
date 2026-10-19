/// ADB protocol command constants
pub const A_SYNC: u32 = 0x434e5953;
pub const A_CNXN: u32 = 0x4e584e43;
pub const A_AUTH: u32 = 0x48545541;
pub const A_OPEN: u32 = 0x4e45504f;
pub const A_OKAY: u32 = 0x59414b4f;
pub const A_CLSE: u32 = 0x45534c43;
pub const A_WRTE: u32 = 0x45545257;
pub const A_STLS: u32 = 0x534c5453;

pub const A_VERSION_MIN: u32 = 0x01000000;
pub const A_VERSION_SKIP_CHECKSUM: u32 = 0x01000001;
pub const A_VERSION: u32 = 0x01000001;
pub const MAX_PAYLOAD_V1: u32 = 4096;
pub const MAX_PAYLOAD: u32 = 1024 * 1024;

/// Calculates the ADB packet payload checksum (sum of all payload bytes as u32).
pub fn calculate_apacket_checksum(payload: &[u8]) -> u32 {
    payload
        .iter()
        .fold(0u32, |acc, &byte| acc.wrapping_add(byte as u32))
}

/// 24-byte ADB message header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Amessage {
    pub command: u32,
    pub arg0: u32,
    pub arg1: u32,
    pub data_length: u32,
    pub data_crc32: u32,
    pub magic: u32,
}

impl Amessage {
    pub const HEADER_SIZE: usize = 24;

    pub fn new(command: u32, arg0: u32, arg1: u32, payload: &[u8]) -> Self {
        let data_crc32 = if arg0 >= A_VERSION_SKIP_CHECKSUM {
            0
        } else {
            calculate_apacket_checksum(payload)
        };
        Self {
            command,
            arg0,
            arg1,
            data_length: payload.len() as u32,
            data_crc32,
            magic: command ^ 0xffff_ffff,
        }
    }

    /// Checks if the message header magic field is valid (`magic == command ^ 0xffffffff`).
    pub fn is_valid(&self) -> bool {
        self.magic == (self.command ^ 0xffff_ffff)
    }

    /// Serializes the header into a 24-byte little-endian array.
    pub fn to_le_bytes(&self) -> [u8; 24] {
        let mut bytes = [0u8; 24];
        bytes[0..4].copy_from_slice(&self.command.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.arg0.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.arg1.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.data_length.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.data_crc32.to_le_bytes());
        bytes[20..24].copy_from_slice(&self.magic.to_le_bytes());
        bytes
    }

    /// Deserializes a header from a 24-byte little-endian array.
    pub fn from_le_bytes(bytes: &[u8; 24]) -> Self {
        Self {
            command: u32::from_le_bytes(bytes[0..4].try_into().unwrap()),
            arg0: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            arg1: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            data_length: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
            data_crc32: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            magic: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        }
    }
}

/// ADB packet containing header and payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apacket {
    pub msg: Amessage,
    pub payload: Vec<u8>,
}

impl Apacket {
    pub fn new(command: u32, arg0: u32, arg1: u32, payload: Vec<u8>) -> Self {
        let msg = Amessage::new(command, arg0, arg1, &payload);
        Self { msg, payload }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amessage_roundtrip() {
        let payload = b"hello world";
        let msg = Amessage::new(A_CNXN, A_VERSION_MIN, MAX_PAYLOAD, payload);
        assert!(msg.is_valid());
        assert_eq!(msg.data_length, 11);
        assert_eq!(msg.data_crc32, calculate_apacket_checksum(payload));

        let bytes = msg.to_le_bytes();
        let parsed = Amessage::from_le_bytes(&bytes);
        assert_eq!(msg, parsed);
        assert!(parsed.is_valid());

        let msg_v1 = Amessage::new(A_CNXN, A_VERSION, MAX_PAYLOAD, payload);
        assert_eq!(msg_v1.data_crc32, 0);
    }

    #[test]
    fn test_invalid_magic() {
        let mut msg = Amessage::new(A_OKAY, 1, 2, &[]);
        assert!(msg.is_valid());
        msg.magic = 0;
        assert!(!msg.is_valid());
    }
}
