use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub enum Role {
    Server,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsError {
    Success,
    CertificateRejected,
    PeerRejectedCertificate,
    UnknownFailure,
}

pub struct TlsConnection {
    role: Role,
    stream: TcpStream,
    config: Arc<Mutex<TlsConfig>>,
    connection: Option<rustls::Connection>,
    post_handshake_check: bool,
}

struct TlsConfig {
    trusted_certs: Vec<rustls::Certificate>,
    cert_verify_callback: Option<Box<dyn Fn() -> bool + Send + Sync>>,
    ca_list: Vec<Vec<u8>>,
    cert_callback: Option<Box<dyn Fn(&[Vec<u8>]) -> Option<(Vec<rustls::Certificate>, rustls::PrivateKey)> + Send + Sync>>,
    cert_chain: Vec<rustls::Certificate>,
    priv_key: rustls::PrivateKey,
}

impl TlsConnection {
    pub fn create(role: Role, cert: &str, priv_key: &str, stream: TcpStream) -> anyhow::Result<Self> {
        if cert.is_empty() || priv_key.is_empty() {
            return Err(anyhow::anyhow!("Empty cert or key"));
        }
        let cert_chain = rustls_pemfile::certs(&mut cert.as_bytes())?
            .into_iter()
            .map(rustls::Certificate)
            .collect::<Vec<_>>();
        let priv_key = rustls_pemfile::pkcs8_private_keys(&mut priv_key.as_bytes())?
            .into_iter()
            .map(rustls::PrivateKey)
            .next()
            .ok_or_else(|| anyhow::anyhow!("No private key found"))?;

        Ok(Self {
            role,
            stream,
            config: Arc::new(Mutex::new(TlsConfig {
                trusted_certs: Vec::new(),
                cert_verify_callback: None,
                ca_list: Vec::new(),
                cert_callback: None,
                cert_chain,
                priv_key,
            })),
            connection: None,
            post_handshake_check: false,
        })
    }

    pub fn add_trusted_certificate(&mut self, cert: &str) -> bool {
        if let Ok(certs) = rustls_pemfile::certs(&mut cert.as_bytes()) {
            let mut config = self.config.lock().unwrap();
            for c in certs {
                config.trusted_certs.push(rustls::Certificate(c));
            }
            true
        } else {
            false
        }
    }

    pub fn set_cert_verify_callback<F>(&mut self, cb: F)
    where
        F: Fn() -> bool + Send + Sync + 'static,
    {
        self.config.lock().unwrap().cert_verify_callback = Some(Box::new(cb));
    }

    pub fn set_client_ca_list(&mut self, ca_list: Vec<Vec<u8>>) {
        self.config.lock().unwrap().ca_list = ca_list;
    }

    pub fn set_certificate_callback<F>(&mut self, cb: F)
    where
        F: Fn(&[Vec<u8>]) -> Option<(Vec<rustls::Certificate>, rustls::PrivateKey)> + Send + Sync + 'static,
    {
        self.config.lock().unwrap().cert_callback = Some(Box::new(cb));
    }

    pub fn export_keying_material(&self, length: usize) -> Vec<u8> {
        let mut out = vec![0u8; length];
        if let Some(conn) = &self.connection {
            if conn
                .export_keying_material(&mut out, b"adb-label", None)
                .is_ok()
            {
                return out;
            }
        }
        Vec::new()
    }

    pub fn enable_client_post_handshake_check(&mut self, enable: bool) {
        self.post_handshake_check = enable;
    }

    pub fn do_handshake(&mut self) -> TlsError {
        let config_lock = self.config.lock().unwrap();

        let mut conn = match self.role {
            Role::Client => {
                let client_config = rustls::ClientConfig::builder()
                    .with_safe_defaults()
                    .with_custom_certificate_verifier(Arc::new(AdbCertVerifier {
                        config: self.config.clone(),
                    }))
                    .with_client_cert_resolver(Arc::new(AdbCertResolver {
                        config: self.config.clone(),
                    }));

                rustls::Connection::Client(
                    rustls::ClientConnection::new(Arc::new(client_config), "adb".try_into().unwrap())
                        .expect("failed to create client connection"),
                )
            }
            Role::Server => {
                let server_config = rustls::ServerConfig::builder()
                    .with_safe_defaults()
                    .with_client_cert_verifier(Arc::new(AdbCertVerifier {
                        config: self.config.clone(),
                    }))
                    .with_cert_resolver(Arc::new(AdbCertResolver {
                        config: self.config.clone(),
                    }));

                rustls::Connection::Server(
                    rustls::ServerConnection::new(Arc::new(server_config))
                        .expect("failed to create server connection"),
                )
            }
        };
        drop(config_lock);

        self.stream.set_nonblocking(false).unwrap();

        loop {
            if !conn.is_handshaking() {
                break;
            }

            match conn.complete_io(&mut self.stream) {
                Ok(_) => {}
                Err(_) => return TlsError::CertificateRejected,
            }
        }

        if self.post_handshake_check && matches!(self.role, Role::Client) {
             match conn.complete_io(&mut self.stream) {
                Ok(_) => {}
                Err(_) => return TlsError::PeerRejectedCertificate,
            }
        }

        self.connection = Some(conn);
        TlsError::Success
    }

    pub fn read_fully(&mut self, size: usize) -> Vec<u8> {
        let mut buf = vec![0u8; size];
        if self.read_fully_buf(&mut buf) {
            buf
        } else {
            Vec::new()
        }
    }

    pub fn read_fully_buf(&mut self, buf: &mut [u8]) -> bool {
        let conn = match &mut self.connection {
            Some(c) => c,
            None => return false,
        };

        let mut pos = 0;
        while pos < buf.len() {
            match conn.reader().read(&mut buf[pos..]) {
                Ok(n) if n > 0 => {
                    pos += n;
                }
                Ok(0) => {
                    match conn.complete_io(&mut self.stream) {
                        Ok(io) if io.0 == 0 && io.1 == 0 => return false, // EOF
                        Ok(_) => continue,
                        Err(_) => return false,
                    }
                }
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if conn.complete_io(&mut self.stream).is_err() {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
        true
    }

    pub fn write_fully(&mut self, data: &[u8]) -> bool {
        let conn = match &mut self.connection {
            Some(c) => c,
            None => return false,
        };

        let mut pos = 0;
        while pos < data.len() {
            match conn.writer().write(&data[pos..]) {
                Ok(n) if n > 0 => {
                    pos += n;
                }
                Ok(0) => break,
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if conn.complete_io(&mut self.stream).is_err() {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }

        while conn.wants_write() {
            if conn.complete_io(&mut self.stream).is_err() {
                return false;
            }
        }
        true
    }

    pub fn set_cert_and_key(_conn: &mut rustls::Connection, _cert: &str, _priv_key: &str) -> bool {
        true
    }
}

struct AdbCertVerifier {
    config: Arc<Mutex<TlsConfig>>,
}

impl rustls::client::ServerCertVerifier for AdbCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        let config = self.config.lock().unwrap();
        if let Some(cb) = &config.cert_verify_callback {
            if cb() {
                return Ok(rustls::client::ServerCertVerified::assertion());
            } else {
                return Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure));
            }
        }

        if config.trusted_certs.contains(end_entity) {
            return Ok(rustls::client::ServerCertVerified::assertion());
        }

        Err(rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer))
    }
}

impl rustls::server::ClientCertVerifier for AdbCertVerifier {
    fn client_auth_root_subjects(&self) -> &[rustls::DistinguishedName] {
        static EMPTY: Vec<rustls::DistinguishedName> = Vec::new();
        &EMPTY
    }

    fn verify_client_cert(
        &self,
        end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _now: SystemTime,
    ) -> Result<rustls::server::ClientCertVerified, rustls::Error> {
        let config = self.config.lock().unwrap();
        if let Some(cb) = &config.cert_verify_callback {
            if cb() {
                return Ok(rustls::server::ClientCertVerified::assertion());
            } else {
                return Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure));
            }
        }

        if config.trusted_certs.contains(end_entity) {
            return Ok(rustls::server::ClientCertVerified::assertion());
        }

        Err(rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer))
    }
}

