use std::fmt;

pub use crate::limits::MAX_PAYLOAD;

/// Shell packet identifier corresponding to shell protocol stream IDs.
///
/// Ports `ShellProtocol::Id` from `original/shell_protocol.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ShellPacketId {
    /// Subprocess standard input stream (`kIdStdin = 0`).
    Stdin = 0,
    /// Subprocess standard output stream (`kIdStdout = 1`).
    Stdout = 1,
    /// Subprocess standard error stream (`kIdStderr = 2`).
    Stderr = 2,
    /// Subprocess exit status code (`kIdExit = 3`).
    Exit = 3,
    /// Close subprocess standard input (`kIdCloseStdin = 4`).
    CloseStdin = 4,
    /// Window size change event (`kIdWindowSizeChange = 5`).
    WindowSizeChange = 5,
    /// Invalid or unknown shell packet (`kIdInvalid = 255`).
    Invalid = 255,
}

impl TryFrom<u8> for ShellPacketId {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ShellPacketId::Stdin),
            1 => Ok(ShellPacketId::Stdout),
            2 => Ok(ShellPacketId::Stderr),
            3 => Ok(ShellPacketId::Exit),
            4 => Ok(ShellPacketId::CloseStdin),
            5 => Ok(ShellPacketId::WindowSizeChange),
            255 => Ok(ShellPacketId::Invalid),
            val => Err(val),
        }
    }
}

/// Errors that can occur when encoding a Shell v2 packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEncodeError {
    /// Payload exceeds maximum allowed size (`MAX_PAYLOAD`).
    PayloadTooLarge { length: usize, max: usize },
}

impl fmt::Display for ShellEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellEncodeError::PayloadTooLarge { length, max } => {
                write!(f, "shell payload too large: {length} bytes (max: {max})")
            }
        }
    }
}

impl std::error::Error for ShellEncodeError {}

/// Errors that can occur when decoding Shell v2 packets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellDecodeError {
    /// Numeric packet ID is not recognized by `ShellPacketId`.
    UnknownPacketId(u8),
    /// Declared payload length exceeds maximum allowed size (`MAX_PAYLOAD`).
    PayloadTooLarge { length: usize, max: usize },
    /// Invalid payload structure for special packet types (e.g. exit code or window size).
    InvalidPayload(String),
}

impl fmt::Display for ShellDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShellDecodeError::UnknownPacketId(id) => write!(f, "unknown shell packet id: {id}"),
            ShellDecodeError::PayloadTooLarge { length, max } => {
                write!(f, "shell packet length exceeds max: {length} (max: {max})")
            }
            ShellDecodeError::InvalidPayload(msg) => write!(f, "invalid shell payload: {msg}"),
        }
    }
}

impl std::error::Error for ShellDecodeError {}

/// Terminal window dimensions used in [`ShellPacketId::WindowSizeChange`].
///
/// Ports `winsize` struct usage from `original/daemon/shell_service.cpp` and `original/client/commandline.cpp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub rows: u16,
    pub cols: u16,
    pub x_pixels: u16,
    pub y_pixels: u16,
}

/// Represents a Shell v2 packet with a 1-byte packet ID, a 4-byte LE length header, and payload data.
///
/// Ports packet framing from `original/shell_protocol.h` and `original/shell_service_protocol.cpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellPacket {
    pub id: ShellPacketId,
    pub data: Vec<u8>,
}

impl ShellPacket {
    /// Header size in bytes: 1 byte ID + 4 bytes LE uint32 length.
    pub const HEADER_SIZE: usize = 5;

    /// Constructs a new [`ShellPacket`].
    pub fn new(id: ShellPacketId, data: Vec<u8>) -> Self {
        Self { id, data }
    }

    /// Constructs a [`ShellPacketId::Exit`] packet with a 1-byte exit code payload.
    ///
    /// Ports exit code packet creation from `original/shell_service_protocol_test.cpp`.
    pub fn exit_code(code: u8) -> Self {
        Self {
            id: ShellPacketId::Exit,
            data: vec![code],
        }
    }

    /// Parses the exit status code from an exit packet.
    pub fn parse_exit_code(&self) -> Result<u8, ShellDecodeError> {
        if self.id != ShellPacketId::Exit {
            return Err(ShellDecodeError::InvalidPayload(format!(
                "expected Exit packet, got {:?}",
                self.id
            )));
        }
        if self.data.is_empty() {
            return Err(ShellDecodeError::InvalidPayload(
                "empty exit code payload".to_string(),
            ));
        }
        Ok(self.data[0])
    }

    /// Constructs a [`ShellPacketId::WindowSizeChange`] packet formatted as human-readable ASCII (`rowsxcols,xpixelsxypixels\0`).
    ///
    /// Ports formatting from `original/client/commandline.cpp`:
    /// `snprintf(shell->data(), shell->data_capacity(), "%dx%d,%dx%d", ws_row, ws_col, ws_xpixel, ws_ypixel)`
    pub fn window_size_change(ws: WindowSize) -> Self {
        let text = format!("{}x{},{}x{}\0", ws.rows, ws.cols, ws.x_pixels, ws.y_pixels);
        Self {
            id: ShellPacketId::WindowSizeChange,
            data: text.into_bytes(),
        }
    }

