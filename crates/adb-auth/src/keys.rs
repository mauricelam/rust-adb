//! Key management for ADB: finding, loading, generating, and saving RSA keys.
//!
//! Ported from:
//! - `original/client/auth.cpp`: `generate_key`, `load_key`, `load_keys`, `get_vendor_keys`,
//!   `get_user_key_path`, `adb_auth_init`, `adb_auth_keygen`, `adb_auth_pubkey`, `read_key_file`,
//!   `hash_key`
//! - `original/adb_auth.h`: public key/auth interfaces
//! - `original/crypto/rsa_2048_key.cpp`: `CalculatePublicKey` and `CreateRSA2048Key`

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use num_bigint_dig::BigUint;
use num_traits::One;
use pkcs8::DecodePrivateKey;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use sha2::{Digest, Sha256};

pub const TOKEN_SIZE: usize = 20;
pub const ANDROID_PUBKEY_MODULUS_SIZE: usize = 2048 / 8; // 256 bytes
pub const ANDROID_PUBKEY_MODULUS_SIZE_WORDS: usize = ANDROID_PUBKEY_MODULUS_SIZE / 4; // 64 words
pub const ANDROID_PUBKEY_ENCODED_SIZE: usize =
    4 + 4 + ANDROID_PUBKEY_MODULUS_SIZE + ANDROID_PUBKEY_MODULUS_SIZE + 4; // 524 bytes

/// Errors that can occur when signing an authentication token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignError {
    /// Token length is not TOKEN_SIZE (20 bytes).
    InvalidTokenSize(usize),
    /// RSA signing operation failed.
    SigningFailed(String),
}

impl std::fmt::Display for SignError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignError::InvalidTokenSize(size) => {
                write!(
                    f,
                    "Invalid token size: expected {}, got {}",
                    TOKEN_SIZE, size
                )
            }
            SignError::SigningFailed(msg) => write!(f, "Signing failed: {}", msg),
        }
    }
}

impl std::error::Error for SignError {}

/// Signs an authentication token challenge using the provided RSA private key.
///
/// Ported from `adb_auth_sign` in `original/client/auth.cpp`.
///
/// The token must be exactly `TOKEN_SIZE` (20) bytes. The token is treated as an
/// already-computed digest (e.g. SHA-1) and signed using PKCS#1 v1.5 with SHA-1
/// without re-hashing the token bytes.
pub fn sign_token(key: &RsaPrivateKey, token: &[u8]) -> Result<Vec<u8>, SignError> {
    if token.len() != TOKEN_SIZE {
        return Err(SignError::InvalidTokenSize(token.len()));
    }

    let scheme = rsa::Pkcs1v15Sign::new::<sha1::Sha1>();
    key.sign(scheme, token)
        .map_err(|e| SignError::SigningFailed(e.to_string()))
}

/// Global key store for loaded RSA private keys indexed by key fingerprint hash.
static G_KEYS: OnceLock<Mutex<BTreeMap<[u8; 32], RsaPrivateKey>>> = OnceLock::new();

fn get_keys_mutex() -> &'static Mutex<BTreeMap<[u8; 32], RsaPrivateKey>> {
    G_KEYS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Compute SHA-256 fingerprint of an RSA private key based on its SubjectPublicKeyInfo DER encoding.
///
/// Ported from `hash_key` in `original/client/auth.cpp`.
pub fn fingerprint(key: &RsaPrivateKey) -> [u8; 32] {
    let pub_key = key.to_public_key();
    let der = pub_key
        .to_public_key_der()
        .expect("failed to encode RSA public key DER");
    let mut hasher = Sha256::new();
    hasher.update(der.as_bytes());
    hasher.finalize().into()
}

/// Compute SHA-256 fingerprint of an RSA private key as a `Vec<u8>`.
///
/// Ported from `hash_key` in `original/client/auth.cpp`.
pub fn hash_key(key: &RsaPrivateKey) -> Vec<u8> {
    fingerprint(key).to_vec()
}

