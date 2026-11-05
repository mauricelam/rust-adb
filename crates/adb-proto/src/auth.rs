//! AUTH exchange sub-command constants and types.
//!
//! Ported from `original/adb_auth.h`.

use std::fmt;

/// Expected size of an authentication token challenge in bytes.
pub const TOKEN_SIZE: usize = 20;

/// Token packet sent by device requesting authentication signature or public key.
pub const ADB_AUTH_TOKEN: u32 = 1;

/// Signature packet sent by host in response to an authentication token challenge.
pub const ADB_AUTH_SIGNATURE: u32 = 2;

/// Public key packet sent by host when all private keys have been tried.
pub const ADB_AUTH_RSAPUBLICKEY: u32 = 3;

/// Authentication sub-command type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum AuthType {
    /// Token packet sent by device requesting signature or public key.
    Token = ADB_AUTH_TOKEN,
    /// Signature packet sent by host in response to token challenge.
    Signature = ADB_AUTH_SIGNATURE,
    /// Public key packet sent by host.
    RsaPublicKey = ADB_AUTH_RSAPUBLICKEY,
}

impl TryFrom<u32> for AuthType {
    type Error = u32;

    fn try_from(val: u32) -> Result<Self, Self::Error> {
        match val {
            ADB_AUTH_TOKEN => Ok(AuthType::Token),
            ADB_AUTH_SIGNATURE => Ok(AuthType::Signature),
            ADB_AUTH_RSAPUBLICKEY => Ok(AuthType::RsaPublicKey),
            other => Err(other),
        }
    }
}

/// A wrapper around a public key line string (e.g., `"<base64> <user>@<host>"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublicKeyLine(String);

impl PublicKeyLine {
    /// Creates a new `PublicKeyLine`.
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Returns the public key string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the inner `String`.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for PublicKeyLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for PublicKeyLine {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for PublicKeyLine {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for PublicKeyLine {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// High-level authentication message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMessage {
    /// Token challenge with fixed `TOKEN_SIZE` (20 bytes).
    Token([u8; TOKEN_SIZE]),
    /// RSA signature response bytes.
    Signature(Vec<u8>),
    /// RSA public key line.
    PublicKey(PublicKeyLine),
}
