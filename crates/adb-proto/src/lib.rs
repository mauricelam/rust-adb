//! Message types, serialization, shell v2 framing and the sync protocol. Pure code, no I/O.

pub mod header;
pub mod shell;

pub use header::{Command, DecodeError, EncodeError, Header};
pub use shell::{
    ShellDecodeError, ShellDecoder, ShellEncodeError, ShellPacket, ShellPacketId, WindowSize,
    MAX_PAYLOAD,
};