/// Store for ADB authentication RSA private keys, supporting standard user key
/// loading/generation and extra vendor key paths from `ADB_VENDOR_KEYS`.
///
/// Keys are stored deduplicated by fingerprint and ordered in ascending byte order
/// of their SHA-256 SubjectPublicKeyInfo fingerprint, matching C++ ADB's `std::map` ordering.
///
/// Note: File watching (inotify monitoring on Linux) is omitted in this implementation and left as a follow-up.
#[derive(Debug, Clone, Default)]
pub struct KeyStore {
    user_key: Option<RsaPrivateKey>,
    keys: BTreeMap<[u8; 32], RsaPrivateKey>,
}

impl KeyStore {
    /// Create a new empty `KeyStore`.
    pub fn new() -> Self {
        Self {
            user_key: None,
            keys: BTreeMap::new(),
        }
    }

    /// Load user key and vendor keys.
    ///
    /// - `user_dir`: optional user directory (e.g. `~/.android`), or user key file path. If `None`, defaults to standard path (`get_user_key_path()`).
    /// - `vendor_keys_env`: optional string with vendor key paths separated by platform path separator. If `None`, reads `ADB_VENDOR_KEYS` environment variable.
    pub fn load(user_dir: Option<&Path>, vendor_keys_env: Option<&str>) -> Self {
        let mut store = Self::new();

        // 1. Resolve user key path
        let user_key_path = match user_dir {
            Some(dir) => {
                if dir.is_dir() {
                    Some(dir.join("adbkey"))
                } else if dir.file_name().is_some_and(|name| name == "adbkey")
                    || (dir.extension().is_none() && dir.exists() && dir.is_file())
                {
                    Some(dir.to_path_buf())
                } else if !dir.exists() && dir.parent().is_some_and(|p| p.exists()) {
                    if dir.extension().is_some() {
                        Some(dir.to_path_buf())
                    } else {
                        Some(dir.join("adbkey"))
                    }
                } else {
                    Some(dir.to_path_buf())
                }
            }
            None => get_user_key_path().ok(),
        };

        if let Some(user_key_path) = user_key_path {
            if let Ok(key) = load_or_generate(&user_key_path) {
                let fp = fingerprint(&key);
                store.keys.insert(fp, key.clone());
                store.user_key = Some(key);
            }
        }

        // 2. Resolve vendor keys environment string
        let vendor_env_owned;
        let vendor_str = match vendor_keys_env {
            Some(s) => s,
            None => {
                vendor_env_owned = env::var("ADB_VENDOR_KEYS").unwrap_or_default();
                &vendor_env_owned
            }
        };

        if !vendor_str.is_empty() {
            let sep = if cfg!(windows) { ';' } else { ':' };
            let mut seen_paths = HashSet::new();
            for path_str in vendor_str.split(sep) {
                let trimmed = path_str.trim();
                if !trimmed.is_empty() && seen_paths.insert(trimmed.to_string()) {
                    store.load_vendor_path(Path::new(trimmed));
                }
            }
        }

        store
    }

    /// Load a vendor path (file or directory, non-recursive).
    fn load_vendor_path(&mut self, path: &Path) {
        let metadata = match fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return,
        };

        if metadata.is_file() {
            self.load_key_file(path);
        } else if metadata.is_dir() {
            let entries = match fs::read_dir(path) {
                Ok(e) => e,
                Err(_) => return,
            };

            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                        if name == "." || name == ".." {
                            continue;
                        }
                        if name.ends_with(".adb_key") {
                            self.load_key_file(&entry_path);
                        }
                    }
                }
            }
        }
    }

    /// Read a key file and insert into store if not already present.
    fn load_key_file(&mut self, path: &Path) {
        if let Ok(key) = read_key_file(path) {
            let fp = fingerprint(&key);
            self.keys.entry(fp).or_insert(key);
        }
    }

    /// Get user private key if loaded.
    pub fn user_key(&self) -> Option<&RsaPrivateKey> {
        self.user_key.as_ref()
    }

    /// Get all loaded private keys sorted by fingerprint in ascending byte order.
    pub fn private_keys(&self) -> Vec<RsaPrivateKey> {
        self.keys.values().cloned().collect()
    }
}

use rsa::RsaPublicKey;

/// Returns standard default comment `login@host`.
///
/// Ported from `GetLoginNameUTF8` and `GetHostNameUTF8` in `original/sysdeps/env.cpp`.
pub fn default_comment() -> String {
    let login = if cfg!(windows) {
        env::var("USERNAME").unwrap_or_default()
    } else {
        env::var("LOGNAME").unwrap_or_default()
    };
    let login = if login.is_empty() {
        "unknown".to_string()
    } else {
        login
    };

    let host = if cfg!(windows) {
        env::var("COMPUTERNAME").unwrap_or_default()
    } else {
        env::var("HOSTNAME").unwrap_or_default()
    };
    let host = if host.is_empty() {
        "unknown".to_string()
    } else {
        host
    };

    format!("{}@{}", login, host)
}

