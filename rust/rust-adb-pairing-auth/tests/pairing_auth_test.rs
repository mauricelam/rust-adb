use rust_adb_pairing_auth::aes_128_gcm::Aes128GcmError;
use rust_adb_pairing_auth::{PairingAuthCtxBuilder, PairingAuthError, Role};

#[test]
fn pairing_auth_empty_password() {
    let pswd = &[];
    let result = PairingAuthCtxBuilder::new(pswd, Role::Client);
    assert!(matches!(result, Err(PairingAuthError::PasswordEmpty)));
}

#[test]
fn pairing_auth_valid_password() {
    let pswd = b"password";
    let client = PairingAuthCtxBuilder::new(pswd, Role::Client).unwrap();
    let server = PairingAuthCtxBuilder::new(pswd, Role::Server).unwrap();

    assert!(!client.msg().is_empty());
    assert!(!server.msg().is_empty());
}

#[test]
fn pairing_auth_different_passwords() {
    let client_builder =
        PairingAuthCtxBuilder::new(&[0x01, 0x02, 0x03], Role::Client).unwrap();
    let client_msg = client_builder.msg().to_vec();

    let server_builder =
        PairingAuthCtxBuilder::new(&[0x01, 0x02, 0x04], Role::Server).unwrap();
    let server_msg = server_builder.msg().to_vec();

    let mut client = client_builder.init_cipher(&server_msg).unwrap();
    let mut server = server_builder.init_cipher(&client_msg).unwrap();

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
    let client_builder = PairingAuthCtxBuilder::new(pswd, Role::Client).unwrap();
    let client_msg = client_builder.msg().to_vec();

    let server_builder = PairingAuthCtxBuilder::new(pswd, Role::Server).unwrap();
    let server_msg = server_builder.msg().to_vec();

    let mut client = client_builder.init_cipher(&server_msg).unwrap();
    let mut server = server_builder.init_cipher(&client_msg).unwrap();

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

#[test]
fn pairing_auth_corrupted_payload() {
    let pswd = &[0x4f, 0x5a, 0x01, 0x46];
    let client_builder = PairingAuthCtxBuilder::new(pswd, Role::Client).unwrap();
    let client_msg = client_builder.msg().to_vec();

    let server_builder = PairingAuthCtxBuilder::new(pswd, Role::Server).unwrap();
    let server_msg = server_builder.msg().to_vec();

    let mut client = client_builder.init_cipher(&server_msg).unwrap();
    let mut server = server_builder.init_cipher(&client_msg).unwrap();

    let msg = &[
        0x2a, 0x2b, 0x2c, 0xff, 0x45, 0x12, 0x33, 0x45, 0x12, 0xea, 0xf2, 0xdb,
    ];

    // Client encrypts, server decrypts
    let encrypted = client.encrypt(msg).unwrap();
    let decrypted = server.decrypt(&encrypted).unwrap();
    assert_eq!(msg.to_vec(), decrypted);

    // Corrupt the payload by appending a byte
    let mut corrupted: Vec<u8> = encrypted.clone();
    corrupted.push(0xaa);
    let decrypted = server.decrypt(&corrupted);
    assert!(matches!(
        decrypted,
        Err(PairingAuthError::CipherError(
            Aes128GcmError::DecryptionFailed
        ))
    ));

    // Corrupt the payload by removing a byte
    let mut corrupted = encrypted;
    corrupted.pop();
    let decrypted = server.decrypt(&corrupted);
    assert!(matches!(
        decrypted,
        Err(PairingAuthError::CipherError(
            Aes128GcmError::DecryptionFailed
        ))
    ));
}