    /// Parses a window size change payload formatted as ASCII text (`rowsxcols,xpixelsxypixels`).
    ///
    /// Ports parsing from `original/daemon/shell_service.cpp`:
    /// `sscanf(input_->data(), "%dx%d,%dx%d", &rows, &cols, &x_pixels, &y_pixels)`
    pub fn parse_window_size(&self) -> Result<WindowSize, ShellDecodeError> {
        if self.id != ShellPacketId::WindowSizeChange {
            return Err(ShellDecodeError::InvalidPayload(format!(
                "expected WindowSizeChange packet, got {:?}",
                self.id
            )));
        }

        let s = match std::str::from_utf8(&self.data) {
            Ok(s) => s.trim_end_matches('\0'),
            Err(_) => {
                return Err(ShellDecodeError::InvalidPayload(
                    "window size payload is not UTF-8 text".to_string(),
                ))
            }
        };

        // Parse "rowsxcols,xpixelsxypixels"
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 2 {
            return Err(ShellDecodeError::InvalidPayload(format!(
                "invalid window size string format: '{s}'"
            )));
        }

        let dim: Vec<&str> = parts[0].split('x').collect();
        let pix: Vec<&str> = parts[1].split('x').collect();

        if dim.len() != 2 || pix.len() != 2 {
            return Err(ShellDecodeError::InvalidPayload(format!(
                "invalid window size dimension format: '{s}'"
            )));
        }

        let rows_str = dim[0];
        let cols_str = dim[1];
        let x_pix_str = pix[0];
        let y_pix_str = pix[1];

        let rows = rows_str.parse::<u16>().map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse rows '{rows_str}': {e}"))
        })?;
        let cols = cols_str.parse::<u16>().map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse cols '{cols_str}': {e}"))
        })?;
        let x_pixels = x_pix_str.parse::<u16>().map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse x_pixels '{x_pix_str}': {e}"))
        })?;
        let y_pixels = y_pix_str.parse::<u16>().map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse y_pixels '{y_pix_str}': {e}"))
        })?;

        Ok(WindowSize {
            rows,
            cols,
            x_pixels,
            y_pixels,
        })
    }

    /// Encodes packet to a `Vec<u8>`, returning [`ShellEncodeError::PayloadTooLarge`] if `data.len() > MAX_PAYLOAD`.
    pub fn encode(&self) -> Result<Vec<u8>, ShellEncodeError> {
        if self.data.len() > MAX_PAYLOAD {
            return Err(ShellEncodeError::PayloadTooLarge {
                length: self.data.len(),
                max: MAX_PAYLOAD,
            });
        }
        let mut buf = Vec::with_capacity(Self::HEADER_SIZE + self.data.len());
        buf.push(self.id as u8);
        buf.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.data);
        Ok(buf)
    }
}

/// Incremental decoder for streaming Shell v2 packets.
///
/// Accepts arbitrary chunk boundaries via [`feed`](Self::feed) and yields complete packets via [`pop_packet`](Self::pop_packet).
/// Ports stream decoding logic from `original/shell_service_protocol.cpp`.
#[derive(Debug, Default)]
pub struct ShellDecoder {
    buffer: Vec<u8>,
    read_pos: usize,
    error: Option<ShellDecodeError>,
}

impl ShellDecoder {
    /// Creates a new, empty [`ShellDecoder`].
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            read_pos: 0,
            error: None,
        }
    }

    /// Feeds incoming byte data into the decoder buffer.
    pub fn feed(&mut self, chunk: &[u8]) {
        if self.read_pos > 0 && self.read_pos == self.buffer.len() {
            self.buffer.clear();
            self.read_pos = 0;
        } else if self.read_pos > 64 * 1024 && self.read_pos * 2 >= self.buffer.len() {
            self.buffer.drain(..self.read_pos);
            self.read_pos = 0;
        }
        self.buffer.extend_from_slice(chunk);
    }

    /// Attempts to extract the next complete [`ShellPacket`] from buffered data.
    ///
    /// Returns `Ok(Some(packet))` if a full packet was parsed.
    /// Returns `Ok(None)` if more input bytes are needed.
    /// Returns `Err(ShellDecodeError)` if the packet header is invalid or payload size exceeds [`MAX_PAYLOAD`].
    /// Sticky: once an error occurs, subsequent calls return the same error until [`clear`](Self::clear) is called.
    pub fn pop_packet(&mut self) -> Result<Option<ShellPacket>, ShellDecodeError> {
        if let Some(ref err) = self.error {
            return Err(err.clone());
        }

        let unread = &self.buffer[self.read_pos..];
        if unread.len() < ShellPacket::HEADER_SIZE {
            return Ok(None);
        }

        let id_byte = unread[0];
        let payload_len = u32::from_le_bytes(unread[1..5].try_into().unwrap()) as usize;

        if payload_len > MAX_PAYLOAD {
            let err = ShellDecodeError::PayloadTooLarge {
                length: payload_len,
                max: MAX_PAYLOAD,
            };
            self.error = Some(err.clone());
            return Err(err);
        }

        let packet_id = match ShellPacketId::try_from(id_byte) {
            Ok(id) => id,
            Err(id) => {
                let err = ShellDecodeError::UnknownPacketId(id);
                self.error = Some(err.clone());
                return Err(err);
            }
        };

        let total_packet_size = ShellPacket::HEADER_SIZE + payload_len;
        if unread.len() < total_packet_size {
            return Ok(None);
        }

        // Complete packet is buffered! Copy payload using a contiguous slice copy.
        let payload_start = self.read_pos + ShellPacket::HEADER_SIZE;
        let payload_end = payload_start + payload_len;
        let data = self.buffer[payload_start..payload_end].to_vec();

        self.read_pos += total_packet_size;

        if self.read_pos == self.buffer.len() {
            self.buffer.clear();
            self.read_pos = 0;
        }

        Ok(Some(ShellPacket {
            id: packet_id,
            data,
        }))
    }

    /// Clears the decoder buffer and any recorded sticky error.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.read_pos = 0;
        self.error = None;
    }
}