/// Encodes an RSA 2048 public key into Android's binary pubkey structure (524 bytes).
///
/// Format (little-endian):
/// - `modulus_size_words`: u32 (64 for 2048-bit RSA)
/// - `n0inv`: u32 (-n^-1 mod 2^32 where n is modulus)
/// - `modulus`: [u8; 256] (modulus n in little-endian order)
/// - `rr`: [u8; 256] (Montgomery parameter R^2 mod n in little-endian order)
/// - `exponent`: u32 (public exponent e, 3 or 65537)
pub fn encode_public_key(key: &RsaPublicKey) -> Result<[u8; ANDROID_PUBKEY_ENCODED_SIZE], String> {
    let n = key.n();
    let e = key.e();

    let n_bytes = n.to_bytes_le();
    if n.bits() != 2048 || n_bytes.len() > ANDROID_PUBKEY_MODULUS_SIZE {
        return Err("RSA public key must be 2048 bits".to_string());
    }

    let e_bytes = e.to_bytes_le();
    let mut e_u32 = 0u32;
    for (i, &b) in e_bytes.iter().enumerate().take(4) {
        e_u32 |= (b as u32) << (i * 8);
    }
    if e_u32 != 3 && e_u32 != 65537 {
        return Err(format!("Unsupported public exponent: {}", e_u32));
    }

    let mut modulus = [0u8; ANDROID_PUBKEY_MODULUS_SIZE];
    modulus[..n_bytes.len()].copy_from_slice(&n_bytes);

    // Compute n0inv = -n^-1 mod 2^32 using Newton-Raphson
    let x = (modulus[0] as u32)
        | ((modulus[1] as u32) << 8)
        | ((modulus[2] as u32) << 16)
        | ((modulus[3] as u32) << 24);

    let mut inv = x;
    inv = inv.wrapping_mul(2u32.wrapping_sub(x.wrapping_mul(inv)));
    inv = inv.wrapping_mul(2u32.wrapping_sub(x.wrapping_mul(inv)));
    inv = inv.wrapping_mul(2u32.wrapping_sub(x.wrapping_mul(inv)));
    inv = inv.wrapping_mul(2u32.wrapping_sub(x.wrapping_mul(inv)));
    let n0inv = 0u32.wrapping_sub(inv);

    // Compute R^2 mod n where R = 2^2048
    let r = BigUint::one() << 2048;
    let r_sq = (&r * &r) % n;
    let rr_bytes = r_sq.to_bytes_le();

    let mut rr = [0u8; ANDROID_PUBKEY_MODULUS_SIZE];
    if rr_bytes.len() <= ANDROID_PUBKEY_MODULUS_SIZE {
        rr[..rr_bytes.len()].copy_from_slice(&rr_bytes);
    }

    let mut buf = [0u8; ANDROID_PUBKEY_ENCODED_SIZE];
    buf[0..4].copy_from_slice(&(ANDROID_PUBKEY_MODULUS_SIZE_WORDS as u32).to_le_bytes());
    buf[4..8].copy_from_slice(&n0inv.to_le_bytes());
    buf[8..264].copy_from_slice(&modulus);
    buf[264..520].copy_from_slice(&rr);
    buf[520..524].copy_from_slice(&e_u32.to_le_bytes());

    Ok(buf)
}

/// Decodes an Android binary pubkey structure (524 bytes) into modulus (`BigUint`) and exponent (`u32`).
pub fn decode_public_key(bytes: &[u8]) -> Result<(BigUint, u32), String> {
    if bytes.len() < ANDROID_PUBKEY_ENCODED_SIZE {
        return Err(format!(
            "Buffer too short: expected at least {} bytes, got {}",
            ANDROID_PUBKEY_ENCODED_SIZE,
            bytes.len()
        ));
    }

    let modulus_size_words = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if modulus_size_words != ANDROID_PUBKEY_MODULUS_SIZE_WORDS as u32 {
        return Err(format!(
            "Invalid modulus size in words: expected {}, got {}",
            ANDROID_PUBKEY_MODULUS_SIZE_WORDS, modulus_size_words
        ));
    }

    let exponent = u32::from_le_bytes(bytes[520..524].try_into().unwrap());
    if exponent != 3 && exponent != 65537 {
        return Err(format!("Invalid public exponent: {}", exponent));
    }

    let modulus_bytes = &bytes[8..264];
    let modulus = BigUint::from_bytes_le(modulus_bytes);

    Ok((modulus, exponent))
}

