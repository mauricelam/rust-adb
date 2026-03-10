//! ADB cryptography utilities.
//! Ported from `crypto`.

use anyhow::anyhow;
use base64::Engine;
use num_bigint_dig::BigUint;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rsa::{traits::PublicKeyParts, RsaPrivateKey};

/// Represents a cryptographic key, wrapping an RSA private key.
/// Ported from `crypto/include/adb/crypto/key.h`.
pub struct Key(RsaPrivateKey);

/// Public key in Android's custom format.
/// Ported from `AndroidPubkey` in `crypto/include/adb/crypto/key.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AndroidPubkey {
    /// Size of the modulus in 32-bit words.
    pub modulus_size_words: u32,
    /// -1 / n[0] mod 2^32.
    pub n0inv: u32,
    /// The modulus as a little-endian byte array.
    pub modulus: [u8; 256],
    /// R^2 mod N.
    pub rr: [u8; 256],
    /// The public exponent.
    pub exponent: u32,
}

impl Key {
    /// Creates a `Key` from a PKCS#8 PEM-encoded string.
    pub fn from_pem(pem: &str) -> anyhow::Result<Self> {
        let key = RsaPrivateKey::from_pkcs8_pem(pem)?;
        Ok(Key(key))
    }

    /// Returns a reference to the underlying RSA private key.
    pub fn privkey(&self) -> &RsaPrivateKey {
        &self.0
    }

    /// Calculate the public key in the android format.
    /// This is a custom format that consists of a C-style struct with the
    /// following fields:
    ///    modulus_size_words: u32,
    ///    n0inv: u32,
    ///    modulus: [u32; 64],
    ///    rr: [u32; 64],
    ///    exponent: u32,
    pub fn android_pubkey(&self) -> anyhow::Result<AndroidPubkey> {
        let n = self.0.n();
        let e = self.0.e();

        let mut n_bytes = n.to_bytes_le();
        if n_bytes.len() > 256 {
            return Err(anyhow!("Only up to 2048-bit RSA keys are supported"));
        }
        n_bytes.resize(256, 0);

        // n0inv = -1 / n[0] mod 2^32
        let n0 = u32::from_le_bytes(n_bytes[0..4].try_into()?);
        let n0inv = calculate_n0inv(n0);

        // rr = R^2 mod N, where R = 2^2048
        let r = BigUint::from(1u32) << 2048;
        let rr = (&r * &r) % n;
        let mut rr_bytes = rr.to_bytes_le();
        rr_bytes.resize(256, 0);

        let mut modulus = [0u8; 256];
        modulus.copy_from_slice(&n_bytes);

        let mut rr = [0u8; 256];
        rr.copy_from_slice(&rr_bytes);

        let e_bytes = e.to_bytes_le();
        let mut e_u32_bytes = [0u8; 4];
        let len = std::cmp::min(e_bytes.len(), 4);
        e_u32_bytes[..len].copy_from_slice(&e_bytes[..len]);
        let exponent = u32::from_le_bytes(e_u32_bytes);

        Ok(AndroidPubkey {
            modulus_size_words: 2048 / 32,
            n0inv,
            modulus,
            rr,
            exponent,
        })
    }

    /// Return the private key as a PEM encoded string.
    /// Ported from original/crypto/key.cpp: `std::string Key::ToPEMString(EVP_PKEY* pkey)`
    pub fn to_pem_string(&self) -> anyhow::Result<String> {
        let pem = self.0.to_pkcs8_pem(Default::default())?;
        Ok(pem.to_string())
    }

    /// Calculates the public key in the format "<pubkey> <user>@<host>".
    /// Ported from original/crypto/rsa_2048_key.cpp: `bool CalculatePublicKey(std::string* out, RSA* private_key)`
    pub fn calculate_public_key(&self) -> anyhow::Result<String> {
        let pubkey = self.android_pubkey()?;
        let mut buf = Vec::with_capacity(524);
        buf.extend_from_slice(&pubkey.modulus_size_words.to_le_bytes());
        buf.extend_from_slice(&pubkey.n0inv.to_le_bytes());
        buf.extend_from_slice(&pubkey.modulus);
        buf.extend_from_slice(&pubkey.rr);
        buf.extend_from_slice(&pubkey.exponent.to_le_bytes());

        let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
        let login = sysdeps::env::get_login_name_utf8().unwrap_or_else(|_| "unknown".to_string());
        let host = sysdeps::env::get_host_name_utf8().unwrap_or_else(|_| "unknown".to_string());

        Ok(format!("{} {}@{}", encoded, login, host))
    }
}

