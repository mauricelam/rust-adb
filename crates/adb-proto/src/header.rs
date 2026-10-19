use std::fmt;

/// ADB wire protocol command identifier.
///
/// Ports command constants (`A_CNXN`, `A_AUTH`, etc.) from `original/adb.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Command {
    /// Obsolete command `A_SYNC` from `original/adb.h`.
    Sync = 0x434e5953,
    /// Connect command `A_CNXN` from `original/adb.h`.
    Cnxn = 0x4e584e43,
    /// Open stream command `A_OPEN` from `original/adb.h`.
    Open = 0x4e45504f,
    /// Okay acknowledgement command `A_OKAY` from `original/adb.h`.
    Okay = 0x59414b4f,
    /// Close stream command `A_CLSE` from `original/adb.h`.
    Clse = 0x45534c43,
    /// Write stream data command `A_WRTE` from `original/adb.h`.
    Wrte = 0x45545257,
    /// Authentication command `A_AUTH` from `original/adb.h`.
    Auth = 0x48545541,
    /// Stream-based TLS command `A_STLS` from `original/adb.h`.
    Stls = 0x534c5453,
}

impl TryFrom<u32> for Command {
    type Error = u32;

    /// Attempts to convert a raw `u32` value into a valid [`Command`].
    ///
    /// Ports command matching logic from `original/adb.cpp` / `original/adb.h`.
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x434e5953 => Ok(Command::Sync),
            0x4e584e43 => Ok(Command::Cnxn),
            0x4e45504f => Ok(Command::Open),
            0x59414b4f => Ok(Command::Okay),
            0x45534c43 => Ok(Command::Clse),
            0x45545257 => Ok(Command::Wrte),
            0x48545541 => Ok(Command::Auth),
            0x534c5453 => Ok(Command::Stls),
            _ => Err(value),
        }
    }
}

/// Errors that can occur when decoding an ADB header.
///
/// Ports error handling logic for invalid header parsing in `original/apacket_reader.cpp`
/// and `original/transport_fd.cpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Provided byte slice is shorter than the required header size (24 bytes).
    BufferTooShort { expected: usize, actual: usize },
    /// Command numeric value is not recognized.
    UnknownCommand(u32),
    /// Header magic field does not equal `command ^ 0xffff_ffff`.
    InvalidMagic { expected: u32, actual: u32 },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::BufferTooShort { expected, actual } => write!(
                f,
                "buffer too short for header: expected {expected} bytes, got {actual}"
            ),
            DecodeError::UnknownCommand(cmd) => {
                write!(f, "unknown ADB command numeric value: 0x{cmd:08x}")
            }
            DecodeError::InvalidMagic { expected, actual } => write!(
                f,
                "invalid header magic: expected 0x{expected:08x}, got 0x{actual:08x}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Errors that can occur when encoding an ADB header into a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Target buffer is too short to write the 24-byte header.
    BufferTooShort { expected: usize, actual: usize },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncodeError::BufferTooShort { expected, actual } => write!(
                f,
                "target buffer too short for header: expected {expected} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Represents the 24-byte header of an ADB protocol packet.
///
/// Ports the `amessage` struct from `original/types.h` and `message` struct from
/// `original/docs/dev/protocol.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Command identifier constant (`command` in `amessage`).
    pub command: Command,
    /// First argument (`arg0` in `amessage`).
    pub arg0: u32,
    /// Second argument (`arg1` in `amessage`).
    pub arg1: u32,
    /// Length of payload in bytes (`data_length` in `amessage`).
    pub data_length: u32,
    /// Checksum or CRC32 of payload (`data_check` in `amessage` / `data_crc32` in `protocol.md`).
    pub data_crc32: u32,
    /// Magic XOR mask (`magic` in `amessage`, equal to `command ^ 0xffff_ffff`).
    pub magic: u32,
}

impl Header {
    /// Total fixed byte length of an ADB message header (24 bytes).
    ///
    /// Ports `sizeof(amessage)` from `original/types.h`.
    pub const SIZE: usize = 24;

    /// Constructs a new [`Header`] with `magic` automatically computed from `command`.
    ///
    /// Ports `amessage` initialization in `original/types.h` and `original/adb.cpp`.
    pub fn new(command: Command, arg0: u32, arg1: u32, data_length: u32, data_crc32: u32) -> Self {
        let cmd_u32 = command as u32;
        Self {
            command,
            arg0,
            arg1,
            data_length,
            data_crc32,
            magic: cmd_u32 ^ 0xffff_ffff,
        }
    }

    /// Encodes the header into a fixed 24-byte little-endian array.
    ///
    /// Ports `amessage` byte serialization in `original/apacket_reader.cpp` and `original/types_test.cpp`.
    pub fn encode(&self) -> [u8; 24] {
        let mut buf = [0u8; 24];
        buf[0..4].copy_from_slice(&(self.command as u32).to_le_bytes());
        buf[4..8].copy_from_slice(&self.arg0.to_le_bytes());
        buf[8..12].copy_from_slice(&self.arg1.to_le_bytes());
        buf[12..16].copy_from_slice(&self.data_length.to_le_bytes());
        buf[16..20].copy_from_slice(&self.data_crc32.to_le_bytes());
        buf[20..24].copy_from_slice(&self.magic.to_le_bytes());
        buf
    }

    /// Encodes the header into the provided byte slice.
    ///
    /// Returns [`EncodeError::BufferTooShort`] if `buf.len() < 24`.
    pub fn encode_to_slice(&self, buf: &mut [u8]) -> Result<(), EncodeError> {
        if buf.len() < Self::SIZE {
            return Err(EncodeError::BufferTooShort {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }
        buf[..Self::SIZE].copy_from_slice(&self.encode());
        Ok(())
    }

    /// Decodes an ADB packet header from a byte slice in little-endian format.
    ///
    /// Validates that the buffer contains at least 24 bytes, the command is valid,
    /// and `magic` matches `command ^ 0xffff_ffff`. Does not panic on short input
    /// or invalid payload sizes.
    ///
    /// Ports header parsing and magic validation from `original/apacket_reader.cpp` and `original/transport_fd.cpp`.
    pub fn decode(buf: &[u8]) -> Result<Header, DecodeError> {
        if buf.len() < Self::SIZE {
            return Err(DecodeError::BufferTooShort {
                expected: Self::SIZE,
                actual: buf.len(),
            });
        }

        let command_u32 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let arg0 = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let arg1 = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let data_length = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let data_crc32 = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        let magic = u32::from_le_bytes(buf[20..24].try_into().unwrap());

        let command =
            Command::try_from(command_u32).map_err(|_| DecodeError::UnknownCommand(command_u32))?;

        let expected_magic = command_u32 ^ 0xffff_ffff;
        if magic != expected_magic {
            return Err(DecodeError::InvalidMagic {
                expected: expected_magic,
                actual: magic,
            });
        }

        Ok(Header {
            command,
            arg0,
            arg1,
            data_length,
            data_crc32,
            magic,
        })
    }
}