/// Formats public key string representation (`<base64> <comment>`).
pub fn public_key_line(key: &RsaPublicKey, comment: &str) -> Result<String, String> {
    let encoded = encode_public_key(key)?;
    let b64 = BASE64.encode(encoded);
    Ok(format!("{} {}", b64, comment))
}

/// Legacy alias / helper for private key public key calculation.
pub fn encode_android_pubkey(key: &RsaPrivateKey) -> Result<Vec<u8>, String> {
    let pub_key = key.to_public_key();
    let encoded = encode_public_key(&pub_key)?;
    Ok(encoded.to_vec())
}

/// Calculate the string representation of an RSA public key (`<base64> <user>@<host>`).
///
/// Ported from `CalculatePublicKey` in `original/crypto/rsa_2048_key.cpp`.
pub fn calculate_public_key(key: &RsaPrivateKey) -> Result<String, String> {
    let pub_key = key.to_public_key();
    let comment = default_comment();
    public_key_line(&pub_key, &comment)
}

/// Read an RSA private key from a PEM file (supporting PKCS#1 and PKCS#8).
///
/// Ported from `read_key_file` in `original/client/auth.cpp`.
pub fn read_key_file<P: AsRef<Path>>(path: P) -> Result<RsaPrivateKey, String> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)
        .map_err(|e| format!("Failed to open '{}': {}", path.display(), e))?;

    // Try reading as PKCS#8 PEM first
    if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(&content) {
        return Ok(key);
    }

    // Try reading as PKCS#1 PEM
    if let Ok(key) = rsa::pkcs1::DecodeRsaPrivateKey::from_pkcs1_pem(&content) {
        return Ok(key);
    }

    Err(format!(
        "Failed to parse private key from '{}'",
        path.display()
    ))
}


/// Load a single key file into memory if not already present.
///
/// Ported from `load_key` in `original/client/auth.cpp`.
pub fn load_key<P: AsRef<Path>>(file: P) -> bool {
    let path = file.as_ref();
    let key = match read_key_file(path) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let fp = fingerprint(&key);
    let mut keys = get_keys_mutex().lock().unwrap();
    keys.entry(fp).or_insert(key);
    true
}

/// Load keys from a file or directory path.
/// If path is a directory and `allow_dir` is true, scans for `.adb_key` files.
/// Does not recurse into nested subdirectories.
///
/// Ported from `load_keys` in `original/client/auth.cpp`.
pub fn load_keys<P: AsRef<Path>>(path: P, allow_dir: bool) -> bool {
    let path = path.as_ref();
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };

    if metadata.is_file() {
        return load_key(path);
    }

    if metadata.is_dir() {
        if !allow_dir {
            return false;
        }

        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(_) => return false,
        };

        let mut loaded = false;
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                    if name == "." || name == ".." {
                        continue;
                    }
                    if name.ends_with(".adb_key") {
                        loaded |= load_key(&entry_path);
                    }
                }
            }
        }
        return loaded;
    }

    false
}

/// Retrieve set of vendor key paths from `ADB_VENDOR_KEYS` environment variable.
///
/// Ported from `get_vendor_keys` in `original/client/auth.cpp`.
pub fn get_vendor_keys() -> Vec<PathBuf> {
    let vendor_env = match env::var("ADB_VENDOR_KEYS") {
        Ok(v) if !v.is_empty() => v,
        _ => return Vec::new(),
    };

    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut result = Vec::new();
    let mut seen = HashSet::new();

    for path_str in vendor_env.split(sep) {
        let trimmed = path_str.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            result.push(PathBuf::from(trimmed));
        }
    }

    result
}

