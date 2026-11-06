//! Device state machine integrating ADB authentication and state transitions.
//!
//! Ported from `original/adb.cpp` and `original/transport.cpp`.

use adb_auth::{AuthAction, AuthEvent, HostAuth};
use adb_proto::Command;

use crate::state::ConnectionState;

/// An outgoing packet produced by the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingPacket {
    /// ADB command code.
    pub command: Command,
    /// First command argument (e.g. subcommand `ADB_AUTH_SIGNATURE` or `ADB_AUTH_RSAPUBLICKEY`).
    pub arg0: u32,
    /// Second command argument.
    pub arg1: u32,
    /// Packet payload.
    pub payload: Vec<u8>,
}

/// Output produced when processing a state machine event.
#[derive(Debug, Clone, Default)]
pub struct DeviceStateMachineOutput {
    /// Outgoing packets to transmit over the transport.
    pub packets: Vec<OutgoingPacket>,
    /// Whether connection has been established (online or unauthorized prompt active).
    pub connection_established: bool,
}

/// Events driving the device state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceStateMachineEvent<'a> {
    /// Received `A_AUTH` with `ADB_AUTH_TOKEN` containing token challenge bytes.
    AuthToken(&'a [u8]),
    /// Received `A_CNXN` carrying the parsed `ConnectionState`.
    Cnxn {
        /// Connection state derived from device banner.
        state: ConnectionState,
    },
    /// The transport disconnected or failed.
    Disconnected,
}

impl<'a> DeviceStateMachineEvent<'a> {
    /// Convenience constructor creating a `Cnxn` event from a banner service type string.
    pub fn cnxn_from_service_type(service_type: &str) -> Self {
        Self::Cnxn {
            state: ConnectionState::from_service_type(service_type),
        }
    }
}

/// Explicit connection progression state of a device transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceConnectionStatus {
    /// Initial or offline state.
    Offline,
    /// Actively authenticating using signature or public key challenges.
    Authorizing,
    /// Connection established either via authorization or prompt displayed.
    Established(ConnectionState),
}

/// Per-transport device state machine managing authentication progression and connection states.
pub struct DeviceStateMachine {
    status: DeviceConnectionStatus,
    auth: HostAuth,
}

impl DeviceStateMachine {
    /// Creates a new `DeviceStateMachine` with initial `Offline` status and provided `HostAuth`.
    pub fn new(auth: HostAuth) -> Self {
        Self {
            status: DeviceConnectionStatus::Offline,
            auth,
        }
    }

    /// Current connection status of the device.
    pub fn status(&self) -> DeviceConnectionStatus {
        self.status
    }

    /// Current connection state of the device.
    pub fn state(&self) -> ConnectionState {
        match self.status {
            DeviceConnectionStatus::Offline => ConnectionState::Offline,
            DeviceConnectionStatus::Authorizing => ConnectionState::Authorizing,
            DeviceConnectionStatus::Established(state) => state,
        }
    }

    /// Whether the device is in an online operational state.
    pub fn is_online(&self) -> bool {
        self.state().is_online()
    }

    /// Whether the connection has been established (either authenticated or prompt shown).
    pub fn is_connection_established(&self) -> bool {
        matches!(self.status, DeviceConnectionStatus::Established(_))
    }

    /// Processes an incoming event, updates internal state, and returns any outputs/packets to send.
    pub fn handle_event(&mut self, event: DeviceStateMachineEvent<'_>) -> DeviceStateMachineOutput {
        let mut output = DeviceStateMachineOutput::default();

        match event {
            DeviceStateMachineEvent::AuthToken(token) => {
                if !matches!(self.status, DeviceConnectionStatus::Authorizing) {
                    self.status = DeviceConnectionStatus::Authorizing;
                }

                let actions = self.auth.handle_event(AuthEvent::AuthToken(token));
                for action in actions {
                    match action {
                        AuthAction::SendPacket {
                            command,
                            arg0,
                            payload,
                        } => {
                            output.packets.push(OutgoingPacket {
                                command,
                                arg0,
                                arg1: 0,
                                payload,
                            });
                        }
                        AuthAction::ReportUnauthorized => {
                            self.status =
                                DeviceConnectionStatus::Established(ConnectionState::Unauthorized);
                        }
                        AuthAction::ReportEstablished => {
                            self.status = DeviceConnectionStatus::Established(self.state());
                            output.connection_established = true;
                        }
                    }
                }
            }
            DeviceStateMachineEvent::Cnxn { state } => {
                let actions = self.auth.handle_event(AuthEvent::Cnxn);
                for action in actions {
                    if let AuthAction::ReportEstablished = action {
                        output.connection_established = true;
                    }
                }
                self.status = DeviceConnectionStatus::Established(state);
                output.connection_established = true;
            }
            DeviceStateMachineEvent::Disconnected => {
                self.status = DeviceConnectionStatus::Offline;
            }
        }

        output
    }
}
