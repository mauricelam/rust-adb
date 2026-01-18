use anyhow::Result;
use num_bigint_dig::{BigUint, ModInverse};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey};
use std::convert::TryInto;
use std::io::Write;

pub struct Key(RsaPrivateKey);

const ANDROID_PUBKEY_MODULUS_SIZE: usize = 2048 / 8;
const ANDROID_PUBKEY_MODULUS_SIZE_WORDS: u32 = (ANDROID_PUBKEY_MODULUS_SIZE / 4) as u32;

#[repr(C)]
struct RSAPublicKey {
    modulus_size_words: u32,
    n0inv: u32,
    modulus: [u8; ANDROID_PUBKEY_MODULUS_SIZE],
    rr: [u8; ANDROID_PUBKEY_MODULUS_SIZE],
    exponent: u32,
}

impl Key {
    /// Calculate the public key in the android format.
    pub fn android_pubkey(&self) -> Result<Vec<u8>> {
        let n = self.0.n();
        let e = self.0.e();

        let mut n_le = n.to_bytes_le();
        n_le.resize(ANDROID_PUBKEY_MODULUS_SIZE, 0);

        // Calculate n0inv = -1 / n[0] mod 2^32
        let n0 = BigUint::from_bytes_le(&n_le[0..4]);
        let r = BigUint::from(0x100000000u64);
        let n0inv = n0.mod_inverse(r).unwrap();
        let mut n0inv_bytes = n0inv.to_bytes_le().1;
        n0inv_bytes.resize(4, 0);
        let n0inv_u32 = u32::from_le_bytes(n0inv_bytes.try_into().unwrap());
        let n0inv = 0x100000000u64 - n0inv_u32 as u64;

        // Calculate rr = (2^2048)^2 mod N
        let r = BigUint::from(1u64) << 2048;
        let n_biguint = BigUint::from_bytes_be(n.to_bytes_be().as_slice());
        let rr = (&r * &r) % n_biguint;

        let mut rr_le = rr.to_bytes_le();
        rr_le.resize(ANDROID_PUBKEY_MODULUS_SIZE, 0);

        let mut e_bytes = e.to_bytes_le();
        e_bytes.resize(4, 0);
        let exponent = u32::from_le_bytes(e_bytes.try_into().unwrap());

        let key = RSAPublicKey {
            modulus_size_words: ANDROID_PUBKEY_MODULUS_SIZE_WORDS,
            n0inv: n0inv as u32,
            modulus: n_le.try_into().unwrap(),
            rr: rr_le.try_into().unwrap(),
            exponent,
        };

        let mut pubkey = Vec::new();
        pubkey.write_all(unsafe {
            std::slice::from_raw_parts(
                &key as *const _ as *const u8,
                std::mem::size_of::<RSAPublicKey>(),
            )
        })?;
        Ok(pubkey)
    }

    /// Return the private key as a PEM encoded string.
    pub fn to_pem_string(&self) -> Result<String> {
        let pem = self.0.to_pkcs8_pem(Default::default())?;
        Ok(pem.to_string())
    }
}

use rcgen::{Certificate, DistinguishedName, KeyPair};

pub fn new_rsa_2048() -> Result<Key> {
    let mut rng = rand::thread_rng();
    let key = RsaPrivateKey::new(&mut rng, 2048)?;
    Ok(Key(key))
}

pub fn generate_x509_certificate(key: &Key) -> Result<Certificate> {
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

    let key_pair = KeyPair::from_pem(&key.to_pem_string()?)?;
    params.key_pair = Some(key_pair);

    let cert = Certificate::from_params(params)?;
    Ok(cert)
}

pub fn x509_to_pem_string(cert: &Certificate) -> Result<String> {
    Ok(cert.serialize_pem()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose;
    use base64::Engine;
    use rsa::pkcs1v15;
    use rsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
    use sha1::{Digest, Sha1};

    #[test]
    fn smoke() {
        let key = new_rsa_2048().unwrap();
        let pubkey = key.android_pubkey().unwrap();
        assert_eq!(pubkey.len(), 524);

        let pubkey_b64 = general_purpose::STANDARD.encode(&pubkey);
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
}