/// Load all vendor keys specified in `ADB_VENDOR_KEYS`.
pub fn load_vendor_keys() -> Vec<RsaPrivateKey> {
    let vendor_paths = get_vendor_keys();
    for path in &vendor_paths {
        load_keys(path, true);
    }

    let keys = get_keys_mutex().lock().unwrap();
    keys.values().cloned().collect()
}

/// Get standard Android home directory path (e.g. `$HOME` on Unix or `%USERPROFILE%` on Windows).
fn get_homedir_path() -> Result<PathBuf, String> {
    if let Ok(home) = env::var("HOME") {
        if !home.trim().is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    if let Ok(profile) = env::var("USERPROFILE") {
        if !profile.trim().is_empty() {
            return Ok(PathBuf::from(profile));
        }
    }
    Err("Could not determine user home directory (HOME or USERPROFILE not set)".to_string())
}

/// Get standard Android user directory path (`~/.android`).
/// Pure getter: does not create directory or fall back to current directory.
///
/// Ported from `adb_get_android_dir_path` in `original/adb_utils.cpp`.
pub fn adb_get_android_dir_path() -> Result<PathBuf, String> {
    let home = get_homedir_path()?;
    Ok(home.join(".android"))
}

/// Ensures `.android` directory exists with 0750 permissions on Unix.
pub fn ensure_android_dir() -> Result<PathBuf, String> {
    let dir = adb_get_android_dir_path()?;
    ensure_dir_0750(&dir)?;
    Ok(dir)
}

/// Helper to create directory hierarchy with mode 0750 on Unix (matching `adb_mkdir(dir, 0750)`).
fn ensure_dir_0750(dir: &Path) -> Result<(), String> {
    if dir.as_os_str().is_empty() || dir.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        builder.mode(0o750);
        builder
            .create(dir)
            .map_err(|e| format!("Failed to create directory '{}': {}", dir.display(), e))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create directory '{}': {}", dir.display(), e))?;
    }
    Ok(())
}

/// Get standard user key path (`~/.android/adbkey`).
///
/// Ported from `get_user_key_path` in `original/client/auth.cpp`.
pub fn get_user_key_path() -> Result<PathBuf, String> {
    adb_get_android_dir_path().map(|p| p.join("adbkey"))
}

/// Generate an RSA 2048 private key and save `file` (private) and `file.pub` (public).
/// Owner-only permissions (0600 on Unix) are enforced on the private key.
/// Private key is formatted in PKCS#8 PEM format (`BEGIN PRIVATE KEY`).
///
/// If `overwrite` is `false`, fails with an error if `file` already exists.
/// Uses a temporary file and atomic rename to prevent leaving partial key files on crash/failure.
pub fn generate_key_opts<P: AsRef<Path>>(file: P, overwrite: bool) -> Result<(), String> {
    let file = file.as_ref();

    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        ensure_dir_0750(parent)?;
    }

    let mut rng = rand::thread_rng();
    let key = RsaPrivateKey::new(&mut rng, 2048)
        .map_err(|e| format!("Failed to generate RSA key: {}", e))?;

    let pubkey_str = calculate_public_key(&key)?;

    // Encode private key in PKCS#8 PEM format matching OpenSSL PEM_write_PrivateKey / C++ ADB
    let pem = key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|e| format!("Failed to encode private key PEM: {}", e))?;

    if overwrite {
        let rand_num: u32 = rand::random();
        let file_name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("adbkey");
        let tmp_file = file.with_file_name(format!(".{}.tmp.{}.{}", file_name, std::process::id(), rand_num));

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut f = options.open(&tmp_file).map_err(|e| {
            format!(
                "Failed to open temp file '{}' for writing: {}",
                tmp_file.display(),
                e
            )
        })?;

        if let Err(e) = f.write_all(pem.as_bytes()) {
            let _ = fs::remove_file(&tmp_file);
            return Err(format!(
                "Failed to write key to '{}': {}",
                tmp_file.display(),
                e
            ));
        }
        if let Err(e) = f.sync_all() {
            let _ = fs::remove_file(&tmp_file);
            return Err(format!("Failed to sync key file '{}': {}", tmp_file.display(), e));
        }
        drop(f);

        if let Err(e) = fs::rename(&tmp_file, file) {
            let _ = fs::remove_file(&tmp_file);
            return Err(format!(
                "Failed to rename temp key file to '{}': {}",
                file.display(),
                e
            ));
        }

        let pub_path = PathBuf::from(format!("{}.pub", file.display()));
        let tmp_pub_path = file.with_file_name(format!(".{}.pub.tmp.{}.{}", file_name, std::process::id(), rand_num));
        if let Err(e) = fs::write(&tmp_pub_path, &pubkey_str) {
            let _ = fs::remove_file(&tmp_pub_path);
            return Err(format!(
                "Failed to write public key to '{}': {}",
                tmp_pub_path.display(),
                e
            ));
        }
        let _ = fs::rename(&tmp_pub_path, &pub_path);
    } else {
        // Exclusively create new private key file (fails if file already exists)
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut f = options.open(file).map_err(|e| {
            format!(
                "Failed to create key file '{}' (already exists or error): {}",
                file.display(),
                e
            )
        })?;

        if let Err(e) = f.write_all(pem.as_bytes()) {
            let _ = fs::remove_file(file);
            return Err(format!(
                "Failed to write key to '{}': {}",
                file.display(),
                e
            ));
        }
        if let Err(e) = f.sync_all() {
            let _ = fs::remove_file(file);
            return Err(format!("Failed to sync key file '{}': {}", file.display(), e));
        }
        drop(f);

        let pub_path = PathBuf::from(format!("{}.pub", file.display()));
        if let Err(e) = fs::write(&pub_path, &pubkey_str) {
            let _ = fs::remove_file(file);
            let _ = fs::remove_file(&pub_path);
            return Err(format!(
                "Failed to write public key to '{}': {}",
                pub_path.display(),
                e
            ));
        }
    }

    Ok(())
}

