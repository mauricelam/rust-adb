pub mod aes_128_gcm;

use aes_128_gcm::{Aes128GcmCipher, Aes128GcmError};
use spake2::{Ed25519Group, Identity, Password, Spake2};

const CLIENT_NAME: &[u8] = b"adb pair client";
const SERVER_NAME: &[u8] = b"adb pair server";

#[derive(Debug)]
pub enum PairingAuthError {
    Spake2Error(spake2::Error),
    CipherError(Aes128GcmError),
    PasswordEmpty,
    CipherNotInitialized,
    AlreadyInitialized,
}

impl From<spake2::Error> for PairingAuthError {
    fn from(e: spake2::Error) -> Self {
        PairingAuthError::Spake2Error(e)
    }
}

impl From<Aes128GcmError> for PairingAuthError {
    fn from(e: Aes128GcmError) -> Self {
        PairingAuthError::CipherError(e)
    }
}

pub struct PairingAuthCtx {
    state: Option<Spake2<Ed25519Group>>,
    our_msg: Vec<u8>,
    cipher: Option<Aes128GcmCipher>,
}

pub enum Role {
    Client,
    Server,
}

impl PairingAuthCtx {
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
            state: Some(state),
            our_msg: our_msg.to_vec(),
            cipher: None,
        })
    }

    pub fn msg(&self) -> &[u8] {
        &self.our_msg
    }

    pub fn init_cipher(&mut self, their_msg: &[u8]) -> Result<(), PairingAuthError> {
        let state = self
            .state
            .take()
            .ok_or(PairingAuthError::AlreadyInitialized)?;
        let key_material = state.finish(their_msg)?;
        let cipher = Aes128GcmCipher::new(&key_material)?;
        self.cipher = Some(cipher);
        Ok(())
    }

    pub fn encrypt(&mut self, data: &[u8]) -> Result<Vec<u8>, PairingAuthError> {
        self.cipher
            .as_mut()
            .ok_or(PairingAuthError::CipherNotInitialized)?
            .encrypt(data)
            .map_err(PairingAuthError::from)
    }

    pub fn decrypt(&mut self, data: &[u8]) -> Result<Vec<u8>, PairingAuthError> {
        self.cipher
            .as_mut()
            .ok_or(PairingAuthError::CipherNotInitialized)?
            .decrypt(data)
            .map_err(PairingAuthError::from)
    }
}