struct AdbCertResolver {
    config: Arc<Mutex<TlsConfig>>,
}

impl rustls::server::ResolvesServerCert for AdbCertResolver {
    fn resolve(&self, _client_hello: rustls::server::ClientHello) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let config = self.config.lock().unwrap();
        let cert_chain = config.cert_chain.clone();
        let priv_key = config.priv_key.clone();
        let key = rustls::sign::any_supported_type(&priv_key).ok()?;
        Some(Arc::new(rustls::sign::CertifiedKey::new(cert_chain, key)))
    }
}

impl rustls::client::ResolvesClientCert for AdbCertResolver {
    fn resolve(
        &self,
        acceptable_issuers: &[&[u8]],
        _sigschemes: &[rustls::SignatureScheme],
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        let config = self.config.lock().unwrap();
        if let Some(cb) = &config.cert_callback {
            let issuers = acceptable_issuers.iter().map(|i| i.to_vec()).collect::<Vec<_>>();
            if let Some((certs, priv_key)) = cb(&issuers) {
                let key = rustls::sign::any_supported_type(&priv_key).ok()?;
                return Some(Arc::new(rustls::sign::CertifiedKey::new(certs, key)));
            }
        }
        let cert_chain = config.cert_chain.clone();
        let priv_key = config.priv_key.clone();
        let key = rustls::sign::any_supported_type(&priv_key).ok()?;
        Some(Arc::new(rustls::sign::CertifiedKey::new(cert_chain, key)))
    }