fn calculate_n0inv(n0: u32) -> u32 {
    let mut inv = 1u32;
    for _ in 0..32 {
        inv = inv.wrapping_mul(2u32.wrapping_sub(n0.wrapping_mul(inv)));
    }
    inv
}

/// Length of a SHA256 digest in bytes.
pub const SHA256_DIGEST_LENGTH: usize = 32;

/// Converts SHA256 bits to a hex string representation.
/// Ported from `SHA256BitsToHexString` in `tls/adb_ca_list.cpp`.
pub fn sha256_bits_to_hex_string(sha256: &[u8]) -> String {
    assert_eq!(sha256.len(), SHA256_DIGEST_LENGTH);
    let mut s = String::with_capacity(SHA256_DIGEST_LENGTH * 2);
    for &b in sha256 {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

/// Converts a SHA256 hex string back to bits.
/// Ported from `SHA256HexStringToBits` in `tls/adb_ca_list.cpp`.
pub fn sha256_hex_string_to_bits(sha256_str: &str) -> Option<Vec<u8>> {
    if sha256_str.len() != SHA256_DIGEST_LENGTH * 2 {
        return None;
    }
    let mut res = Vec::with_capacity(SHA256_DIGEST_LENGTH);
    for i in 0..SHA256_DIGEST_LENGTH {
        let s = &sha256_str[i * 2..i * 2 + 2];
        let b = u8::from_str_radix(s, 16).ok()?;
        res.push(b);
    }
    Some(res)
}

use rcgen::{Certificate, DistinguishedName};

/// Generates a new 2048-bit RSA key.
/// Ported from `CreateRSA2048Key` in `crypto/rsa_2048_key.cpp`.
pub fn new_rsa_2048() -> anyhow::Result<Key> {
    let mut rng = rand::thread_rng();
    let key = RsaPrivateKey::new(&mut rng, 2048)?;
    Ok(Key(key))
}

/// Generates a self-signed X.509 certificate for the given key.
/// Ported from `GenerateX509Certificate` in `crypto/x509_generator.cpp`.
pub fn generate_x509_certificate(key: &Key) -> anyhow::Result<Certificate> {
    let mut params = rcgen::CertificateParams::default();
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(rcgen::DnType::CountryName, "US");
    distinguished_name.push(rcgen::DnType::OrganizationName, "Android");
    distinguished_name.push(rcgen::DnType::CommonName, "Adb");
    params.distinguished_name = distinguished_name;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    params.alg = &rcgen::PKCS_RSA_SHA256;

    let key_pair = rcgen::KeyPair::from_pem(&key.to_pem_string()?)?;
    params.key_pair = Some(key_pair);

    let cert = Certificate::from_params(params)?;
    Ok(cert)
}

/// Returns the X.509 certificate as a PEM encoded string.
/// Ported from `X509ToPEMString` in `crypto/x509_generator.cpp`.
pub fn x509_to_pem_string(cert: &Certificate) -> anyhow::Result<String> {
    Ok(cert.serialize_pem()?)
}

/// Creates a CA issuer Distinguished Name from an encoded public key.
/// Ported from `CreateCAIssuerFromEncodedKey` in `tls/adb_ca_list.cpp`.
pub fn create_ca_issuer_from_encoded_key(key: &str) -> anyhow::Result<Vec<u8>> {
    if key.is_empty() {
        return Err(anyhow!("Key cannot be empty"));
    }

    let mut params = rcgen::CertificateParams::default();
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(rcgen::DnType::OrganizationName, "AdbKey-0");
    distinguished_name.push(rcgen::DnType::CommonName, key);
    params.distinguished_name = distinguished_name;

    let cert = Certificate::from_params(params)?;
    let der = cert.serialize_der()?;
    let (_, parsed_cert) = x509_parser::parse_x509_certificate(&der)
        .map_err(|e| anyhow!("Failed to parse certificate: {}", e))?;

    Ok(parsed_cert.tbs_certificate.subject.as_raw().to_vec())
}

use x509_parser::prelude::FromDer;

/// Parses an encoded public key from a CA issuer Distinguished Name.
/// Ported from `ParseEncodedKeyFromCAIssuer` in `tls/adb_ca_list.cpp`.
pub fn parse_encoded_key_from_ca_issuer(der: &[u8]) -> anyhow::Result<Option<String>> {
    let (_, subject) = x509_parser::x509::X509Name::from_der(der)
        .map_err(|e| anyhow!("Failed to parse Name: {}", e))?;

    let mut is_adb_key = false;
    let mut key = None;

    for rdns in subject.iter_rdn() {
        for attr in rdns.iter() {
            let oid = attr.attr_type().to_string();
            // 2.5.4.10 is OrganizationName
            if oid == "2.5.4.10" {
                if let Ok(val) = attr.attr_value().as_str() {
                    if val == "AdbKey-0" {
                        is_adb_key = true;
                    }
                }
            }
            // 2.5.4.3 is CommonName
            if oid == "2.5.4.3" {
                if let Ok(val) = attr.attr_value().as_str() {
                    key = Some(val.to_string());
                }
            }
        }
    }

    if is_adb_key {
        Ok(key)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use rsa::pkcs1v15;
    use rsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
    use sha1::{Digest, Sha1};

    #[test]
    fn smoke() {
        let key = new_rsa_2048().unwrap();
        let pubkey_struct = key.android_pubkey().unwrap();
        assert_eq!(pubkey_struct.modulus_size_words, 64);

        // SAFETY: AndroidPubkey is #[repr(C)] and has a fixed size.
        let pubkey_bytes: [u8; 524] = unsafe { std::mem::transmute(pubkey_struct) };
        let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(&pubkey_bytes);
        println!("pubkey_b64: {}", pubkey_b64);

        let pem = key.to_pem_string().unwrap();
        assert!(!pem.is_empty());

        // Sign something and verify it.
        let data = b"abcdefghij123456789";
        let hashed = Sha1::digest(data);
        let signing_key = pkcs1v15::SigningKey::<Sha1>::new_unprefixed(key.0.clone());
        let signature = signing_key.sign_prehash(&hashed).unwrap();

        let verifying_key =
            pkcs1v15::VerifyingKey::<Sha1>::new_unprefixed(signing_key.as_ref().to_public_key());
        assert!(verifying_key.verify_prehash(&hashed, &signature).is_ok());
    }

    #[test]
    fn x509() {
        let key = new_rsa_2048().unwrap();
        let cert = generate_x509_certificate(&key).unwrap();
        let pem = x509_to_pem_string(&cert).unwrap();
        assert!(!pem.is_empty());

        // Check that the cert is signed with the correct key.
        let key_pair = rcgen::KeyPair::from_pem(&key.to_pem_string().unwrap()).unwrap();
        assert_eq!(
            cert.get_key_pair().public_key_raw(),
            key_pair.public_key_raw()
        );
    }

    #[test]
    fn test_calculate_public_key() {
        let key = new_rsa_2048().unwrap();
        let pubkey_plus_name = key.calculate_public_key().unwrap();
        let split: Vec<&str> = pubkey_plus_name.split_whitespace().collect();
        assert_eq!(split.len(), 2);
        assert!(split[1].contains('@'));

        let pubkey_b64 = split[0];
        let pubkey_bytes = base64::engine::general_purpose::STANDARD.decode(pubkey_b64).unwrap();
        assert_eq!(pubkey_bytes.len(), 524);
    }

    #[test]
    fn test_sha256_utils() {
        let mut bits = Vec::new();
        for i in 0..SHA256_DIGEST_LENGTH {
            bits.push(i as u8);
        }

        let hex = sha256_bits_to_hex_string(&bits);
        assert_eq!(
            hex,
            "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F"
        );

        let out_bits = sha256_hex_string_to_bits(&hex).unwrap();
        assert_eq!(bits, out_bits);

        assert!(sha256_hex_string_to_bits("").is_none());
        assert!(sha256_hex_string_to_bits("G0").is_none());
    }

    #[test]
    fn test_ca_list_smoke() {
        let key = "A45BC1FF6C89BF0E65F9BA153FBC98764969B4113F1CF878EEF9BF1C3F9C9227";
        let der = create_ca_issuer_from_encoded_key(key).unwrap();
        let out_key = parse_encoded_key_from_ca_issuer(&der).unwrap().unwrap();
        assert_eq!(key, out_key);
    }

    #[test]
    fn test_ca_list_empty_key() {
        assert!(create_ca_issuer_from_encoded_key("").is_err());
    }
}
