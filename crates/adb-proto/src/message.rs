//! High-level typed ADB protocol messages.
//!
//! Provides [`Message`] representing typed commands (`CNXN`, `AUTH`, `OPEN`, `OKAY`, `WRTE`, `CLSE`, `STLS`),
//! encoding and decoding between raw [`Header`], payload bytes, and typed structures.

use std::fmt;

use crate::auth::{AuthMessage, AuthType, PublicKeyLine, TOKEN_SIZE};
use crate::header::{ChecksumMode, Command, Header};
use crate::types::{AckBytes, LocalId, RemoteId};

/// Errors that can occur when decoding a [`Message`] from a [`Header`] and payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageDecodeError {
    /// Token challenge payload size does not match [`TOKEN_SIZE`].
    InvalidTokenLength { expected: usize, actual: usize },
    /// Unsupported or unknown authentication sub-command type.
    UnknownAuthType(u32),
    /// Invalid remote or local ID for stream operations.
    InvalidStreamId,
    /// UTF-8 decode error in banner, service destination, or public key string.
    InvalidUtf8(String),
}

impl fmt::Display for MessageDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTokenLength { expected, actual } => {
                write!(f, "invalid token length: expected {expected}, got {actual}")
            }
            Self::UnknownAuthType(val) => write!(f, "unknown auth type: {val}"),
            Self::InvalidStreamId => write!(f, "invalid stream id (zero where non-zero expected)"),
            Self::InvalidUtf8(msg) => write!(f, "invalid utf-8: {msg}"),
        }
    }
}

impl std::error::Error for MessageDecodeError {}

/// High-level typed ADB wire protocol message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Connection handshake message (`A_CNXN`).
    Cnxn {
        /// Protocol version (arg0).
        version: u32,
        /// Maximum payload limit (arg1).
        max_payload: u32,
        /// Identity banner string.
        banner: String,
    },
    /// Authentication message (`A_AUTH`).
    Auth(AuthMessage),
    /// Open logical stream request (`A_OPEN`).
    Open {
        /// Peer's remote ID (arg0 on receive).
        remote_id: RemoteId,
        /// Send buffer / delayed-ack bytes (arg1).
        send_bytes: u32,
        /// Destination service string (e.g. "shell:ls").
        service: String,
    },
    /// Stream acknowledgment or ready message (`A_OKAY`).
    Okay {
        /// Local stream ID.
        local_id: LocalId,
        /// Remote stream ID.
        remote_id: RemoteId,
        /// Optional delayed acknowledgment bytes in payload.
        ack_bytes: Option<AckBytes>,
    },
    /// Stream data payload message (`A_WRTE`).
    Wrte {
        /// Local stream ID.
        local_id: LocalId,
        /// Remote stream ID.
        remote_id: RemoteId,
        /// Stream payload bytes.
        data: Vec<u8>,
    },
    /// Stream close message (`A_CLSE`).
    Clse {
        /// Local stream ID, if allocated.
        local_id: Option<LocalId>,
        /// Remote stream ID, if allocated (arg0 on wire can be 0 for rejected OPEN).
        remote_id: Option<RemoteId>,
    },
    /// TLS setup message (`A_STLS`).
    Stls {
        /// Protocol / TLS version.
        version: u32,
    },
}

