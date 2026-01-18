use anyhow::Result;
use rsa::pkcs8::EncodePrivateKey;
use rsa::{RsaPrivateKey, RsaPublicKey};

pub struct Key(RsaPrivateKey);

impl Key {
    /// Calculate the public key in the android format.
    /// This is a custom format that consists of a C-style struct with the
    /// following fields:
    ///    modulus_size_words: u32,
    ///    n0inv: u32,
    ///    modulus: [u8; 256],
    ///    rr: [u8; 256],
    ///    exponent: u32,
    pub fn android_pubkey(&self) -> Result<RsaPublicKey> {
        Ok(self.0.to_public_key())
    }

    /// Return the private key as a PEM encoded string.
    pub fn to_pem_string(&self) -> Result<String> {
        let pem = self.0.to_pkcs8_pem(Default::default())?;
        Ok(pem.to_string())
    }
}

use rcgen::{Certificate, DistinguishedName};

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

    let key_pair = rcgen::KeyPair::from_pem(&key.to_pem_string()?)?;
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
    use rsa::pkcs8::EncodePublicKey;
    use rsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
    use sha1::{Digest, Sha1};

    #[test]
    fn smoke() {
        let key = new_rsa_2048().unwrap();
        let pubkey = key.android_pubkey().unwrap();
        let pubkey_der = pubkey.to_public_key_der().unwrap();
        assert_eq!(pubkey_der.as_bytes().len(), 294);

        let pubkey_b64 = general_purpose::STANDARD.encode(&pubkey_der);
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
