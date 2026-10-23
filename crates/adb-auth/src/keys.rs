//! Key management for ADB: finding, loading, generating, and saving RSA keys.
//!
//! Ported from:
//! - `original/client/auth.cpp`: `generate_key`, `load_key`, `load_keys`, `get_vendor_keys`,
//!   `get_user_key_path`, `read_key_file`, `hash_key`
//! - `original/adb_auth.h`: public key/auth interfaces
//! - `original/crypto/rsa_2048_key.cpp`: `CalculatePublicKey` and `CreateRSA2048Key`

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use num_bigint_dig::BigUint;
use num_traits::One;
use pkcs8::DecodePrivateKey;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};

pub const TOKEN_SIZE: usize = 20;
pub const ANDROID_PUBKEY_MODULUS_SIZE: usize = 2048 / 8; // 256 bytes
pub const ANDROID_PUBKEY_MODULUS_SIZE_WORDS: usize = ANDROID_PUBKEY_MODULUS_SIZE / 4; // 64 words
pub const ANDROID_PUBKEY_ENCODED_SIZE: usize =
    4 + 4 + ANDROID_PUBKEY_MODULUS_SIZE + ANDROID_PUBKEY_MODULUS_SIZE + 4; // 524 bytes

/// Errors that can occur during key operations.
#[derive(Debug)]
pub enum KeyError {
    /// An I/O error occurred.
    Io(std::io::Error),
    /// Failed to parse a PEM-encoded key.
    ParsePem(String),
    /// RSA key modulus is not 2048 bits.
    UnsupportedModulusSize {
        expected_bits: usize,
        actual_bits: usize,
    },
    /// Public exponent is unsupported or exceeds 32 bits.
    UnsupportedExponent(String),
    /// Public key buffer is too short.
    BufferTooShort { expected: usize, actual: usize },
    /// Invalid modulus size in words.
    InvalidModulusSizeWords { expected: u32, actual: u32 },
    /// Could not determine user home directory.
    HomeDirNotFound,
    /// Key generation failed.
    KeyGeneration(String),
    /// Public key encoding or formatting failed.
    PublicKeyEncoding(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Io(err) => write!(f, "I/O error: {}", err),
            KeyError::ParsePem(msg) => write!(f, "Failed to parse PEM key: {}", msg),
            KeyError::UnsupportedModulusSize {
                expected_bits,
                actual_bits,
            } => {
                write!(
                    f,
                    "RSA public key must be {} bits, got {}",
                    expected_bits, actual_bits
                )
            }
            KeyError::UnsupportedExponent(msg) => write!(f, "Unsupported public exponent: {}", msg),
            KeyError::BufferTooShort { expected, actual } => {
                write!(
                    f,
                    "Buffer too short: expected at least {} bytes, got {}",
                    expected, actual
                )
            }
            KeyError::InvalidModulusSizeWords { expected, actual } => {
                write!(
                    f,
                    "Invalid modulus size in words: expected {}, got {}",
                    expected, actual
                )
            }
            KeyError::HomeDirNotFound => write!(
                f,
                "Could not determine user home directory (HOME or USERPROFILE not set)"
            ),
            KeyError::KeyGeneration(msg) => write!(f, "Key generation failed: {}", msg),
            KeyError::PublicKeyEncoding(msg) => write!(f, "Public key encoding failed: {}", msg),
        }
    }
}

impl std::error::Error for KeyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            KeyError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for KeyError {
    fn from(err: std::io::Error) -> Self {
        KeyError::Io(err)
    }
}

