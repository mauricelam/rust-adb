use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_1_rust_base64_part_equals_reference() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let fixture_priv = Path::new(&manifest_dir).join("tests/fixtures/test_key");
    let fixture_pub = Path::new(&manifest_dir).join("tests/fixtures/test_key.pub");

    let priv_key = adb_auth::read_key_file(&fixture_priv).expect("failed to read test_key fixture");
    let expected_pub_str =
        fs::read_to_string(&fixture_pub).expect("failed to read test_key.pub fixture");

    let pub_key = priv_key.to_public_key();
    let line = adb_auth::public_key_line(&pub_key, "comment").expect("public_key_line failed");

    let rust_b64 = line.split_whitespace().next().unwrap();
    let ref_b64 = expected_pub_str.split_whitespace().next().unwrap();

    assert_eq!(
        rust_b64, ref_b64,
        "Rust base64 encoding does not match reference test_key.pub"
    );
}

#[test]
fn test_2_oracle_against_reference_adb_fresh_keys() {
    let reference_adb = match env::var("ADB_REFERENCE") {
        Ok(val) if !val.is_empty() => val,
        _ => {
            let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
            format!("{}/../../binaries/linux/adb", manifest_dir)
        }
    };

    if !Path::new(&reference_adb).exists() {
        eprintln!(
            "Skipping test_2: ADB reference binary not found at {}",
            reference_adb
        );
        return;
    }

    let temp_dir = TempDir::new().unwrap();

    for i in 0..3 {
        let key_path = temp_dir.path().join(format!("key_{}", i));

        // Generate key using reference adb
        let status = Command::new(&reference_adb)
            .arg("keygen")
            .arg(&key_path)
            .status()
            .expect("Failed to execute adb keygen");
        assert!(status.success());

        let pub_file_path = temp_dir.path().join(format!("key_{}.pub", i));
        let pub_file_content =
            fs::read_to_string(&pub_file_path).expect("failed to read generated .pub file");
        let ref_file_b64 = pub_file_content.split_whitespace().next().unwrap();

        // Get public key string from `adb pubkey`
        let output = Command::new(&reference_adb)
            .arg("pubkey")
            .arg(&key_path)
            .output()
            .expect("Failed to execute adb pubkey");
        assert!(output.status.success());
        let pubkey_cmd_content = String::from_utf8_lossy(&output.stdout);
        let ref_cmd_b64 = pubkey_cmd_content.split_whitespace().next().unwrap();

        assert_eq!(ref_file_b64, ref_cmd_b64);

        // Read key in Rust and compute public key line
        let rust_priv = adb_auth::read_key_file(&key_path).expect("failed to read key in rust");
        let rust_pub = rust_priv.to_public_key();
        let rust_line =
            adb_auth::public_key_line(&rust_pub, "dummy").expect("public_key_line failed");
        let rust_b64 = rust_line.split_whitespace().next().unwrap();

        assert_eq!(
            rust_b64, ref_file_b64,
            "Key {} base64 mismatch against reference",
            i
        );
    }
}

#[test]
fn test_3_binary_structure_and_base64_lengths() {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();

    let binary = adb_auth::encode_public_key(&pub_key).expect("encode_public_key failed");
    assert_eq!(binary.len(), 524, "Binary structure must be 524 bytes");

    let line = adb_auth::public_key_line(&pub_key, "test@host").expect("public_key_line failed");
    let b64 = line.split_whitespace().next().unwrap();
    assert_eq!(b64.len(), 700, "Base64 text must be 700 characters");
}

#[test]
fn test_4_round_trip_decoding() {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let pub_key = priv_key.to_public_key();

    let binary = adb_auth::encode_public_key(&pub_key).expect("encode_public_key failed");
    let (modulus, exponent) =
        adb_auth::decode_public_key(&binary).expect("decode_public_key failed");

    assert_eq!(&modulus, pub_key.n());
    let e_bytes = pub_key.e().to_bytes_le();
    let mut e_u32 = 0u32;
    for (i, &b) in e_bytes.iter().enumerate().take(4) {
        e_u32 |= (b as u32) << (i * 8);
    }
    assert_eq!(exponent, e_u32);
}

#[test]
fn test_5_rejection_of_invalid_keys_and_malformed_input() {
    let mut rng = rand::thread_rng();

    // 1024-bit key must be rejected
    let priv_1024 = RsaPrivateKey::new(&mut rng, 1024).unwrap();
    let pub_1024 = priv_1024.to_public_key();
    assert!(adb_auth::encode_public_key(&pub_1024).is_err());

    // Short buffer in decode_public_key
    let short_buf = [0u8; 500];
    assert!(adb_auth::decode_public_key(&short_buf).is_err());

    // Invalid modulus size words
    let mut bad_words = [0u8; 524];
    bad_words[0..4].copy_from_slice(&32u32.to_le_bytes()); // expected 64
    bad_words[520..524].copy_from_slice(&65537u32.to_le_bytes());
    assert!(adb_auth::decode_public_key(&bad_words).is_err());

    // Invalid exponent in decode_public_key
    let mut bad_exp = [0u8; 524];
    bad_exp[0..4].copy_from_slice(&64u32.to_le_bytes());
    bad_exp[520..524].copy_from_slice(&17u32.to_le_bytes()); // invalid exponent
    assert!(adb_auth::decode_public_key(&bad_exp).is_err());
}

#[test]
fn test_rejection_of_large_exponent() {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let n = priv_key.n().clone();
    let large_e = (num_bigint_dig::BigUint::from(1u64) << 32) + 65537u32;
    let pub_key = rsa::RsaPublicKey::new(n, large_e).expect("valid RsaPublicKey construction");

    assert!(adb_auth::encode_public_key(&pub_key).is_err());
}

#[test]
fn test_default_comment_resolution() {
    let comment = adb_auth::default_comment();
    assert!(
        comment.contains('@'),
        "default_comment must be of form user@host, got: {}",
        comment
    );

    let parts: Vec<&str> = comment.splitn(2, '@').collect();
    let user = parts[0];
    let host = parts[1];

    assert!(
        !user.is_empty(),
        "User in default_comment should not be empty"
    );
    assert!(
        !host.is_empty(),
        "Host in default_comment should not be empty"
    );

    #[cfg(unix)]
    {
        let whoami = Command::new("whoami").output();
        let hostname = Command::new("hostname").output();

        if let (Ok(u_out), Ok(h_out)) = (whoami, hostname) {
            let sys_user = String::from_utf8_lossy(&u_out.stdout).trim().to_string();
            let sys_host = String::from_utf8_lossy(&h_out.stdout).trim().to_string();

            if !sys_user.is_empty() {
                assert_eq!(
                    user, sys_user,
                    "User in default_comment should match OS user"
                );
            }
            if !sys_host.is_empty() {
                assert_eq!(
                    host, sys_host,
                    "Host in default_comment should match OS host"
                );
            }
        }
    }
}