/// Generate an RSA 2048 private key and save `file` (private) and `file.pub` (public).
/// Owner-only permissions (0600 on Unix) are enforced on the private key.
/// Never overwrites an existing key file (`overwrite = false`).
///
/// Ported from `generate_key` in `original/client/auth.cpp`.
pub fn generate_key<P: AsRef<Path>>(file: P) -> Result<(), String> {
    generate_key_opts(file, false)
}

/// Load or generate the default user key at standard location or custom path.
/// Concurrent-safe: if key generation races with another thread or process, reads the created key.
///
/// Note on pre-existing file permissions: Matching C++ ADB (`read_key_file` in `original/client/auth.cpp`),
/// loading an existing key file with permissions looser than `0600` (such as `0644`) reads the key
/// without modifying the file's permissions on disk.
///
/// Ported from `load_userkey` in `original/client/auth.cpp`.
pub fn load_or_generate<P: AsRef<Path>>(path: P) -> Result<RsaPrivateKey, String> {
    let path = path.as_ref();
    if path.exists() {
        if let Ok(key) = read_key_file(path) {
            return Ok(key);
        }
    }

    match generate_key_opts(path, false) {
        Ok(()) => read_key_file(path),
        Err(_) => {
            for _ in 0..20 {
                if let Ok(key) = read_key_file(path) {
                    return Ok(key);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            read_key_file(path)
        }
    }
}

/// Initialize ADB authentication by loading (or generating) user key and loading vendor keys.
///
/// Ported from `adb_auth_init` in `original/client/auth.cpp`.
pub fn adb_auth_init() {
    let store = KeyStore::load(None, None);
    let mut keys = get_keys_mutex().lock().unwrap();
    *keys = store.keys;
}

/// Command-line helper to generate key pair at specified filename.
/// Allows overwriting existing file matching `adb keygen` commandline behavior in C++ ADB.
///
/// Ported from `adb_auth_keygen` in `original/client/auth.cpp`.
pub fn adb_auth_keygen<P: AsRef<Path>>(filename: P) -> Result<(), String> {
    generate_key_opts(filename, true)
}

/// Command-line helper to print public key string for a key file.
///
/// Ported from `adb_auth_pubkey` in `original/client/auth.cpp`.
pub fn adb_auth_pubkey<P: AsRef<Path>>(filename: P) -> Result<String, String> {
    let key = read_key_file(filename)?;
    calculate_public_key(&key)
}

/// Retrieve all currently loaded private keys.
///
/// Ported from `adb_auth_get_private_keys` in `original/client/auth.cpp`.
pub fn adb_auth_get_private_keys() -> Vec<RsaPrivateKey> {
    let keys = get_keys_mutex().lock().unwrap();
    keys.values().cloned().collect()
}
