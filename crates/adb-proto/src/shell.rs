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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowSize {
    pub rows: u16,
    pub cols: u16,
    pub x_pixels: u16,
    pub y_pixels: u16,
}

impl WindowSize {
    /// Constructs a new `WindowSize`.
    pub const fn new(rows: u16, cols: u16, x_pixels: u16, y_pixels: u16) -> Self {
        Self {
            rows,
            cols,
            x_pixels,
            y_pixels,
        }
    }

    /// Encodes window size as ASCII text (`rowsxcols,xpixelsxypixels\0`).
    pub fn encode_to_bytes(&self) -> Vec<u8> {
        format!(
            "{}x{},{}x{}\0",
            self.rows, self.cols, self.x_pixels, self.y_pixels
        )
        .into_bytes()
    }

    /// Parses a window size change payload formatted as ASCII text (`rowsxcols,xpixelsxypixels`).
    ///
    /// Ports parsing from `original/daemon/shell_service.cpp`:
    /// `sscanf(input_->data(), "%dx%d,%dx%d", &rows, &cols, &x_pixels, &y_pixels)`
    pub fn parse(bytes: &[u8]) -> Result<Self, ShellDecodeError> {
        let s = match std::str::from_utf8(bytes) {
            Ok(s) => s.trim_end_matches('\0'),
            Err(_) => {
                return Err(ShellDecodeError::InvalidPayload(
                    "window size payload is not UTF-8 text".to_string(),
                ))
            }
        };

        // Lenient sscanf-like parsing: find first comma, split each by 'x'
        let (first, second) = s.split_once(',').ok_or_else(|| {
            ShellDecodeError::InvalidPayload(format!("invalid window size string format: '{s}'"))
        })?;

        let (rows_str, cols_str) = first.split_once('x').ok_or_else(|| {
            ShellDecodeError::InvalidPayload(format!("invalid window size dimension format: '{s}'"))
        })?;

        let (x_pix_str, y_pix_str) = second.split_once('x').ok_or_else(|| {
            ShellDecodeError::InvalidPayload(format!("invalid window size dimension format: '{s}'"))
        })?;

        let rows = parse_int_prefix(rows_str).map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse rows '{rows_str}': {e}"))
        })?;
        let cols = parse_int_prefix(cols_str).map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse cols '{cols_str}': {e}"))
        })?;
        let x_pixels = parse_int_prefix(x_pix_str).map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse x_pixels '{x_pix_str}': {e}"))
        })?;
        let y_pixels = parse_int_prefix(y_pix_str).map_err(|e| {
            ShellDecodeError::InvalidPayload(format!("failed to parse y_pixels '{y_pix_str}': {e}"))
        })?;

        Ok(Self {
            rows,
            cols,
            x_pixels,
            y_pixels,
        })
    }
}

fn parse_int_prefix(s: &str) -> Result<u16, std::num::ParseIntError> {
    // Like sscanf %d, parse leading digits
    let trimmed = s.trim_start();
    let end = trimmed
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(idx, _)| idx)
        .unwrap_or(trimmed.len());
    trimmed[..end].parse::<u16>()
}

/// Strongly typed Shell v2 message representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellMessage {
    /// Subprocess standard input data (`kIdStdin = 0`).
    Stdin(Vec<u8>),
    /// Subprocess standard output data (`kIdStdout = 1`).
    Stdout(Vec<u8>),
    /// Subprocess standard error data (`kIdStderr = 2`).
    Stderr(Vec<u8>),
    /// Subprocess exit status code (`kIdExit = 3`).
    Exit(u8),
    /// Close subprocess standard input (`kIdCloseStdin = 4`).
    CloseStdin,
    /// Window size change event (`kIdWindowSizeChange = 5`).
    WindowSize(WindowSize),
}

impl ShellMessage {
    /// Header size in bytes: 1 byte ID + 4 bytes LE uint32 length.
    pub const HEADER_SIZE: usize = 5;

