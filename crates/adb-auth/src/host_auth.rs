//! Host-side authentication state machine for ADB AUTH exchange.
//!
//! Ported from:
//! - `original/client/auth.cpp`: `send_auth_response`, `send_auth_publickey`, `adb_auth_sign`
//! - `original/adb.cpp`: `handle_packet` (A_AUTH case)
//! - `original/transport.cpp`: `atransport::NextKey`, `atransport::ResetKeys`

use std::collections::VecDeque;

use adb_proto::{Command, ADB_AUTH_RSAPUBLICKEY, ADB_AUTH_SIGNATURE, MAX_PAYLOAD_V1};
use rsa::RsaPrivateKey;

use crate::keys::sign_token;

/// Events received from the device during host authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent<'a> {
    /// Device sent an `A_AUTH` packet with `ADB_AUTH_TOKEN` containing the challenge token payload.
    AuthToken(&'a [u8]),
    /// Device sent `A_CNXN` indicating connection was approved/established.
    Cnxn,
}

/// Actions produced by the `HostAuth` state machine in response to an `AuthEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthAction {
    /// Packet to send to the device over the transport.
    SendPacket {
        command: Command,
        arg0: u32,
        payload: Vec<u8>,
    },
    /// Report that the transport state is unauthorized (`kCsUnauthorized`).
    ReportUnauthorized,
    /// Report that the transport connection is established (`connection_established = true`).
    ReportEstablished,
}

/// Pure host-side authentication state machine.
///
/// Created once per connection/transport from available RSA private keys and user public key line.
/// Contains no I/O, no threads, and no locks.
#[derive(Debug, Clone)]
pub struct HostAuth {
    keys: VecDeque<RsaPrivateKey>,
    user_pubkey: String,
}

impl HostAuth {
    /// Create a new `HostAuth` state machine for a connection.
    ///
    /// `keys`: list of private keys to attempt (ordered according to task 2.1b).
    /// `user_pubkey`: the user key's public key line (`adbkey.pub`).
    pub fn new(keys: Vec<RsaPrivateKey>, user_pubkey: String) -> Self {
        Self {
            keys: VecDeque::from(keys),
            user_pubkey,
        }
    }

    /// Process an incoming authentication event and return actions to execute.
    ///
    /// Ported from `send_auth_response` and `send_auth_publickey` in `original/client/auth.cpp`
    /// and `handle_packet` A_AUTH case in `original/adb.cpp`.
    pub fn handle_event(&mut self, event: AuthEvent<'_>) -> Vec<AuthAction> {
        let mut actions = Vec::new();

        match event {
            AuthEvent::AuthToken(token) => {
                // Ported from atransport::NextKey() in original/transport.cpp:
                // NextKey pops the first key from keys_ queue.
                if let Some(key) = self.keys.pop_front() {
                    // Try signing the token with this key
                    if let Ok(sig) = sign_token(&key, token) {
                        actions.push(AuthAction::SendPacket {
                            command: Command::Auth,
                            arg0: ADB_AUTH_SIGNATURE,
                            payload: sig,
                        });
                    } else {
                        // adb_auth_sign failed (e.g. invalid token length).
                        // Key was consumed (popped), but no packet is sent.
                    }
                } else {
                    // No more private keys left.
                    // Ported from original/client/auth.cpp send_auth_response:
                    // t->SetConnectionState(kCsUnauthorized);
                    // t->SetConnectionEstablished(true);
                    // send_auth_publickey(t);
                    actions.push(AuthAction::ReportUnauthorized);
                    actions.push(AuthAction::ReportEstablished);

                    // Ported from send_auth_publickey in original/client/auth.cpp:
                    // Check if public key string length < MAX_PAYLOAD_V1 (4096 bytes)
                    if self.user_pubkey.len() < MAX_PAYLOAD_V1 {
                        let mut payload = self.user_pubkey.as_bytes().to_vec();
                        // adbd expects a null-terminated string.
                        payload.push(0);

                        actions.push(AuthAction::SendPacket {
                            command: Command::Auth,
                            arg0: ADB_AUTH_RSAPUBLICKEY,
                            payload,
                        });
                    }
                }
            }
            AuthEvent::Cnxn => {
                // Device sent CNXN (e.g., after user approved dialog on device).
                actions.push(AuthAction::ReportEstablished);
            }
        }

        actions
    }
}
