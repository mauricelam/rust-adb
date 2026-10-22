//! Message types, serialization, shell v2 framing, smart-socket framing, connection banners, features, and the sync protocol. Pure code, no I/O.

pub mod auth;
pub mod banner;
pub mod header;
pub mod shell;
pub mod smart_socket;
pub mod sync;

pub use auth::{ADB_AUTH_RSAPUBLICKEY, ADB_AUTH_SIGNATURE, ADB_AUTH_TOKEN};
pub use banner::{
    negotiate_max_payload, negotiate_version, supported_features, Banner, Feature, Features,
    A_VERSION, A_VERSION_MIN, A_VERSION_SKIP_CHECKSUM, MAX_PAYLOAD_V1,
};
pub use header::{Command, DecodeError, EncodeError, Header};
pub use shell::{
    ShellDecodeError, ShellDecoder, ShellEncodeError, ShellPacket, ShellPacketId, WindowSize,
    MAX_PAYLOAD,
};
pub use smart_socket::{
    encode_request, parse_protocol_string, parse_reply, ProtocolString, SmartSocketError,
    SmartSocketReply,
};
pub use sync::{
    SyncDecodeError, SyncDecoder, SyncDecoderMode, SyncEncodeError, SyncMessage, ID_DATA,
    ID_DENT_V1, ID_DONE, ID_FAIL, ID_LIST_V1, ID_LSTAT_V1, ID_OKAY, ID_QUIT, ID_RECV_V1,
    ID_SEND_V1, SYNC_DATA_MAX, SYNC_PATH_MAX,
};