impl Message {
    /// Decodes a [`Message`] from a parsed [`Header`] and its payload slice.
    pub fn decode(header: &Header, payload: &[u8]) -> Result<Self, MessageDecodeError> {
        match header.command() {
            Command::Cnxn => {
                let banner = String::from_utf8(payload.to_vec())
                    .map_err(|e| MessageDecodeError::InvalidUtf8(e.to_string()))?;
                Ok(Message::Cnxn {
                    version: header.arg0(),
                    max_payload: header.arg1(),
                    banner,
                })
            }
            Command::Auth => {
                let auth_type = AuthType::try_from(header.arg0())
                    .map_err(|_| MessageDecodeError::UnknownAuthType(header.arg0()))?;
                match auth_type {
                    AuthType::Token => {
                        if payload.len() != TOKEN_SIZE {
                            return Err(MessageDecodeError::InvalidTokenLength {
                                expected: TOKEN_SIZE,
                                actual: payload.len(),
                            });
                        }
                        let mut token = [0u8; TOKEN_SIZE];
                        token.copy_from_slice(payload);
                        Ok(Message::Auth(AuthMessage::Token(token)))
                    }
                    AuthType::Signature => {
                        Ok(Message::Auth(AuthMessage::Signature(payload.to_vec())))
                    }
                    AuthType::RsaPublicKey => {
                        let mut p = payload;
                        while p.last() == Some(&0) {
                            p = &p[..p.len() - 1];
                        }
                        let s = std::str::from_utf8(p)
                            .map_err(|e| MessageDecodeError::InvalidUtf8(e.to_string()))?;
                        Ok(Message::Auth(AuthMessage::PublicKey(PublicKeyLine::new(s))))
                    }
                }
            }
            Command::Open => {
                let remote_id =
                    RemoteId::from_u32(header.arg0()).ok_or(MessageDecodeError::InvalidStreamId)?;
                let send_bytes = header.arg1();
                let mut p = payload;
                while p.last() == Some(&0) {
                    p = &p[..p.len() - 1];
                }
                let service = std::str::from_utf8(p)
                    .map_err(|e| MessageDecodeError::InvalidUtf8(e.to_string()))?
                    .to_string();
                Ok(Message::Open {
                    remote_id,
                    send_bytes,
                    service,
                })
            }
            Command::Okay => {
                let remote_id =
                    RemoteId::from_u32(header.arg0()).ok_or(MessageDecodeError::InvalidStreamId)?;
                let local_id =
                    LocalId::from_u32(header.arg1()).ok_or(MessageDecodeError::InvalidStreamId)?;
                let ack_bytes = if payload.len() == 4 {
                    Some(AckBytes::from_le_bytes(payload.try_into().unwrap()))
                } else {
                    None
                };
                Ok(Message::Okay {
                    local_id,
                    remote_id,
                    ack_bytes,
                })
            }
            Command::Wrte => {
                let remote_id =
                    RemoteId::from_u32(header.arg0()).ok_or(MessageDecodeError::InvalidStreamId)?;
                let local_id =
                    LocalId::from_u32(header.arg1()).ok_or(MessageDecodeError::InvalidStreamId)?;
                Ok(Message::Wrte {
                    local_id,
                    remote_id,
                    data: payload.to_vec(),
                })
            }
            Command::Clse => {
                let local_id = LocalId::from_u32(header.arg1());
                let remote_id = RemoteId::from_u32(header.arg0());
                Ok(Message::Clse {
                    local_id,
                    remote_id,
                })
            }
            Command::Stls => Ok(Message::Stls {
                version: header.arg0(),
            }),
            Command::Sync => Err(MessageDecodeError::UnknownAuthType(header.arg0())),
        }
    }

    /// Encodes this message into a raw [`Header`] and payload byte vector for transmission.
    pub fn encode(&self, checksum_mode: ChecksumMode) -> (Header, Vec<u8>) {
        let (command, arg0, arg1, payload) = match self {
            Message::Cnxn {
                version,
                max_payload,
                banner,
            } => (
                Command::Cnxn,
                *version,
                *max_payload,
                banner.as_bytes().to_vec(),
            ),
            Message::Auth(auth_msg) => match auth_msg {
                AuthMessage::Token(token) => {
                    (Command::Auth, AuthType::Token as u32, 0, token.to_vec())
                }
                AuthMessage::Signature(sig) => {
                    (Command::Auth, AuthType::Signature as u32, 0, sig.clone())
                }
                AuthMessage::PublicKey(pubkey) => {
                    let mut p = pubkey.as_str().as_bytes().to_vec();
                    p.push(0);
                    (Command::Auth, AuthType::RsaPublicKey as u32, 0, p)
                }
            },
            Message::Open {
                remote_id,
                send_bytes,
                service,
            } => {
                let mut p = service.as_bytes().to_vec();
                p.push(0);
                (Command::Open, remote_id.get(), *send_bytes, p)
            }
            Message::Okay {
                local_id,
                remote_id,
                ack_bytes,
            } => {
                let payload = match ack_bytes {
                    Some(bytes) => bytes.to_le_bytes().to_vec(),
                    None => Vec::new(),
                };
                (Command::Okay, local_id.get(), remote_id.get(), payload)
            }
            Message::Wrte {
                local_id,
                remote_id,
                data,
            } => (Command::Wrte, local_id.get(), remote_id.get(), data.clone()),
            Message::Clse {
                local_id,
                remote_id,
            } => (
                Command::Clse,
                local_id.map_or(0, |id| id.get()),
                remote_id.map_or(0, |id| id.get()),
                Vec::new(),
            ),
            Message::Stls { version } => (Command::Stls, *version, 0, Vec::new()),
        };

        let data_crc32 = match checksum_mode {
            ChecksumMode::Skip => 0,
            ChecksumMode::Compute => payload.iter().map(|&b| b as u32).sum(),
        };

        let header = Header::new(command, arg0, arg1, payload.len() as u32, data_crc32);
        (header, payload)
    }
}
