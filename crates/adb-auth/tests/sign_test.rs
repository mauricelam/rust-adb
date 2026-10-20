use adb_auth::{read_key_file, sign_token, SignError, TOKEN_SIZE};
use rsa::traits::PublicKeyParts;
use rsa::Pkcs1v15Sign;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn test_1_known_answer_vector() {
    let key_path = fixture_path("test_key");
    let token_path = fixture_path("token.bin");
    let sig_path = fixture_path("sig.bin");

    let priv_key = read_key_file(&key_path).expect("failed to load fixture private key");
    let token = std::fs::read(&token_path).expect("failed to load fixture token");
    let expected_sig = std::fs::read(&sig_path).expect("failed to load fixture signature");

    assert_eq!(token.len(), TOKEN_SIZE);
    let sig = sign_token(&priv_key, &token).expect("sign_token failed");
    assert_eq!(sig, expected_sig);
}

#[test]
fn test_2_verify_signature_with_public_key() {
    let key_path = fixture_path("test_key");
    let token_path = fixture_path("token.bin");

    let priv_key = read_key_file(&key_path).expect("failed to load fixture private key");
    let token = std::fs::read(&token_path).expect("failed to load fixture token");

    let sig = sign_token(&priv_key, &token).expect("sign_token failed");

    let pub_key = priv_key.to_public_key();
    let scheme = Pkcs1v15Sign::new::<sha1::Sha1>();

    pub_key
        .verify(scheme, &token, &sig)
        .expect("signature verification failed");
}

#[test]
fn test_3_signing_is_deterministic() {
    let key_path = fixture_path("test_key");
    let token = [42u8; 20];

    let priv_key = read_key_file(&key_path).expect("failed to load fixture private key");

    let sig1 = sign_token(&priv_key, &token).expect("sign_token 1 failed");
    let sig2 = sign_token(&priv_key, &token).expect("sign_token 2 failed");

    assert_eq!(sig1, sig2);
}

#[test]
fn test_4_invalid_token_size_returns_error() {
    let key_path = fixture_path("test_key");
    let priv_key = read_key_file(&key_path).expect("failed to load fixture private key");

    // Empty token (0 bytes)
    let err_empty = sign_token(&priv_key, &[]).unwrap_err();
    assert_eq!(err_empty, SignError::InvalidTokenSize(0));

    // 19 bytes
    let token_19 = [0u8; 19];
    let err_19 = sign_token(&priv_key, &token_19).unwrap_err();
    assert_eq!(err_19, SignError::InvalidTokenSize(19));

    // 21 bytes
    let token_21 = [0u8; 21];
    let err_21 = sign_token(&priv_key, &token_21).unwrap_err();
    assert_eq!(err_21, SignError::InvalidTokenSize(21));
}

#[test]
fn test_5_signature_size_256_bytes_for_2048_bit_key() {
    let key_path = fixture_path("test_key");
    let priv_key = read_key_file(&key_path).expect("failed to load fixture private key");
    assert_eq!(priv_key.size(), 256);

    let token = [0xABu8; 20];
    let sig = sign_token(&priv_key, &token).expect("sign_token failed");
    assert_eq!(sig.len(), 256);
}
