//! AUTH exchange sub-command constants.
//!
//! Ported from `original/adb_auth.h`.

/// Token packet sent by device requesting authentication signature or public key.
pub const ADB_AUTH_TOKEN: u32 = 1;

/// Signature packet sent by host in response to an authentication token challenge.
pub const ADB_AUTH_SIGNATURE: u32 = 2;

/// Public key packet sent by host when all private keys have been tried.
pub const ADB_AUTH_RSAPUBLICKEY: u32 = 3;
