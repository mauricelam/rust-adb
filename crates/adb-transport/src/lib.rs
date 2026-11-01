//! adb-transport: Connection and transport interfaces for USB, TCP/IP, and mDNS.

pub mod handshake;
pub mod transport;

pub use handshake::{perform_cnxn_handshake, HandshakeResult};
pub use transport::{
    calculate_checksum, read_packet_from_stream, write_packet_to_stream, AsyncStreamTransport,
    Packet, Transport, TransportError,
};
