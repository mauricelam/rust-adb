//! ADB Wi-Fi pairing protocol implementation.
//! Ported from `pairing_connection`.

use adb_transport::tls_connection::{Role as TlsRole, TlsConnection};
use anyhow::{anyhow, Result};
use rust_adb_pairing_auth::{PairingAuthCtxBuilder, Role as AuthRole};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use zerocopy::{IntoBytes, FromBytes, Immutable, KnownLayout};

/// Maximum size of peer information.
pub const K_MAX_PEER_INFO_SIZE: usize = 8192;

/// Information about a pairing peer.
/// Ported from `PeerInfo` in `pairing_connection.h`.
#[repr(C, packed)]
#[derive(Clone, Copy, IntoBytes, FromBytes, Immutable, KnownLayout)]
pub struct PeerInfo {
    /// Type of peer information.
    pub info_type: u8,
    /// Peer information data.
    pub data: [u8; K_MAX_PEER_INFO_SIZE - 1],
}

impl PeerInfo {
    /// Creates a new `PeerInfo` structure.
    pub fn new(info_type: u8, data_bytes: &[u8]) -> Self {
        let mut info = Self {
            info_type,
            data: [0u8; K_MAX_PEER_INFO_SIZE - 1],
        };
        let len = std::cmp::min(data_bytes.len(), K_MAX_PEER_INFO_SIZE - 1);
        info.data[..len].copy_from_slice(&data_bytes[..len]);
        info
    }
}

/// Protocol definitions for pairing packets.
pub mod proto {
    /// Pairing packet types.
    pub mod pairing_packet {
        /// Type of pairing packet.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(i32)]
        pub enum Type {
            /// SPAKE2 message packet.
            Spake2Msg = 0,
            /// Peer information packet.
            PeerInfo = 1,
        }
    }
}

#[repr(C, packed)]
#[derive(IntoBytes, FromBytes, Immutable, KnownLayout)]
struct PairingPacketHeader {
    version: u8,
    packet_type: u8,
    payload: u32,
}

const K_CURRENT_KEY_HEADER_VERSION: u8 = 1;

/// Role of the pairing participant.
pub enum Role {
    /// The client role.
    Client,
    /// The server role.
    Server,
}

/// Represents a pairing connection.
/// Ported from `PairingConnection` in `pairing_connection.h`.
pub struct PairingConnection {
    role: Role,
    pswd: Vec<u8>,
    peer_info: PeerInfo,
    cert: Vec<u8>,
    priv_key: Vec<u8>,
}

impl PairingConnection {
    /// Creates a new `PairingConnection`.
    pub fn new(
        role: Role,
        pswd: Vec<u8>,
        peer_info: PeerInfo,
        cert: Vec<u8>,
        priv_key: Vec<u8>,
    ) -> Self {
        Self {
            role,
            pswd,
            peer_info,
            cert,
            priv_key,
        }
    }

    /// Starts the pairing process in a background thread.
    pub fn start<F>(self, stream: TcpStream, cb: F) -> Result<()>
    where
        F: FnOnce(Option<PeerInfo>) + Send + 'static,
    {
        thread::spawn(move || {
            let res = self.do_pairing(stream);
            cb(res.ok());
        });
        Ok(())
    }

