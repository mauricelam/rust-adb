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
#[derive(Debug, Clone)]
pub enum DeviceStateMachineEvent<'a> {
    /// Received `A_AUTH` with `ADB_AUTH_TOKEN` containing token challenge bytes.
    AuthToken(&'a [u8]),
    /// Received `A_CNXN` with banner service type (e.g. "device", "recovery").
    Cnxn {
        /// Service type reported in device banner.
        service_type: String,
    },
    /// The transport disconnected or failed.
    Disconnected,
}

/// Per-transport device state machine managing authentication progression and connection states.
pub struct DeviceStateMachine {
    state: ConnectionState,
    auth: HostAuth,
    connection_established: bool,
}

impl DeviceStateMachine {
    /// Creates a new `DeviceStateMachine` with initial `Offline` state and provided `HostAuth`.
    pub fn new(auth: HostAuth) -> Self {
        Self {
            state: ConnectionState::Offline,
            auth,
            connection_established: false,
        }
    }

    /// Current connection state of the device.
    pub fn state(&self) -> ConnectionState {
        self.state
    }

    /// Whether the device is in an online operational state.
    pub fn is_online(&self) -> bool {
        self.state.is_online()
    }

    /// Whether the connection has been established (either authenticated or prompt shown).
    pub fn is_connection_established(&self) -> bool {
        self.connection_established
    }

    /// Processes an incoming event, updates internal state, and returns any outputs/packets to send.
    pub fn handle_event(&mut self, event: DeviceStateMachineEvent<'_>) -> DeviceStateMachineOutput {
        let mut output = DeviceStateMachineOutput::default();

        match event {
            DeviceStateMachineEvent::AuthToken(token) => {
                if self.state != ConnectionState::Authorizing {
                    self.state = ConnectionState::Authorizing;
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
                            self.state = ConnectionState::Unauthorized;
                        }
                        AuthAction::ReportEstablished => {
                            self.connection_established = true;
                            output.connection_established = true;
                        }
                    }
                }
            }
            DeviceStateMachineEvent::Cnxn { service_type } => {
                let actions = self.auth.handle_event(AuthEvent::Cnxn);
                for action in actions {
                    if let AuthAction::ReportEstablished = action {
                        self.connection_established = true;
                        output.connection_established = true;
                    }
                }
                self.state = ConnectionState::from_service_type(&service_type);
            }
            DeviceStateMachineEvent::Disconnected => {
                self.state = ConnectionState::Offline;
                self.connection_established = false;
            }
        }

        output
    }
}
