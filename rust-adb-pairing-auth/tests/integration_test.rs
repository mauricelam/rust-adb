use rust_adb_pairing_auth::aes_128_gcm::{Aes128GcmCipher, Aes128GcmError};
use rust_adb_pairing_auth::{PairingAuthCtx, PairingAuthError, Role};

#[test]
fn aes_128_gcm_init_empty_material() {
    let material = &[];
    let result = Aes128GcmCipher::new(material);
    assert!(matches!(result, Err(Aes128GcmError::KeyMaterialEmpty)));
}

#[test]
fn aes_128_gcm_encrypt_decrypt() {
    let msg = b"alice and bob, sitting in a binary tree";
    let material = b"test material";

    let mut alice = Aes128GcmCipher::new(material).unwrap();
    let mut bob = Aes128GcmCipher::new(material).unwrap();

    let encrypted = alice.encrypt(msg).unwrap();
    let decrypted = bob.decrypt(&encrypted).unwrap();

    assert_eq!(msg.to_vec(), decrypted);
}

#[test]
fn pairing_auth_empty_password() {
    let pswd = &[];
    let result = PairingAuthCtx::new(pswd, Role::Client);
    assert!(matches!(result, Err(PairingAuthError::PasswordEmpty)));
}

#[test]
fn pairing_auth_valid_password() {
    let pswd = b"password";
    let client = PairingAuthCtx::new(pswd, Role::Client).unwrap();
    let server = PairingAuthCtx::new(pswd, Role::Server).unwrap();

    assert!(!client.msg().is_empty());
    assert!(!server.msg().is_empty());
}

#[test]
fn pairing_auth_different_passwords() {
    let mut client = PairingAuthCtx::new(&[0x01, 0x02, 0x03], Role::Client).unwrap();
    let client_msg = client.msg().to_vec();

    let mut server = PairingAuthCtx::new(&[0x01, 0x02, 0x04], Role::Server).unwrap();
    let server_msg = server.msg().to_vec();

    client.init_cipher(&server_msg).unwrap();
    server.init_cipher(&client_msg).unwrap();

    let msg = &[0x2a, 0x2b, 0x2c];
    let encrypted = client.encrypt(msg).unwrap();
    let decrypted = server.decrypt(&encrypted);
    assert!(matches!(
        decrypted,
        Err(PairingAuthError::CipherError(
            Aes128GcmError::DecryptionFailed
        ))
    ));
}

#[test]
fn pairing_auth_same_passwords() {
    let pswd = &[0x4f, 0x5a, 0x01, 0x46];
    let mut client = PairingAuthCtx::new(pswd, Role::Client).unwrap();
    let client_msg = client.msg().to_vec();

    let mut server = PairingAuthCtx::new(pswd, Role::Server).unwrap();
    let server_msg = server.msg().to_vec();

    client.init_cipher(&server_msg).unwrap();
    server.init_cipher(&client_msg).unwrap();

    let msg = &[0x2a, 0x2b, 0x2c, 0xff, 0x45, 0x12, 0x33];

    // Client encrypts, server decrypts
    let encrypted = client.encrypt(msg).unwrap();
    let decrypted = server.decrypt(&encrypted).unwrap();
    assert_eq!(msg.to_vec(), decrypted);

    // Server encrypts, client decrypts
    let encrypted = server.encrypt(msg).unwrap();
    let decrypted = client.decrypt(&encrypted).unwrap();
    assert_eq!(msg.to_vec(), decrypted);
}