    fn do_pairing(&self, stream: TcpStream) -> Result<PeerInfo> {
        // 1. TLS Handshake
        let mut pswd = self.pswd.clone();

        let role = match self.role {
            Role::Client => TlsRole::Client,
            Role::Server => TlsRole::Server,
        };

        let mut tls = TlsConnection::create(
            role,
            &String::from_utf8_lossy(&self.cert),
            &String::from_utf8_lossy(&self.priv_key),
            stream,
        )
        .map_err(|e| anyhow!("Failed to create TlsConnection: {}", e))?;

        // Allow any peer certificate for pairing
        tls.set_cert_verify_callback(|| true);

        tls.do_handshake();

        // Export keying material to append to password
        let exported = tls.export_keying_material(64);
        if exported.is_empty() {
            return Err(anyhow!("Failed to export keying material"));
        }
        pswd.extend_from_slice(&exported);

        // 2. SPAKE2 Exchange
        let auth_role = match self.role {
            Role::Client => AuthRole::Client,
            Role::Server => AuthRole::Server,
        };
        let builder = PairingAuthCtxBuilder::new(&pswd, auth_role)
            .map_err(|e| anyhow!("Failed to create auth builder: {}", e))?;

        let our_msg = builder.msg().to_vec();
        self.write_packet(&mut tls, proto::pairing_packet::Type::Spake2Msg, &our_msg)?;

        let (header, their_msg) = self.read_packet(&mut tls)?;
        if header.packet_type != proto::pairing_packet::Type::Spake2Msg as u8 {
            return Err(anyhow!("Unexpected packet type: {}", header.packet_type));
        }

        let mut auth_ctx = builder
            .init_cipher(&their_msg)
            .map_err(|e| anyhow!("Failed to init cipher: {}", e))?;

        // 3. PeerInfo Exchange
        let encrypted_info = auth_ctx
            .encrypt(self.peer_info.as_bytes())
            .map_err(|e| anyhow!("Encryption failed: {}", e))?;
        self.write_packet(&mut tls, proto::pairing_packet::Type::PeerInfo, &encrypted_info)?;

        let (header, encrypted_their_info) = self.read_packet(&mut tls)?;
        if header.packet_type != proto::pairing_packet::Type::PeerInfo as u8 {
            return Err(anyhow!("Unexpected packet type: {}", header.packet_type));
        }

        let their_info_bytes = auth_ctx
            .decrypt(&encrypted_their_info)
            .map_err(|e| anyhow!("Decryption failed: {}", e))?;

        let their_info = PeerInfo::read_from_bytes(&their_info_bytes[..])
            .map_err(|e| anyhow!("Invalid peer info size or format: {}", e))?;

        Ok(their_info)
    }

    fn write_packet(
        &self,
        tls: &mut TlsConnection,
        p_type: proto::pairing_packet::Type,
        payload: &[u8],
    ) -> Result<()> {
        let header = PairingPacketHeader {
            version: K_CURRENT_KEY_HEADER_VERSION,
            packet_type: p_type as u8,
            payload: (payload.len() as u32).to_be(),
        };
        if !tls.write_fully(header.as_bytes()) || !tls.write_fully(payload) {
            return Err(anyhow!("Write failed"));
        }
        Ok(())
    }

    fn read_packet(&self, tls: &mut TlsConnection) -> Result<(PairingPacketHeader, Vec<u8>)> {
        let mut header_bytes = [0u8; std::mem::size_of::<PairingPacketHeader>()];
        if !tls.read_fully_buf(&mut header_bytes) {
            return Err(anyhow!("Read header failed"));
        }
        let header = PairingPacketHeader::read_from_bytes(&header_bytes[..])
            .map_err(|e| anyhow!("Invalid header: {}", e))?;

        if header.version != K_CURRENT_KEY_HEADER_VERSION {
            return Err(anyhow!("Version mismatch"));
        }
        let payload_len = u32::from_be(header.payload) as usize;
        let payload = tls.read_fully(payload_len);
        if payload.len() != payload_len {
            return Err(anyhow!("Read payload failed"));
        }
        Ok((header, payload))
    }
}

/// A server that listens for pairing requests.
/// Ported from `PairingServer` in `pairing_connection.h`.
pub struct PairingServer {
    pswd: Vec<u8>,
    peer_info: PeerInfo,
    cert: Vec<u8>,
    priv_key: Vec<u8>,
    port: u16,
}

impl PairingServer {
    /// Creates a new `PairingServer`.
    pub fn new(
        pswd: Vec<u8>,
        peer_info: PeerInfo,
        cert: Vec<u8>,
        priv_key: Vec<u8>,
        port: u16,
    ) -> Self {
        Self {
            pswd,
            peer_info,
            cert,
            priv_key,
            port,
        }
    }