    fn has_certs(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn load_certs() -> (String, String, String, String) {
        let key = rust_adb_crypto::new_rsa_2048().unwrap();
        let cert = rust_adb_crypto::generate_x509_certificate(&key).unwrap();
        let server_cert = rust_adb_crypto::x509_to_pem_string(&cert).unwrap();
        let server_key = key.to_pem_string().unwrap();

        let key = rust_adb_crypto::new_rsa_2048().unwrap();
        let cert = rust_adb_crypto::generate_x509_certificate(&key).unwrap();
        let client_cert = rust_adb_crypto::x509_to_pem_string(&cert).unwrap();
        let client_key = key.to_pem_string().unwrap();

        (server_cert, server_key, client_cert, client_key)
    }

    #[test]
    fn test_invalid_creation_params() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        assert!(TlsConnection::create(Role::Server, "", "key", stream.try_clone().unwrap()).is_err());
        assert!(TlsConnection::create(Role::Server, "cert", "", stream).is_err());
    }

    #[test]
    fn test_no_trusted_certificates() {
        let (server_cert, server_key, client_cert, client_key) = load_certs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut server = TlsConnection::create(
                Role::Server,
                &server_cert,
                &server_key,
                stream,
            )
            .unwrap();
            server.do_handshake()
        });

        let client_stream = std::net::TcpStream::connect(addr).unwrap();
        let mut client = TlsConnection::create(
            Role::Client,
            &client_cert,
            &client_key,
            client_stream,
        )
        .unwrap();

        assert_eq!(client.do_handshake(), TlsError::CertificateRejected);
        assert_eq!(server_thread.join().unwrap(), TlsError::CertificateRejected);
    }

    #[test]
    fn test_add_trusted_certificates() {
        let (server_cert, server_key, client_cert, client_key) = load_certs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sc = server_cert.clone();
        let sk = server_key.clone();
        let cc = client_cert.clone();
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut server = TlsConnection::create(
                Role::Server,
                &sc,
                &sk,
                stream,
            )
            .unwrap();
            server.add_trusted_certificate(&cc);
            assert_eq!(server.do_handshake(), TlsError::Success);

            let msg = server.read_fully(4);
            assert_eq!(msg, b"ping");
            server.write_fully(b"pong");
        });

        let client_stream = std::net::TcpStream::connect(addr).unwrap();
        let mut client = TlsConnection::create(
            Role::Client,
            &client_cert,
            &client_key,
            client_stream,
        )
        .unwrap();
        client.add_trusted_certificate(&server_cert);

        assert_eq!(client.do_handshake(), TlsError::Success);
        client.write_fully(b"ping");
        let resp = client.read_fully(4);
        assert_eq!(resp, b"pong");

        server_thread.join().unwrap();
    }

    #[test]
    fn test_export_keying_material() {
        let (server_cert, server_key, client_cert, client_key) = load_certs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sc = server_cert.clone();
        let sk = server_key.clone();
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut server = TlsConnection::create(
                Role::Server,
                &sc,
                &sk,
                stream,
            )
            .unwrap();
            server.set_cert_verify_callback(|| true);
            assert_eq!(server.do_handshake(), TlsError::Success);
            server.export_keying_material(32)
        });

        let client_stream = std::net::TcpStream::connect(addr).unwrap();
        let mut client = TlsConnection::create(
            Role::Client,
            &client_cert,
            &client_key,
            client_stream,
        )
        .unwrap();
        client.set_cert_verify_callback(|| true);

        assert_eq!(client.do_handshake(), TlsError::Success);
        let client_key = client.export_keying_material(32);
        let server_key = server_thread.join().unwrap();

        assert_eq!(client_key.len(), 32);
        assert_eq!(client_key, server_key);
    }

    #[test]
    fn test_certificate_callback() {
        let (server_cert, server_key, client_cert, client_key) = load_certs();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let sc = server_cert.clone();
        let sk = server_key.clone();
        let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut server = TlsConnection::create(
                Role::Server,
                &sc,
                &sk,
                stream,
            )
            .unwrap();
            server.set_cert_verify_callback(|| true);
            assert_eq!(server.do_handshake(), TlsError::Success);
        });

        let client_stream = std::net::TcpStream::connect(addr).unwrap();
        let dummy_key = rust_adb_crypto::new_rsa_2048().unwrap();
        let dummy_cert = rust_adb_crypto::generate_x509_certificate(&dummy_key).unwrap();
        let dummy_cert_pem = rust_adb_crypto::x509_to_pem_string(&dummy_cert).unwrap();
        let dummy_key_pem = dummy_key.to_pem_string().unwrap();

        let mut client = TlsConnection::create(
            Role::Client,
            &dummy_cert_pem,
            &dummy_key_pem,
            client_stream,
        )
        .unwrap();
        client.set_cert_verify_callback(|| true);

        let cc = client_cert.clone();
        let ck = client_key.clone();
        client.set_certificate_callback(move |_acceptable_issuers| {
            let cert_chain = rustls_pemfile::certs(&mut cc.as_bytes()).unwrap()
                .into_iter().map(rustls::Certificate).collect();
            let priv_key = rustls_pemfile::pkcs8_private_keys(&mut ck.as_bytes()).unwrap()
                .into_iter().map(rustls::PrivateKey).next().unwrap();
            Some((cert_chain, priv_key))
        });

        assert_eq!(client.do_handshake(), TlsError::Success);
        server_thread.join().unwrap();
    }

    #[test]
    fn test_client_ca_list_adb_ca_list() {
         let (server_cert, server_key, client_cert, client_key) = load_certs();
         let listener = TcpListener::bind("127.0.0.1:0").unwrap();
         let addr = listener.local_addr().unwrap();

         let sc = server_cert.clone();
         let sk = server_key.clone();
         let key_hash = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
         let server_thread = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut server = TlsConnection::create(
                Role::Server,
                &sc,
                &sk,
                stream,
            )
            .unwrap();
            server.set_cert_verify_callback(|| true);
            let ca_issuer = rust_adb_crypto::create_ca_issuer_from_encoded_key(key_hash).unwrap();
            server.set_client_ca_list(vec![ca_issuer]);
            assert_eq!(server.do_handshake(), TlsError::Success);
         });

         let client_stream = std::net::TcpStream::connect(addr).unwrap();
         let mut client = TlsConnection::create(
            Role::Client,
            &client_cert,
            &client_key,
            client_stream,
        )
        .unwrap();
        client.set_cert_verify_callback(|| true);

        let kh = key_hash.to_string();
        client.set_certificate_callback(move |acceptable_issuers| {
            if acceptable_issuers.is_empty() { return None; }
            let out_key = rust_adb_crypto::parse_encoded_key_from_ca_issuer(&acceptable_issuers[0]).unwrap();
            if out_key == Some(kh.clone()) {
                // Success
            }
            None // Continue with default
        });

        assert_eq!(client.do_handshake(), TlsError::Success);
        server_thread.join().unwrap();
    }
}
