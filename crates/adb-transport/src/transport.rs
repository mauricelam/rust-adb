use adb_proto::{Command, DecodeError, Header};
use std::fmt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Calculates ADB payload checksum (sum of unsigned bytes).
pub fn calculate_checksum(payload: &[u8]) -> u32 {
    payload.iter().map(|&b| b as u32).sum()
}

/// Errors that can occur during packet transmission or transport operations.
#[derive(Debug)]
pub enum TransportError {
    /// Standard I/O error.
    Io(std::io::Error),
    /// Error decoding header.
    Decode(DecodeError),
    /// Header payload CRC checksum mismatch.
    ChecksumMismatch { expected: u32, actual: u32 },
    /// Unexpected packet command received.
    UnexpectedCommand(Command),
    /// Payload size exceeds negotiated max payload.
    PayloadTooLarge { max: usize, actual: usize },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Decode(err) => write!(f, "header decode error: {err}"),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "checksum mismatch: expected 0x{expected:08x}, got 0x{actual:08x}"
            ),
            Self::UnexpectedCommand(cmd) => write!(f, "unexpected command: {cmd:?}"),
            Self::PayloadTooLarge { max, actual } => {
                write!(f, "payload too large: max {max} bytes, got {actual}")
            }
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Decode(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for TransportError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<DecodeError> for TransportError {
    fn from(err: DecodeError) -> Self {
        Self::Decode(err)
    }
}

/// Represents a complete ADB wire packet containing a header and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub header: Header,
    pub payload: Vec<u8>,
}

impl Packet {
    /// Constructs a new [`Packet`]. Computes checksum using [`calculate_checksum`]
    /// unless `skip_checksum` is `true`.
    pub fn new(
        command: Command,
        arg0: u32,
        arg1: u32,
        payload: Vec<u8>,
        skip_checksum: bool,
    ) -> Self {
        let data_crc32 = if skip_checksum {
            0
        } else {
            calculate_checksum(&payload)
        };
        let header = Header::new(command, arg0, arg1, payload.len() as u32, data_crc32);
        Self { header, payload }
    }
}

/// Async transport trait providing packet reading and writing primitives.
pub trait Transport: Send + Sync {
    /// Reads a single packet from the transport.
    fn read_packet(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Packet, TransportError>> + Send;

    /// Writes a single packet to the transport.
    fn write_packet(
        &mut self,
        packet: &Packet,
    ) -> impl std::future::Future<Output = Result<(), TransportError>> + Send;
}

/// Extension or wrapper for streams implementing `AsyncRead` and `AsyncWrite`.
pub async fn read_packet_from_stream<R: AsyncRead + Unpin + Send>(
    stream: &mut R,
    skip_checksum: bool,
) -> Result<Packet, TransportError> {
    let mut header_buf = [0u8; Header::SIZE];
    stream.read_exact(&mut header_buf).await?;
    let header = Header::decode(&header_buf)?;

    let mut payload = vec![0u8; header.data_length as usize];
    if header.data_length > 0 {
        stream.read_exact(&mut payload).await?;
    }

    if !skip_checksum && header.data_crc32 != 0 {
        let actual_checksum = calculate_checksum(&payload);
        if actual_checksum != header.data_crc32 {
            return Err(TransportError::ChecksumMismatch {
                expected: header.data_crc32,
                actual: actual_checksum,
            });
        }
    }

    Ok(Packet { header, payload })
}

pub async fn write_packet_to_stream<W: AsyncWrite + Unpin + Send>(
    stream: &mut W,
    packet: &Packet,
) -> Result<(), TransportError> {
    let header_buf = packet.header.encode();
    stream.write_all(&header_buf).await?;
    if !packet.payload.is_empty() {
        stream.write_all(&packet.payload).await?;
    }
    stream.flush().await?;
    Ok(())
}

/// Blanked implementation of [`Transport`] for framing over `AsyncRead + AsyncWrite`.
#[derive(Debug)]
pub struct AsyncStreamTransport<S> {
    stream: S,
    skip_checksum: bool,
}

impl<S> AsyncStreamTransport<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            skip_checksum: false,
        }
    }

    pub fn set_skip_checksum(&mut self, skip_checksum: bool) {
        self.skip_checksum = skip_checksum;
    }

    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + Sync> Transport for AsyncStreamTransport<S> {
    async fn read_packet(&mut self) -> Result<Packet, TransportError> {
        read_packet_from_stream(&mut self.stream, self.skip_checksum).await
    }

    async fn write_packet(&mut self, packet: &Packet) -> Result<(), TransportError> {
        write_packet_to_stream(&mut self.stream, packet).await
    }
}
