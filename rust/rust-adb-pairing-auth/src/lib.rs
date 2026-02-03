//! A Rust port of the ADB pairing authentication library.
//!
//! This crate provides a Rust implementation of the ADB pairing authentication
//! protocol, which is based on SPAKE2. It is a port of the C++ implementation
//! in `original/pairing_auth`.

pub mod aes_128_gcm;

use self::aes_128_gcm::{Aes128GcmCipher, Aes128GcmError};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use thiserror::Error;

const CLIENT_NAME: &[u8] = b"adb pair client";
const SERVER_NAME: &[u8] = b"adb pair server";

/// Error type for the pairing authentication process.
#[derive(Debug, Error)]
pub enum PairingAuthError {
    /// An error occurred in the SPAKE2 protocol.
    #[error("SPAKE2 error")]
    Spake2Error,
    /// An error occurred in the AES-128-GCM cipher.
    #[error("Cipher error")]
    CipherError(#[from] Aes128GcmError),
    /// The password was empty.
    #[error("Password cannot be empty")]
    PasswordEmpty,
}

impl From<spake2::Error> for PairingAuthError {
    fn from(_: spake2::Error) -> Self {
        PairingAuthError::Spake2Error
    }
}

/// A builder for the `PairingAuthCtx`. This is used to create a new pairing
/// context and initialize the cipher.
///
/// This is a port of the C++ implementation in
/// `original/pairing_auth/pairing_auth.cpp`.
///
/// The Rust API has been changed to use a builder pattern for better type
/// safety. This ensures that the `PairingAuthCtx` can only be created after the
/// cipher has been successfully initialized.
pub struct PairingAuthCtxBuilder {
    state: Spake2<Ed25519Group>,
    our_msg: Vec<u8>,
}

/// The role of the pairing participant.
pub enum Role {
    /// The client role.
    Client,
    /// The server role.
    Server,
}

impl PairingAuthCtxBuilder {
    /// Creates a new `PairingAuthCtxBuilder`.
    ///
    /// # Arguments
    ///
    /// * `pswd` - The shared password.
    /// * `role` - The role of this participant.
    pub fn new(pswd: &[u8], role: Role) -> Result<Self, PairingAuthError> {
        if pswd.is_empty() {
            return Err(PairingAuthError::PasswordEmpty);
        }

        let password = Password::new(pswd);
        let client_id = Identity::new(CLIENT_NAME);
        let server_id = Identity::new(SERVER_NAME);

        let (state, our_msg) = match role {
            Role::Client => Spake2::<Ed25519Group>::start_a(&password, &client_id, &server_id),
            Role::Server => Spake2::<Ed25519Group>::start_b(&password, &client_id, &server_id),
        };

        Ok(Self {
            state,
            our_msg: our_msg.to_vec(),
        })
    }

    /// Returns the message to be sent to the other party.
    pub fn msg(&self) -> &[u8] {
        &self.our_msg
    }

    /// Initializes the cipher with the other party's message and returns a
    /// `PairingAuthCtx`.
    ///
    /// # Arguments
    ///
    /// * `their_msg` - The message received from the other party.
    pub fn init_cipher(self, their_msg: &[u8]) -> Result<PairingAuthCtx, PairingAuthError> {
        let key_material = self.state.finish(their_msg)?;
        let cipher = Aes128GcmCipher::new(&key_material)?;
        Ok(PairingAuthCtx { cipher })
    }
}

/// A pairing authentication context. This is used to encrypt and decrypt
/// messages after the cipher has been initialized.
pub struct PairingAuthCtx {
    cipher: Aes128GcmCipher,
}

impl PairingAuthCtx {
    /// Encrypts the given data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to encrypt.
    pub fn encrypt(&mut self, data: &[u8]) -> Result<Vec<u8>, PairingAuthError> {
        Ok(self.cipher.encrypt(data)?)
    }

    /// Decrypts the given data.
    ///
    /// # Arguments
    ///
    /// * `data` - The data to decrypt.
    pub fn decrypt(&mut self, data: &[u8]) -> Result<Vec<u8>, PairingAuthError> {
        Ok(self.cipher.decrypt(data)?)
    }
}