    /// Starts the pairing server.
    pub fn start<F>(self, cb: F) -> Result<u16>
    where
        F: Fn(Option<PeerInfo>) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port))?;
        let port = listener.local_addr()?.port();
        let cb = Arc::new(cb);

        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let conn = PairingConnection::new(
                            Role::Server,
                            self.pswd.clone(),
                            self.peer_info.clone(),
                            self.cert.clone(),
                            self.priv_key.clone(),
                        );
                        let cb_clone = cb.clone();
                        let _ = conn.start(stream, move |res| {
                            cb_clone(res);
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_adb_crypto::{generate_x509_certificate, new_rsa_2048, x509_to_pem_string};

    fn create_test_keys() -> (String, String) {
        let key = new_rsa_2048().unwrap();
        let cert = generate_x509_certificate(&key).unwrap();
        (
            x509_to_pem_string(&cert).unwrap(),
            key.to_pem_string().unwrap(),
        )
    }

    fn create_peer_info(name: &str) -> PeerInfo {
        PeerInfo::new(0, name.as_bytes())
    }

    #[test]
    fn smoke_valid_pairing() {
        let pswd = vec![0x01, 0x03, 0x05, 0x07];
        let server_info = create_peer_info("my_server_name");
        let client_info = create_peer_info("my_client_name");

        let (server_cert, server_key) = create_test_keys();
        let (client_cert, client_key) = create_test_keys();

        let pair = Arc::new((Mutex::new(None), Condvar::new()));
        let pair_clone = pair.clone();

        let server = PairingServer::new(
            pswd.clone(),
            server_info,
            server_cert.as_bytes().to_vec(),
            server_key.as_bytes().to_vec(),
            0,
        );

        let port = server
            .start(move |res| {
                let (lock, cv) = &*pair_clone;
                let mut result = lock.lock().unwrap();
                *result = Some(res);
                cv.notify_one();
            })
            .unwrap();

        let client = PairingConnection::new(
            Role::Client,
            pswd,
            client_info,
            client_cert.as_bytes().to_vec(),
            client_key.as_bytes().to_vec(),
        );

        let stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        let client_pair = Arc::new((Mutex::new(None), Condvar::new()));
        let client_pair_clone = client_pair.clone();

        client
            .start(stream, move |res| {
                let (lock, cv) = &*client_pair_clone;
                let mut result = lock.lock().unwrap();
                *result = Some(res);
                cv.notify_one();
            })
            .unwrap();

        // Wait for server
        let (lock, cv) = &*pair;
        let mut res = lock.lock().unwrap();
        let start = std::time::Instant::now();
        while res.is_none() && start.elapsed().as_secs() < 10 {
            res = cv
                .wait_timeout(res, std::time::Duration::from_secs(1))
                .unwrap()
                .0;
        }
        let server_res = res.take().unwrap().unwrap();
        assert_eq!(server_res.info_type, 0);
        assert!(String::from_utf8_lossy(&server_res.data).starts_with("my_client_name"));

        // Wait for client
        let (lock, cv) = &*client_pair;
        let mut res = lock.lock().unwrap();
        let start = std::time::Instant::now();
        while res.is_none() && start.elapsed().as_secs() < 10 {
            res = cv
                .wait_timeout(res, std::time::Duration::from_secs(1))
                .unwrap()
                .0;
        }
        let client_res = res.take().unwrap().unwrap();
        assert_eq!(client_res.info_type, 0);
        assert!(String::from_utf8_lossy(&client_res.data).starts_with("my_server_name"));
    }

    #[test]
    fn test_invalid_pswd() {
        let server_pswd = vec![0x01];
        let client_pswd = vec![0x02];
        let server_info = create_peer_info("s");
        let client_info = create_peer_info("c");

        let (server_cert, server_key) = create_test_keys();
        let (client_cert, client_key) = create_test_keys();

        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let pair_clone = pair.clone();

        let server = PairingServer::new(
            server_pswd,
            server_info,
            server_cert.as_bytes().to_vec(),
            server_key.as_bytes().to_vec(),
            0,
        );

        let port = server
            .start(move |res| {
                if res.is_none() {
                    let (lock, cv) = &*pair_clone;
                    let mut done = lock.lock().unwrap();
                    *done = true;
                    cv.notify_one();
                }
            })
            .unwrap();

        let client = PairingConnection::new(
            Role::Client,
            client_pswd,
            client_info,
            client_cert.as_bytes().to_vec(),
            client_key.as_bytes().to_vec(),
        );

        let stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        client.start(stream, |_| {}).unwrap();

        let (lock, cv) = &*pair;
        let mut done = lock.lock().unwrap();
        let start = std::time::Instant::now();
        while !*done && start.elapsed().as_secs() < 10 {
            done = cv
                .wait_timeout(done, std::time::Duration::from_secs(1))
                .unwrap()
                .0;
        }
        assert!(*done);
    }
}