    /// Returns the packet ID byte for this message.
    pub fn id(&self) -> ShellPacketId {
        match self {
            ShellMessage::Stdin(_) => ShellPacketId::Stdin,
            ShellMessage::Stdout(_) => ShellPacketId::Stdout,
            ShellMessage::Stderr(_) => ShellPacketId::Stderr,
            ShellMessage::Exit(_) => ShellPacketId::Exit,
            ShellMessage::CloseStdin => ShellPacketId::CloseStdin,
            ShellMessage::WindowSize(_) => ShellPacketId::WindowSizeChange,
        }
    }

    /// Encodes this message into a Shell v2 packet byte vector.
    pub fn encode(&self) -> Result<Vec<u8>, ShellEncodeError> {
        let (id, payload) = match self {
            ShellMessage::Stdin(d) => (ShellPacketId::Stdin as u8, d.as_slice()),
            ShellMessage::Stdout(d) => (ShellPacketId::Stdout as u8, d.as_slice()),
            ShellMessage::Stderr(d) => (ShellPacketId::Stderr as u8, d.as_slice()),
            ShellMessage::Exit(code) => (ShellPacketId::Exit as u8, std::slice::from_ref(code)),
            ShellMessage::CloseStdin => (ShellPacketId::CloseStdin as u8, &[][..]),
            ShellMessage::WindowSize(ws) => {
                let bytes = ws.encode_to_bytes();
                let mut buf = Vec::with_capacity(Self::HEADER_SIZE + bytes.len());
                buf.push(ShellPacketId::WindowSizeChange as u8);
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(&bytes);
                return Ok(buf);
            }
        };

        if payload.len() > MAX_PAYLOAD {
            return Err(ShellEncodeError::PayloadTooLarge {
                length: payload.len(),
                max: MAX_PAYLOAD,
            });
        }

        let mut buf = Vec::with_capacity(Self::HEADER_SIZE + payload.len());
        buf.push(id);
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(payload);
        Ok(buf)
    }

    /// Parses a raw [`ShellPacket`] into a typed [`ShellMessage`].
    pub fn from_packet(packet: ShellPacket) -> Result<Self, ShellDecodeError> {
        match packet.id {
            ShellPacketId::Stdin => Ok(ShellMessage::Stdin(packet.data)),
            ShellPacketId::Stdout => Ok(ShellMessage::Stdout(packet.data)),
            ShellPacketId::Stderr => Ok(ShellMessage::Stderr(packet.data)),
            ShellPacketId::Exit => {
                if packet.data.is_empty() {
                    return Err(ShellDecodeError::InvalidPayload(
                        "empty exit code payload".to_string(),
                    ));
                }
                Ok(ShellMessage::Exit(packet.data[0]))
            }
            ShellPacketId::CloseStdin => Ok(ShellMessage::CloseStdin),
            ShellPacketId::WindowSizeChange => {
                let ws = WindowSize::parse(&packet.data)?;
                Ok(ShellMessage::WindowSize(ws))
            }
        }
    }
}

/// Represents a raw Shell v2 packet with a 1-byte packet ID and payload bytes.
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

    /// Constructs a [`ShellPacketId::WindowSizeChange`] packet.
    pub fn window_size_change(ws: WindowSize) -> Self {
        Self {
            id: ShellPacketId::WindowSizeChange,
            data: ws.encode_to_bytes(),
        }
    }

    /// Parses window dimensions from a window size change packet.
    pub fn parse_window_size(&self) -> Result<WindowSize, ShellDecodeError> {
        if self.id != ShellPacketId::WindowSizeChange {
            return Err(ShellDecodeError::InvalidPayload(format!(
                "expected WindowSizeChange packet, got {:?}",
                self.id
            )));
        }
        WindowSize::parse(&self.data)
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

/// Incremental decoder for streaming Shell v2 packets and messages.
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

    /// Attempts to extract the next complete typed [`ShellMessage`].
    pub fn pop_message(&mut self) -> Result<Option<ShellMessage>, ShellDecodeError> {
        match self.pop_packet()? {
            Some(pkt) => ShellMessage::from_packet(pkt).map(Some),
            None => Ok(None),
        }
    }

    /// Clears the decoder buffer and any recorded sticky error.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.read_pos = 0;
        self.error = None;
    }
}