impl PartialEq for KeyError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (KeyError::ParsePem(a), KeyError::ParsePem(b)) => a == b,
            (
                KeyError::UnsupportedModulusSize {
                    expected_bits: e1,
                    actual_bits: a1,
                },
                KeyError::UnsupportedModulusSize {
                    expected_bits: e2,
                    actual_bits: a2,
                },
            ) => e1 == e2 && a1 == a2,
            (KeyError::UnsupportedExponent(a), KeyError::UnsupportedExponent(b)) => a == b,
            (
                KeyError::BufferTooShort {
                    expected: e1,
                    actual: a1,
                },
                KeyError::BufferTooShort {
                    expected: e2,
                    actual: a2,
                },
            ) => e1 == e2 && a1 == a2,
            (
                KeyError::InvalidModulusSizeWords {
                    expected: e1,
                    actual: a1,
                },
                KeyError::InvalidModulusSizeWords {
                    expected: e2,
                    actual: a2,
                },
            ) => e1 == e2 && a1 == a2,
            (KeyError::HomeDirNotFound, KeyError::HomeDirNotFound) => true,
            (KeyError::KeyGeneration(a), KeyError::KeyGeneration(b)) => a == b,
            (KeyError::PublicKeyEncoding(a), KeyError::PublicKeyEncoding(b)) => a == b,
            (KeyError::Io(a), KeyError::Io(b)) => a.kind() == b.kind(),
            _ => false,
        }
    }
}

impl Eq for KeyError {}

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

/// Signs an authentication token challenge using the provided RSA private key with blinded exponentiation.
///
/// Ported from `adb_auth_sign` in `original/client/auth.cpp`.
///
/// The token must be exactly `TOKEN_SIZE` (20) bytes. The token is treated as an
/// already-computed digest (e.g. SHA-1) and signed using PKCS#1 v1.5 with SHA-1
/// without re-hashing the token bytes. Private key operation uses `sign_with_rng` to blind
/// timing side-channels.
pub fn sign_token(key: &RsaPrivateKey, token: &[u8]) -> Result<Vec<u8>, SignError> {
    if token.len() != TOKEN_SIZE {
        return Err(SignError::InvalidTokenSize(token.len()));
    }

    let scheme = rsa::Pkcs1v15Sign::new::<sha1::Sha1>();
    let mut rng = rand::thread_rng();
    key.sign_with_rng(&mut rng, scheme, token)
        .map_err(|e| SignError::SigningFailed(e.to_string()))
}

