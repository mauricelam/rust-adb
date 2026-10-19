//! Message types, serialization, shell v2 framing and the sync protocol. Pure code, no I/O.

pub mod header;

pub use header::{Command, DecodeError, EncodeError, Header};
