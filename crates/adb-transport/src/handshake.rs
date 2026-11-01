use crate::transport::{Packet, Transport, TransportError};
use adb_proto::{
    client_supported_features, negotiate_max_payload, negotiate_version, Banner, BannerEntry,
    Command, FeatureOptions, Features, A_VERSION, MAX_PAYLOAD,
};

/// Result of a successful CNXN protocol handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeResult {
    pub negotiated_version: u32,
    pub negotiated_max_payload: usize,
    pub remote_banner: Banner,
    pub remote_features: Features,
}

/// Executes the host-side `CNXN` handshake over the provided transport.
///
/// Sends local CNXN packet with host version (`A_VERSION`), max payload (`MAX_PAYLOAD`),
/// and banner formatted with supported client features. Receives and validates the
/// remote CNXN response packet, negotiating the protocol version, maximum payload limit,
/// and remote features.
///
/// Ports handshake logic from `send_connect` and `handle_new_connection` in `original/adb.cpp`
/// and `atransport::update_version` in `original/transport.cpp`.
pub async fn perform_cnxn_handshake<T: Transport + ?Sized>(
    transport: &mut T,
    options: &FeatureOptions,
) -> Result<HandshakeResult, TransportError> {
    let local_features = client_supported_features(options);
    let local_banner = Banner {
        system_type: "host".to_string(),
        serial: "".to_string(),
        entries: vec![BannerEntry::Features(local_features)],
        ends_with_semicolon: true,
    };

    let connection_str = local_banner.to_string();
    let local_payload = connection_str.into_bytes();

    // CNXN packets are sent before version negotiation is complete, so we skip checksum
    // calculation for outbound CNXN when running modern protocol versions (>= A_VERSION_SKIP_CHECKSUM).
    let send_packet = Packet::new(
        Command::Cnxn,
        A_VERSION,
        MAX_PAYLOAD as u32,
        local_payload,
        true,
    );

    transport.write_packet(&send_packet).await?;

    let response = transport.read_packet().await?;
    if response.header.command != Command::Cnxn {
        return Err(TransportError::UnexpectedCommand(response.header.command));
    }

    let remote_version = response.header.arg0;
    let remote_max_payload = response.header.arg1 as usize;

    let negotiated_version = negotiate_version(A_VERSION, remote_version);
    let negotiated_max_payload = negotiate_max_payload(MAX_PAYLOAD, remote_max_payload);

    let banner_str = String::from_utf8_lossy(&response.payload);
    let remote_banner = Banner::parse(&banner_str);
    let remote_features = remote_banner.features();

    Ok(HandshakeResult {
        negotiated_version,
        negotiated_max_payload,
        remote_banner,
        remote_features,
    })
}
