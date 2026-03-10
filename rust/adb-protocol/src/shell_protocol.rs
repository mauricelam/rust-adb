//! ADB shell protocol implementation.

use std::io::{Read, Write};

/// Shell protocol packet types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ShellId {
    /// Input to the shell process.
    Stdin = 0,
    /// Output from the shell process.
    Stdout = 1,
    /// Error output from the shell process.
    Stderr = 2,
    /// Process exit status.
    Exit = 3,
    /// Signal that stdin has been closed.
    CloseStdin = 4,
    /// Signal a window size change.
    WindowSizeChange = 5,
    /// Invalid ID.
    Invalid = 255,
}

impl From<u8> for ShellId {
    fn from(id: u8) -> Self {
        match id {
            0 => ShellId::Stdin,
            1 => ShellId::Stdout,
            2 => ShellId::Stderr,
            3 => ShellId::Exit,
            4 => ShellId::CloseStdin,
            5 => ShellId::WindowSizeChange,
            _ => ShellId::Invalid,
        }
    }
}

/// A shell protocol packet.
pub struct ShellProtocol {
    /// Packet type ID.
    pub id: ShellId,
    /// Packet payload.
    pub data: Vec<u8>,
}

impl ShellProtocol {
    /// Creates a new, empty `ShellProtocol` packet.
    pub fn new() -> Self {
        Self {
            id: ShellId::Invalid,
            data: Vec::new(),
        }
    }

    /// Reads a shell protocol packet from the reader.
    pub fn read<R: Read>(&mut self, reader: &mut R) -> std::io::Result<bool> {
        let mut header = [0u8; 5];
        match reader.read_exact(&mut header) {
            Ok(_) => {
                self.id = ShellId::from(header[0]);
                let len = u32::from_le_bytes(header[1..5].try_into().unwrap()) as usize;
                self.data.resize(len, 0);
                reader.read_exact(&mut self.data)?;
                Ok(true)
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Writes the packet to the writer.
    pub fn write<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let mut header = [0u8; 5];
        header[0] = self.id as u8;
        header[1..5].copy_from_slice(&(self.data.len() as u32).to_le_bytes());
        writer.write_all(&header)?;
        writer.write_all(&self.data)
    }

    /// Helper to write a single shell protocol packet.
    pub fn write_packet<W: Write>(writer: &mut W, id: ShellId, data: &[u8]) -> std::io::Result<()> {
        let mut header = [0u8; 5];
        header[0] = id as u8;
        header[1..5].copy_from_slice(&(data.len() as u32).to_le_bytes());
        writer.write_all(&header)?;
        writer.write_all(data)
    }
}

impl Default for ShellProtocol {
    fn default() -> Self {
        Self::new()
    }
}
