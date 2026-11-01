use crate::transport::{
    read_packet_from_stream, write_packet_to_stream, Packet, Transport, TransportError,
};
use std::net::SocketAddr;
use tokio::net::{TcpStream, ToSocketAddrs};

/// TCP-backed ADB transport wrapping a [`TcpStream`].
///
/// Disables Nagle's algorithm (`set_nodelay(true)`) matching C++ ADB's `disable_tcp_nagle()`.
#[derive(Debug)]
pub struct TcpTransport {
    stream: TcpStream,
    skip_checksum: bool,
}

impl TcpTransport {
    /// Connects to a remote address over TCP, disables Nagle's algorithm, and constructs a [`TcpTransport`].
    pub async fn connect<A: ToSocketAddrs>(addr: A) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(addr).await?;
        Ok(Self::from_stream(stream))
    }

    /// Constructs a [`TcpTransport`] from an established [`TcpStream`].
    ///
    /// Configures `TCP_NODELAY` on the stream if supported.
    pub fn from_stream(stream: TcpStream) -> Self {
        let _ = stream.set_nodelay(true);
        Self {
            stream,
            skip_checksum: false,
        }
    }

    /// Configures whether packet payload checksum calculation/verification should be skipped.
    pub fn set_skip_checksum(&mut self, skip_checksum: bool) {
        self.skip_checksum = skip_checksum;
    }

    /// Returns the local socket address of the underlying TCP stream, if available.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.stream.local_addr()?)
    }

    /// Returns the peer remote socket address of the underlying TCP stream, if available.
    pub fn peer_addr(&self) -> Result<SocketAddr, TransportError> {
        Ok(self.stream.peer_addr()?)
    }

    /// Unwraps and returns the underlying [`TcpStream`].
    pub fn into_inner(self) -> TcpStream {
        self.stream
    }
}

impl Transport for TcpTransport {
    async fn read_packet(&mut self) -> Result<Packet, TransportError> {
        read_packet_from_stream(&mut self.stream, self.skip_checksum).await
    }

    async fn write_packet(&mut self, packet: &Packet) -> Result<(), TransportError> {
        write_packet_to_stream(&mut self.stream, packet).await
    }
}
