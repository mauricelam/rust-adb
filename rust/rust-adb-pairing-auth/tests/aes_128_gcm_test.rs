use rust_adb_pairing_auth::aes_128_gcm::{Aes128GcmCipher, Aes128GcmError};

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
