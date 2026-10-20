//! Message types, serialization, shell v2 framing, smart-socket framing, and the sync protocol. Pure code, no I/O.

pub mod header;
pub mod shell;
pub mod smart_socket;
 
pub use header::{Command, DecodeError, EncodeError, Header};
pub use shell::{
    ShellDecodeError, ShellDecoder, ShellEncodeError, ShellPacket, ShellPacketId, WindowSize,
    MAX_PAYLOAD,
};
pub use smart_socket::{
    encode_request, parse_protocol_string, parse_reply, ProtocolString, SmartSocketError,
    SmartSocketReply,
};
