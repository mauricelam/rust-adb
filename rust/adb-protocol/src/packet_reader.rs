//! Utilities for reading ADB packets from a byte stream.

use adb_types::{Amessage, Apacket, Block};
use std::io::Read;
use thiserror::Error;

/// Errors that can occur while reading an ADB packet.
#[derive(Debug, Error)]
pub enum AddError {
    /// An IO error occurred.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// The packet magic value was invalid.
    #[error("Invalid magic")]
    InvalidMagic,
    /// The packet checksum was invalid.
    #[error("Invalid checksum")]
    InvalidChecksum,
    /// The payload size exceeds the maximum allowed.
    #[error("Oversized payload")]
    OversizedPayload,
}

/// A reader for ADB packets that maintains state between reads.
pub struct APacketReader {
    header_buf: [u8; std::mem::size_of::<Amessage>()],
    header_pos: usize,
    payload: Option<Block>,
}

impl APacketReader {
    /// Creates a new `APacketReader`.
    pub fn new() -> Self {
        Self {
            header_buf: [0u8; std::mem::size_of::<Amessage>()],
            header_pos: 0,
            payload: None,
        }
    }

    /// Attempts to read a packet from the given reader.
    /// Returns `Ok(Some(packet))` if a complete packet was read, `Ok(None)` if more data is needed.
    pub fn read_packet<R: Read>(&mut self, reader: &mut R) -> Result<Option<Apacket>, AddError> {
        while self.header_pos < self.header_buf.len() {
            match reader.read(&mut self.header_buf[self.header_pos..]) {
                Ok(0) => return Err(AddError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "EOF"))),
                Ok(n) => self.header_pos += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) => return Err(AddError::Io(e)),
            }
        }

        let msg: Amessage = unsafe { std::ptr::read_unaligned(self.header_buf.as_ptr() as *const Amessage) };
        if !msg.check_magic() {
            return Err(AddError::InvalidMagic);
        }

        if msg.data_length as usize > crate::MAX_PAYLOAD {
            return Err(AddError::OversizedPayload);
        }

        if self.payload.is_none() {
            self.payload = Some(Block::new(msg.data_length as usize));
        }

        let payload = self.payload.as_mut().unwrap();
        while payload.remaining() > 0 {
            let pos = payload.position() as usize;
            match reader.read(&mut payload.get_mut()[pos..]) {
                Ok(0) => return Err(AddError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "EOF"))),
                Ok(n) => payload.set_position((pos + n) as u64),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(e) => return Err(AddError::Io(e)),
            }
        }

        let payload = self.payload.take().unwrap();
        self.header_pos = 0;

        Ok(Some(Apacket { msg, payload }))
    }
}

impl Default for APacketReader {
    fn default() -> Self {
        Self::new()
    }
}
