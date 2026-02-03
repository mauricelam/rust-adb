use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Key, Nonce};
use hkdf::{Hkdf, InvalidLength};
use sha2::Sha256;
use thiserror::Error;

const HKDF_KEY_LENGTH: usize = 16;
const INFO: &[u8] = b"adb pairing_auth aes-128-gcm key";

#[derive(Debug, Error)]
pub enum Aes128GcmError {
    #[error("Key material cannot be empty.")]
    KeyMaterialEmpty,
    #[error("Invalid length for HKDF")]
    HkdfInvalidLength,
    #[error("Encryption failed")]
    EncryptionFailed,
    #[error("Decryption failed")]
    DecryptionFailed,
}

impl From<InvalidLength> for Aes128GcmError {
    fn from(_: InvalidLength) -> Self {
        Aes128GcmError::HkdfInvalidLength
    }
}

/// A cipher for encrypting and decrypting data using AES-128-GCM.
/// This is a port of the C++ implementation in `original/pairing_auth/aes_128_gcm.cpp`.
pub struct Aes128GcmCipher {
    key: Key<Aes128Gcm>,
    enc_sequence: u64,
    dec_sequence: u64,
}

impl Aes128GcmCipher {
    /// Creates a new `Aes128GcmCipher` from the given key material.
    pub fn new(key_material: &[u8]) -> Result<Self, Aes128GcmError> {
        if key_material.is_empty() {
            return Err(Aes128GcmError::KeyMaterialEmpty);
        }

        let hkdf = Hkdf::<Sha256>::new(None, key_material);
        let mut okm = [0u8; HKDF_KEY_LENGTH];
        hkdf.expand(INFO, &mut okm)?;

        Ok(Self {
            key: *Key::<Aes128Gcm>::from_slice(&okm),
            enc_sequence: 0,
            dec_sequence: 0,
        })
    }

    /// Encrypt a block of data.
    ///
    /// This consumes all data in `data` and returns the encrypted data. The
    /// data contains information needed for decryption that is specific to
    /// this implementation and is therefore only suitable for decryption with
    /// this class.
    pub fn encrypt(&mut self, data: &[u8]) -> Result<Vec<u8>, Aes128GcmError> {
        let cipher = Aes128Gcm::new(&self.key);
        // AES-128 nonce is 12 bytes
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&self.enc_sequence.to_le_bytes());
        let nonce = Nonce::from(nonce_bytes);

        let result = cipher
            .encrypt(&nonce, data)
            .map_err(|_| Aes128GcmError::EncryptionFailed)?;
        self.enc_sequence += 1;
        Ok(result)
    }

    /// Decrypt a block of data.
    ///
    /// This consumes all data in `data` and returns the decrypted data.
    pub fn decrypt(&mut self, data: &[u8]) -> Result<Vec<u8>, Aes128GcmError> {
        let cipher = Aes128Gcm::new(&self.key);
        let mut nonce_bytes = [0u8; 12];
        nonce_bytes[..8].copy_from_slice(&self.dec_sequence.to_le_bytes());
        let nonce = Nonce::from(nonce_bytes);

        let result = cipher
            .decrypt(&nonce, data)
            .map_err(|_| Aes128GcmError::DecryptionFailed)?;
        self.dec_sequence += 1;
        Ok(result)
    }
}
