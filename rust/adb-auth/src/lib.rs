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

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use adb_types::{Apacket, Block};
use anyhow::anyhow;
use base64::{engine::general_purpose, Engine as _};
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use rust_adb_crypto::{new_rsa_2048, Key};
use sha1::Sha1;

pub const ADB_AUTH_TOKEN: u32 = 1;
pub const ADB_AUTH_SIGNATURE: u32 = 2;
pub const ADB_AUTH_RSAPUBLICKEY: u32 = 3;

pub const TOKEN_SIZE: usize = 20;

lazy_static::lazy_static! {
    static ref G_KEYS: Mutex<HashMap<String, Key>> = Mutex::new(HashMap::new());
}

const A_AUTH: u32 = 0x48545541;

/// Ported from `original/client/auth.cpp`: `get_user_key_path` equivalent.
fn get_user_key_path() -> anyhow::Result<PathBuf> {
    Ok(Path::join(
        &adb_utils::adb_get_android_dir_path()
            .ok_or_else(|| anyhow!("Could not find android directory"))?,
        "adbkey",
    ))
}

/// Ported from `original/client/auth.cpp`: `generate_key` equivalent.
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
    // SAFETY: AndroidPubkey is #[repr(C)] and has a fixed size.
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

/// Ported from `original/client/auth.cpp`: `load_key` equivalent.
pub fn load_key(path: &Path) -> anyhow::Result<()> {
    let content = fs::read_to_string(path)?;
    let key = Key::from_pem(&content)?;

    let mut keys = G_KEYS.lock().unwrap();
    keys.insert(path.to_string_lossy().into_owned(), key);
    Ok(())
}

/// Ported from `original/client/auth.cpp`: `adb_auth_init` equivalent.
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

/// Ported from `original/client/auth.cpp`: `adb_auth_sign` equivalent.
pub fn adb_auth_sign(key: &Key, token: &[u8]) -> anyhow::Result<Vec<u8>> {
    let signing_key = SigningKey::<Sha1>::new_unprefixed(key.privkey().clone());
    let signature = signing_key
        .try_sign(token)
        .map_err(|e| anyhow!("Signing failed: {}", e))?;
    Ok(signature.to_vec())
}

/// Ported from `original/client/auth.cpp`: `send_auth_response` equivalent.
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
    use super::*;

    #[test]
    fn test_auth_smoke() {
        let key = new_rsa_2048().unwrap();
        let token = b"12345678901234567890";
        let sig = adb_auth_sign(&key, token).unwrap();
        assert!(!sig.is_empty());
    }
}