/// Compute SHA-256 fingerprint of an RSA private key based on its SubjectPublicKeyInfo DER encoding.
///
/// Ported from `hash_key` in `original/client/auth.cpp`.
pub fn fingerprint(key: &RsaPrivateKey) -> Result<[u8; 32], KeyError> {
    let pub_key = key.to_public_key();
    let der = pub_key
        .to_public_key_der()
        .map_err(|e| KeyError::PublicKeyEncoding(e.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(der.as_bytes());
    Ok(hasher.finalize().into())
}

/// Compute SHA-256 fingerprint of an RSA private key as a `Vec<u8>`.
///
/// Ported from `hash_key` in `original/client/auth.cpp`.
pub fn hash_key(key: &RsaPrivateKey) -> Result<Vec<u8>, KeyError> {
    fingerprint(key).map(|fp| fp.to_vec())
}

/// Store for ADB authentication RSA private keys, supporting standard user key
/// loading/generation and extra vendor key paths from `ADB_VENDOR_KEYS`.
///
/// Keys are stored deduplicated by fingerprint and ordered in ascending byte order
/// of their SHA-256 SubjectPublicKeyInfo fingerprint, matching C++ ADB's `std::map` ordering.
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
                if let Ok(fp) = fingerprint(&key) {
                    store.keys.insert(fp, key.clone());
                    store.user_key = Some(key);
                }
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
    pub fn load_vendor_path(&mut self, path: &Path) {
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
    pub fn load_key_file(&mut self, path: &Path) {
        if let Ok(key) = read_key_file(path) {
            if let Ok(fp) = fingerprint(&key) {
                self.keys.entry(fp).or_insert(key);
            }
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

/// Returns standard default comment `login@host`.
///
/// Ported from `GetLoginNameUTF8` and `GetHostNameUTF8` in `original/sysdeps/env.cpp`.
pub fn default_comment() -> String {
    let login = get_login_name_utf8();
    let login = if login.is_empty() {
        "unknown".to_string()
    } else {
        login
    };

    let host = get_host_name_utf8();
    let host = if host.is_empty() {
        "unknown".to_string()
    } else {
        host
    };

    format!("{}@{}", login, host)
}

fn get_login_name_utf8() -> String {
    let env_var = if cfg!(windows) { "USERNAME" } else { "LOGNAME" };
    if let Ok(val) = env::var(env_var) {
        if !val.trim().is_empty() {
            return val;
        }
    }
    if !cfg!(windows) {
        if let Ok(val) = env::var("USER") {
            if !val.trim().is_empty() {
                return val;
            }
        }
    }

    #[cfg(unix)]
    {
        let ptr = unsafe { libc::getlogin() };
        if !ptr.is_null() {
            if let Ok(cstr) = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_str() {
                if !cstr.is_empty() {
                    return cstr.to_string();
                }
            }
        }
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::WindowsProgramming::GetUserNameW;
        let mut buf = [0u16; 256];
        let mut size = buf.len() as u32;
        if unsafe { GetUserNameW(buf.as_mut_ptr(), &mut size) } != 0 && size > 0 {
            let len = (size as usize).saturating_sub(1);
            if let Ok(s) = String::from_utf16(&buf[..len]) {
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }

    String::new()
}

fn get_host_name_utf8() -> String {
    let env_var = if cfg!(windows) {
        "COMPUTERNAME"
    } else {
        "HOSTNAME"
    };
    if let Ok(val) = env::var(env_var) {
        if !val.trim().is_empty() {
            return val;
        }
    }

    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let res = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
        if res == 0 {
            if let Ok(cstr) = std::ffi::CStr::from_bytes_until_nul(&buf) {
                if let Ok(s) = cstr.to_str() {
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;
        let mut buf = [0u16; 256];
        let mut size = buf.len() as u32;
        if unsafe { GetComputerNameW(buf.as_mut_ptr(), &mut size) } != 0 && size > 0 {
            if let Ok(s) = String::from_utf16(&buf[..size as usize]) {
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }

    String::new()
}

/// Encodes an RSA 2048 public key into Android's binary pubkey structure (524 bytes).
///
/// Format (little-endian):
/// - `modulus_size_words`: u32 (64 for 2048-bit RSA)
/// - `n0inv`: u32 (-n^-1 mod 2^32 where n is modulus)
/// - `modulus`: [u8; 256] (modulus n in little-endian order)
/// - `rr`: [u8; 256] (Montgomery parameter R^2 mod n in little-endian order)
/// - `exponent`: u32 (public exponent e, 3 or 65537)
pub fn encode_public_key(
    key: &RsaPublicKey,
) -> Result<[u8; ANDROID_PUBKEY_ENCODED_SIZE], KeyError> {
    let n = key.n();
    let e = key.e();

    let n_bytes = n.to_bytes_le();
    if n.bits() != 2048 || n_bytes.len() > ANDROID_PUBKEY_MODULUS_SIZE {
        return Err(KeyError::UnsupportedModulusSize {
            expected_bits: 2048,
            actual_bits: n.bits(),
        });
    }

    let e_bytes = e.to_bytes_le();
    if e_bytes.len() > 4 {
        return Err(KeyError::UnsupportedExponent(e.to_string()));
    }

    let mut e_u32 = 0u32;
    for (i, &b) in e_bytes.iter().enumerate() {
        e_u32 |= (b as u32) << (i * 8);
    }
    if e_u32 != 3 && e_u32 != 65537 {
        return Err(KeyError::UnsupportedExponent(e.to_string()));
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
pub fn decode_public_key(bytes: &[u8]) -> Result<(BigUint, u32), KeyError> {
    if bytes.len() < ANDROID_PUBKEY_ENCODED_SIZE {
        return Err(KeyError::BufferTooShort {
            expected: ANDROID_PUBKEY_ENCODED_SIZE,
            actual: bytes.len(),
        });
    }

    let modulus_size_words = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if modulus_size_words != ANDROID_PUBKEY_MODULUS_SIZE_WORDS as u32 {
        return Err(KeyError::InvalidModulusSizeWords {
            expected: ANDROID_PUBKEY_MODULUS_SIZE_WORDS as u32,
            actual: modulus_size_words,
        });
    }

    let exponent = u32::from_le_bytes(bytes[520..524].try_into().unwrap());
    if exponent != 3 && exponent != 65537 {
        return Err(KeyError::UnsupportedExponent(exponent.to_string()));
    }

    let modulus_bytes = &bytes[8..264];
    let modulus = BigUint::from_bytes_le(modulus_bytes);

    Ok((modulus, exponent))
}

/// Formats public key string representation (`<base64> <comment>`).
pub fn public_key_line(key: &RsaPublicKey, comment: &str) -> Result<String, KeyError> {
    let encoded = encode_public_key(key)?;
    let b64 = BASE64.encode(encoded);
    Ok(format!("{} {}", b64, comment))
}

/// Legacy alias / helper for private key public key calculation.
pub fn encode_android_pubkey(key: &RsaPrivateKey) -> Result<Vec<u8>, KeyError> {
    let pub_key = key.to_public_key();
    let encoded = encode_public_key(&pub_key)?;
    Ok(encoded.to_vec())
}

/// Calculate the string representation of an RSA public key (`<base64> <user>@<host>`).
///
/// Ported from `CalculatePublicKey` in `original/crypto/rsa_2048_key.cpp`.
pub fn calculate_public_key(key: &RsaPrivateKey) -> Result<String, KeyError> {
    let pub_key = key.to_public_key();
    let comment = default_comment();
    public_key_line(&pub_key, &comment)
}

/// Read an RSA private key from a PEM file (supporting PKCS#1 and PKCS#8).
///
/// Ported from `read_key_file` in `original/client/auth.cpp`.
pub fn read_key_file<P: AsRef<Path>>(path: P) -> Result<RsaPrivateKey, KeyError> {
    let path = path.as_ref();
    let content = fs::read_to_string(path)?;

    // Try reading as PKCS#8 PEM first
    if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(&content) {
        return Ok(key);
    }

    // Try reading as PKCS#1 PEM
    if let Ok(key) = rsa::pkcs1::DecodeRsaPrivateKey::from_pkcs1_pem(&content) {
        return Ok(key);
    }

    Err(KeyError::ParsePem(format!(
        "Failed to parse private key from '{}'",
        path.display()
    )))
}

/// Get standard Android home directory path (e.g. `$HOME` on Unix or `%USERPROFILE%` on Windows).
fn get_homedir_path() -> Result<PathBuf, KeyError> {
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
    Err(KeyError::HomeDirNotFound)
}

/// Get standard Android user directory path (`~/.android`).
/// Pure getter: does not create directory or fall back to current directory.
///
/// Ported from `adb_get_android_dir_path` in `original/adb_utils.cpp`.
pub fn adb_get_android_dir_path() -> Result<PathBuf, KeyError> {
    let home = get_homedir_path()?;
    Ok(home.join(".android"))
}

/// Ensures `.android` directory exists with 0750 permissions on Unix.
pub fn ensure_android_dir() -> Result<PathBuf, KeyError> {
    let dir = adb_get_android_dir_path()?;
    ensure_dir_0750(&dir)?;
    Ok(dir)
}

/// Helper to create directory hierarchy with mode 0750 on Unix (matching `adb_mkdir(dir, 0750)`).
fn ensure_dir_0750(dir: &Path) -> Result<(), KeyError> {
    if dir.as_os_str().is_empty() || dir.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        builder.mode(0o750);
        builder.create(dir)?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

/// Get standard user key path (`~/.android/adbkey`).
///
/// Ported from `get_user_key_path` in `original/client/auth.cpp`.
pub fn get_user_key_path() -> Result<PathBuf, KeyError> {
    adb_get_android_dir_path().map(|p| p.join("adbkey"))
}

/// Generate an RSA 2048 private key and save `file` (private) and `file.pub` (public).
/// Owner-only permissions (0600 on Unix) are enforced on the private key.
/// Private key is formatted in PKCS#8 PEM format (`BEGIN PRIVATE KEY`).
///
/// If `overwrite` is `false`, fails with an error if `file` already exists.
/// Uses a temporary file and atomic rename to prevent leaving partial key files on crash/failure.
pub fn generate_key_opts<P: AsRef<Path>>(file: P, overwrite: bool) -> Result<(), KeyError> {
    let file = file.as_ref();

    if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
        ensure_dir_0750(parent)?;
    }

    let mut rng = rand::thread_rng();
    let key =
        RsaPrivateKey::new(&mut rng, 2048).map_err(|e| KeyError::KeyGeneration(e.to_string()))?;

    let pubkey_str = calculate_public_key(&key)?;

    let pem = key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|e| KeyError::KeyGeneration(e.to_string()))?;

    if overwrite {
        let rand_num: u32 = rand::random();
        let file_name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("adbkey");
        let tmp_file = file.with_file_name(format!(
            ".{}.tmp.{}.{}",
            file_name,
            std::process::id(),
            rand_num
        ));

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut f = options.open(&tmp_file)?;

        if let Err(e) = f.write_all(pem.as_bytes()) {
            let _ = fs::remove_file(&tmp_file);
            return Err(e.into());
        }
        if let Err(e) = f.sync_all() {
            let _ = fs::remove_file(&tmp_file);
            return Err(e.into());
        }
        drop(f);

        if let Err(e) = fs::rename(&tmp_file, file) {
            let _ = fs::remove_file(&tmp_file);
            return Err(e.into());
        }

        let pub_path = PathBuf::from(format!("{}.pub", file.display()));
        let tmp_pub_path = file.with_file_name(format!(
            ".{}.pub.tmp.{}.{}",
            file_name,
            std::process::id(),
            rand_num
        ));
        if let Err(e) = fs::write(&tmp_pub_path, &pubkey_str) {
            let _ = fs::remove_file(&tmp_pub_path);
            return Err(e.into());
        }
        let _ = fs::rename(&tmp_pub_path, &pub_path);
    } else {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut f = options.open(file)?;

        if let Err(e) = f.write_all(pem.as_bytes()) {
            let _ = fs::remove_file(file);
            return Err(e.into());
        }
        if let Err(e) = f.sync_all() {
            let _ = fs::remove_file(file);
            return Err(e.into());
        }
        drop(f);

        let pub_path = PathBuf::from(format!("{}.pub", file.display()));
        if let Err(e) = fs::write(&pub_path, &pubkey_str) {
            let _ = fs::remove_file(file);
            let _ = fs::remove_file(&pub_path);
            return Err(e.into());
        }
    }

    Ok(())
}

/// Generate an RSA 2048 private key and save `file` (private) and `file.pub` (public).
/// Owner-only permissions (0600 on Unix) are enforced on the private key.
/// Never overwrites an existing key file (`overwrite = false`).
///
/// Ported from `generate_key` in `original/client/auth.cpp`.
pub fn generate_key<P: AsRef<Path>>(file: P) -> Result<(), KeyError> {
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
pub fn load_or_generate<P: AsRef<Path>>(path: P) -> Result<RsaPrivateKey, KeyError> {
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

/// Command-line helper to generate key pair at specified filename.
/// Allows overwriting existing file matching `adb keygen` commandline behavior in C++ ADB.
///
/// Ported from `adb_auth_keygen` in `original/client/auth.cpp`.
pub fn adb_auth_keygen<P: AsRef<Path>>(filename: P) -> Result<(), KeyError> {
    generate_key_opts(filename, true)
}

/// Command-line helper to print public key string for a key file.
///
/// Ported from `adb_auth_pubkey` in `original/client/auth.cpp`.
pub fn adb_auth_pubkey<P: AsRef<Path>>(filename: P) -> Result<String, KeyError> {
    let key = read_key_file(filename)?;
    calculate_public_key(&key)
}
