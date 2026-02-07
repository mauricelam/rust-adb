/*
 * Copyright (C) 2012 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! This crate ports the ADB daemon-side authentication logic from the C++ implementation.
//!
//! Ported from:
//! - `original/daemon/auth.cpp`

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use adb_auth::{ADB_AUTH_TOKEN, TOKEN_SIZE};
use adb_types::{Apacket, Block};
use anyhow::anyhow;
use base64::{engine::general_purpose, Engine as _};
use rsa::pkcs1v15::VerifyingKey;
use rsa::signature::Verifier;
use sha1::Sha1;

static G_AUTHORIZED_KEYS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn get_authorized_keys() -> &'static Mutex<Vec<String>> {
    G_AUTHORIZED_KEYS.get_or_init(|| {
        let mut keys = Vec::new();
        let _ = load_authorized_keys_into(&mut keys);
        Mutex::new(keys)
    })
}

/// Ported from `original/daemon/auth.cpp`: `get_adb_keys_path` equivalent.
/// Returns the path to the file containing authorized public keys.
pub fn get_adb_keys_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ADB_VENDOR_KEYS") {
        return Some(PathBuf::from(path));
    }

    let mut path = adb_utils::adb_get_android_dir_path()?;
    path.push("adb_keys");
    Some(path)
}

fn load_authorized_keys_into(keys: &mut Vec<String>) -> anyhow::Result<()> {
    let path = get_adb_keys_path().ok_or_else(|| anyhow!("Could not determine adb_keys path"))?;
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    keys.clear();
    for line in content.lines() {
        let line = line.trim();
        if !line.is_empty() && !line.starts_with('#') {
            if !keys.contains(&line.to_string()) {
                keys.push(line.to_string());
            }
        }
    }
    Ok(())
}

/// Ported from `original/daemon/auth.cpp`: `load_authorized_keys` equivalent.
/// Loads authorized public keys from the system's `adb_keys` file into the cache.
pub fn load_authorized_keys() -> anyhow::Result<()> {
    let mut keys = get_authorized_keys().lock().unwrap();
    load_authorized_keys_into(&mut keys)
}

/// Ensures that the authorized keys are loaded into the cache.
pub fn ensure_authorized_keys_loaded() {
    let _ = get_authorized_keys();
}

/// Ported from `original/daemon/auth.cpp`: `save_authorized_key` equivalent.
/// Saves a new authorized public key to the system's `adb_keys` file and adds it to the cache.
pub fn save_authorized_key(key: &str) -> anyhow::Result<()> {
    ensure_authorized_keys_loaded();

    let mut keys = get_authorized_keys().lock().unwrap();
    if keys.contains(&key.to_string()) {
        return Ok(());
    }

    let path = get_adb_keys_path().ok_or_else(|| anyhow!("Could not determine adb_keys path"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    use std::io::Write;
    writeln!(file, "{}", key)?;

    keys.push(key.to_string());
    Ok(())
}

/// Ported from `original/daemon/auth.cpp`: `adbd_auth_verify`.
/// Verifies an ADB authentication signature against a single public key line.
pub fn adbd_auth_verify(token: &[u8], sig: &[u8], public_key_line: &str) -> bool {
    let public_key_b64 = public_key_line.split_whitespace().next().unwrap_or("");
    let pubkey_encoded = match general_purpose::STANDARD.decode(public_key_b64) {
        Ok(d) => d,
        Err(_) => return false,
    };

    if pubkey_encoded.len() != std::mem::size_of::<rust_adb_crypto::AndroidPubkey>() {
        return false;
    }

    // Decode the Android RSA pubkey format back to rsa::RsaPublicKey.
    // SAFETY: We checked the length above.
    let pubkey_struct: rust_adb_crypto::AndroidPubkey =
        unsafe { std::ptr::read_unaligned(pubkey_encoded.as_ptr() as *const _) };

    let n = rsa::BigUint::from_bytes_le(&pubkey_struct.modulus);
    let e = rsa::BigUint::from(pubkey_struct.exponent);

    let pubkey = match rsa::RsaPublicKey::new(n, e) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let verifying_key = VerifyingKey::<Sha1>::new_unprefixed(pubkey);
    let signature = match rsa::pkcs1v15::Signature::try_from(sig) {
        Ok(s) => s,
        Err(_) => return false,
    };
    verifying_key.verify(token, &signature).is_ok()
}

/// Ported from `original/daemon/auth.cpp`: `adbd_auth_verify` (looping variant).
/// Verifies an ADB authentication signature against all authorized keys in the cache.
pub fn adbd_auth_verify_all(token: &[u8], sig: &[u8]) -> bool {
    let keys = get_authorized_keys().lock().unwrap();
    for key in keys.iter() {
        if adbd_auth_verify(token, sig, key) {
            return true;
        }
    }
    false
}

/// Ported from `original/daemon/auth.cpp`: `send_auth_request`.
/// Generates a new authentication token and wraps it in an `A_AUTH` packet.
pub fn send_auth_request() -> (Apacket, [u8; TOKEN_SIZE]) {
    let mut token = [0u8; TOKEN_SIZE];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut token);

    let mut p = Apacket::default();
    p.msg.command = adb_protocol::A_AUTH;
    p.msg.arg0 = ADB_AUTH_TOKEN;
    p.msg.data_length = TOKEN_SIZE as u32;
    p.msg.magic = p.msg.command ^ 0xffffffff;
    p.payload = Block::from_vec(token.to_vec());

    (p, token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adb_auth::adb_auth_sign;
    use rust_adb_crypto::{new_rsa_2048, AndroidPubkey};

    #[test]
    fn test_adbd_auth_verify_all() {
        let key = new_rsa_2048().unwrap();
        let token = b"12345678901234567890";
        let sig = adb_auth_sign(&key, token).unwrap();

        let pubkey_struct = key.android_pubkey().unwrap();
        let pubkey_bytes: [u8; std::mem::size_of::<AndroidPubkey>()] =
            unsafe { std::mem::transmute(pubkey_struct) };
        let pubkey_b64 = general_purpose::STANDARD.encode(pubkey_bytes);
        let public_key_line = format!("{} adb@host", pubkey_b64);

        {
            let mut keys = get_authorized_keys().lock().unwrap();
            keys.clear();
            keys.push("invalid_key".to_string());
            keys.push(public_key_line);
        }

        assert!(adbd_auth_verify_all(token, &sig));
    }

    #[test]
    fn test_load_save_authorized_keys() {
        let dir = tempfile::tempdir().unwrap();
        let adb_keys_path = dir.path().join("adb_keys");
        std::env::set_var("ADB_VENDOR_KEYS", &adb_keys_path);

        let test_key = "test_key_line";
        save_authorized_key(test_key).unwrap();

        {
            let keys = get_authorized_keys().lock().unwrap();
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0], test_key);
        }

        // Clear and reload
        {
            let mut keys = get_authorized_keys().lock().unwrap();
            keys.clear();
        }
        load_authorized_keys().unwrap();
        {
            let keys = get_authorized_keys().lock().unwrap();
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0], test_key);
        }

        // Test duplicate prevention
        save_authorized_key(test_key).unwrap();
        {
            let keys = get_authorized_keys().lock().unwrap();
            assert_eq!(keys.len(), 1);
        }

        std::env::remove_var("ADB_VENDOR_KEYS");
    }
}
