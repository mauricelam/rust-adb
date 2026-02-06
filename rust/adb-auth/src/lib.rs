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

//! This crate ports the ADB authentication logic from the C++ implementation.
//!
//! Ported from:
//! - `original/adb_auth.h`
//! - `original/client/auth.cpp`
//! - `original/daemon/auth.cpp`

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use adb_types::{Apacket, Block};
use anyhow::anyhow;
use base64::{engine::general_purpose, Engine as _};
use rsa::pkcs1v15::{SigningKey, VerifyingKey};
use rsa::signature::{SignatureEncoding, Signer, Verifier};
use rust_adb_crypto::{new_rsa_2048, Key};
use sha1::Sha1;

pub const ADB_AUTH_TOKEN: u32 = 1;
pub const ADB_AUTH_SIGNATURE: u32 = 2;
pub const ADB_AUTH_RSAPUBLICKEY: u32 = 3;

pub const TOKEN_SIZE: usize = 20;

lazy_static::lazy_static! {
    static ref G_KEYS: Mutex<HashMap<String, Key>> = Mutex::new(HashMap::new());
    static ref G_AUTHORIZED_KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
}

static G_AUTHORIZED_KEYS_LOADED: std::sync::Once = std::sync::Once::new();

const A_AUTH: u32 = 0x48545541;

pub fn get_adb_keys_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ADB_VENDOR_KEYS") {
        return Some(PathBuf::from(path));
    }

    let mut path = adb_utils::adb_get_android_dir_path()?;
    path.push("adb_keys");
    Some(path)
}

/// Ported from `original/daemon/auth.cpp`: `IteratePublicKeys` equivalent
pub fn load_authorized_keys() -> anyhow::Result<()> {
    let path = get_adb_keys_path().ok_or_else(|| anyhow!("Could not determine adb_keys path"))?;
    if !path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(path)?;
    let mut keys = G_AUTHORIZED_KEYS.lock().unwrap();
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

pub fn ensure_authorized_keys_loaded() {
    G_AUTHORIZED_KEYS_LOADED.call_once(|| {
        if let Err(e) = load_authorized_keys() {
            log::error!("Failed to load authorized keys: {}", e);
        }
    });
}

pub fn save_authorized_key(key: &str) -> anyhow::Result<()> {
    ensure_authorized_keys_loaded();

    let mut keys = G_AUTHORIZED_KEYS.lock().unwrap();
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

/// Ported from `original/client/auth.cpp`: `get_user_key_path`
fn get_user_key_path() -> anyhow::Result<PathBuf> {
    Ok(Path::join(
        &adb_utils::adb_get_android_dir_path()
            .ok_or_else(|| anyhow!("Could not find android directory"))?,
        "adbkey",
    ))
}

/// Ported from `original/client/auth.cpp`: `generate_key`
pub fn adb_auth_keygen(filename: &Path) -> anyhow::Result<()> {
    let key = new_rsa_2048()?;
    let pem = key.to_pem_string()?;
    fs::write(filename, pem)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(filename, fs::Permissions::from_mode(0o600))?;
    }

    let pubkey_struct = key.android_pubkey()?;
    // SAFETY: AndroidPubkey is #[repr(C)] and has a fixed size of 524 bytes.
    let pubkey_bytes: [u8; std::mem::size_of::<rust_adb_crypto::AndroidPubkey>()] =
        unsafe { std::mem::transmute(pubkey_struct) };
    let pubkey_b64 = general_purpose::STANDARD.encode(pubkey_bytes);

    let hostname = hostname::get()?.to_string_lossy().into_owned();
    let login = "adb"; // Simplified
    let comment = format!(" {}@{}", login, hostname);

    fs::write(
        filename.with_extension("pub"),
        format!("{}{}", pubkey_b64, comment),
    )?;

    Ok(())
}

/// Ported from `original/client/auth.cpp`: `load_key`
pub fn load_key(path: &Path) -> anyhow::Result<()> {
    let content = fs::read_to_string(path)?;
    let key = Key::from_pem(&content)?;

    let mut keys = G_KEYS.lock().unwrap();
    keys.insert(path.to_string_lossy().into_owned(), key);
    Ok(())
}

/// Ported from `original/client/auth.cpp`: `adb_auth_init`
pub fn adb_auth_init() -> anyhow::Result<()> {
    let path = get_user_key_path()?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        adb_auth_keygen(&path)?;
    }
    load_key(&path)?;
    Ok(())
}

