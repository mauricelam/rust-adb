//! adb-proto: Message types, serialization, shell v2 framing, smart-socket framing, connection banners, features, and the sync protocol. Pure code, no I/O.

pub mod auth;
pub mod banner;
pub mod header;
pub mod limits;
pub mod message;
pub mod service;
pub mod shell;
pub mod smart_socket;
pub mod socket_spec;
pub mod state;
pub mod sync;
pub mod transport_selector;
pub mod types;

pub use auth::{
    AuthMessage, AuthType, PublicKeyLine, ADB_AUTH_RSAPUBLICKEY, ADB_AUTH_SIGNATURE,
    ADB_AUTH_TOKEN, TOKEN_SIZE,
};
pub use banner::{
    client_supported_features, negotiate_max_payload, negotiate_version,
    server_advertised_features, Banner, BannerEntry, Feature, FeatureOptions, Features,
};
pub use header::{ChecksumMode, Command, DecodeError, EncodeError, Header};
pub use limits::{
    A_VERSION, A_VERSION_MIN, A_VERSION_SKIP_CHECKSUM, INITIAL_DELAYED_ACK_BYTES, MAX_PAYLOAD,
    MAX_PAYLOAD_V1,
};
pub use message::{Message, MessageDecodeError};
pub use service::{HostService, Service, TrackDevicesMode};
pub use shell::{
    ShellDecodeError, ShellDecoder, ShellEncodeError, ShellPacket, ShellPacketId, WindowSize,
};
pub use smart_socket::{
    encode_request, parse_protocol_string, parse_reply, ProtocolString, SmartSocketError,
    SmartSocketReply,
};
pub use socket_spec::{ParseSocketSpecError, SocketSpec};
pub use state::{ConnectionState, ParseConnectionStateError};
pub use sync::{
    SyncDecodeError, SyncDecoder, SyncDecoderMode, SyncEncodeError, SyncMessage, ID_DATA,
    ID_DENT_V1, ID_DONE, ID_FAIL, ID_LIST_V1, ID_LSTAT_V1, ID_OKAY, ID_QUIT, ID_RECV_V1,
    ID_SEND_V1, SYNC_DATA_MAX, SYNC_PATH_MAX,
};
pub use transport_selector::{format_host_command, TransportSelector};
pub use types::{
    AdbPort, ConsolePort, IdError, LocalId, MaxPayload, ProtocolVersion, RemoteId, ServerVersion,
    TransportId, ADB_SERVER_VERSION, SERVER_VERSION,
};
