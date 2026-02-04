use anyhow::anyhow;
use num_bigint_dig::BigUint;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rsa::{traits::PublicKeyParts, RsaPrivateKey};

pub struct Key(RsaPrivateKey);

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AndroidPubkey {
    pub modulus_size_words: u32,
    pub n0inv: u32,
    pub modulus: [u8; 256],
    pub rr: [u8; 256],
    pub exponent: u32,
}

impl Key {
    pub fn from_pem(pem: &str) -> anyhow::Result<Self> {
        let key = RsaPrivateKey::from_pkcs8_pem(pem)?;
        Ok(Key(key))
    }

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
    pub fn to_pem_string(&self) -> anyhow::Result<String> {
        let pem = self.0.to_pkcs8_pem(Default::default())?;
        Ok(pem.to_string())
    }
}

fn calculate_n0inv(n0: u32) -> u32 {
    let mut inv = 1u32;
    for _ in 0..32 {
        inv = inv.wrapping_mul(2u32.wrapping_sub(n0.wrapping_mul(inv)));
    }
    inv
}

use rcgen::{Certificate, DistinguishedName};

pub fn new_rsa_2048() -> anyhow::Result<Key> {
    let mut rng = rand::thread_rng();
    let key = RsaPrivateKey::new(&mut rng, 2048)?;
    Ok(Key(key))
}

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

pub fn x509_to_pem_string(cert: &Certificate) -> anyhow::Result<String> {
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
        let pubkey_struct = key.android_pubkey().unwrap();
        assert_eq!(pubkey_struct.modulus_size_words, 64);

        // SAFETY: AndroidPubkey is #[repr(C)] and has a fixed size.
        let pubkey_bytes: [u8; 524] = unsafe { std::mem::transmute(pubkey_struct) };
        let pubkey_b64 = general_purpose::STANDARD.encode(&pubkey_bytes);
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