/// Ported from `original/client/auth.cpp`: `adb_auth_sign`
pub fn adb_auth_sign(key: &Key, token: &[u8]) -> anyhow::Result<Vec<u8>> {
    let signing_key = SigningKey::<Sha1>::new_unprefixed(key.privkey().clone());
    let signature = signing_key
        .try_sign(token)
        .map_err(|e| anyhow!("Signing failed: {}", e))?;
    Ok(signature.to_vec())
}

/// Ported from `original/daemon/auth.cpp`: `adbd_auth_verify`
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

pub fn adbd_auth_verify_all(token: &[u8], sig: &[u8]) -> bool {
    ensure_authorized_keys_loaded();
    let keys = G_AUTHORIZED_KEYS.lock().unwrap();
    for key in keys.iter() {
        if adbd_auth_verify(token, sig, key) {
            return true;
        }
    }
    false
}

/// Ported from `original/daemon/auth.cpp`: `send_auth_request`
pub fn send_auth_request() -> (Apacket, [u8; TOKEN_SIZE]) {
    let mut token = [0u8; TOKEN_SIZE];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut token);

    let mut p = Apacket::default();
    p.msg.command = A_AUTH;
    p.msg.arg0 = ADB_AUTH_TOKEN;
    p.msg.data_length = TOKEN_SIZE as u32;
    p.msg.magic = p.msg.command ^ 0xffffffff;
    p.payload = Block::from_vec(token.to_vec());

    (p, token)
}

/// Ported from `original/client/auth.cpp`: `send_auth_response`
pub fn send_auth_response(token: &[u8], key: &Key) -> anyhow::Result<Apacket> {
    let signature = adb_auth_sign(key, token)?;

    let mut p = Apacket::default();
    p.msg.command = A_AUTH;
    p.msg.arg0 = ADB_AUTH_SIGNATURE;
    p.msg.data_length = signature.len() as u32;
    p.msg.magic = p.msg.command ^ 0xffffffff;
    p.payload = Block::from_vec(signature);

    Ok(p)
}

#[cfg(test)]
mod tests {
    use rust_adb_crypto::AndroidPubkey;

    use super::*;

    #[test]
    fn test_auth_smoke() {
        let key = new_rsa_2048().unwrap();
        let token = b"12345678901234567890";
        let sig = adb_auth_sign(&key, token).unwrap();
        assert!(!sig.is_empty());

        let pubkey_struct = key.android_pubkey().unwrap();
        let pubkey_bytes: [u8; std::mem::size_of::<AndroidPubkey>()] =
            unsafe { std::mem::transmute(pubkey_struct) };
        let pubkey_b64 = general_purpose::STANDARD.encode(pubkey_bytes);

        // Verification should work even with full line (with comment)
        let hostname = "test-host";
        let login = "adb";
        let comment = format!(" {}@{}", login, hostname);
        let public_key_line = format!("{}{}", pubkey_b64, comment);

        assert!(adbd_auth_verify(token, &sig, &public_key_line));
    }

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
            let mut keys = G_AUTHORIZED_KEYS.lock().unwrap();
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
            let keys = G_AUTHORIZED_KEYS.lock().unwrap();
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0], test_key);
        }

        // Clear and reload
        {
            let mut keys = G_AUTHORIZED_KEYS.lock().unwrap();
            keys.clear();
        }
        load_authorized_keys().unwrap();
        {
            let keys = G_AUTHORIZED_KEYS.lock().unwrap();
            assert_eq!(keys.len(), 1);
            assert_eq!(keys[0], test_key);
        }

        // Test duplicate prevention
        save_authorized_key(test_key).unwrap();
        {
            let keys = G_AUTHORIZED_KEYS.lock().unwrap();
            assert_eq!(keys.len(), 1);
        }

        std::env::remove_var("ADB_VENDOR_KEYS");
    }
}
