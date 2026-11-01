//! adb-server: Host server daemon implementation and service routing.

pub mod registry;
pub mod state;
pub mod state_machine;
pub mod transport_type;

pub use registry::{DeviceEntry, DeviceRegistry, RegistrationError, TransportId};
pub use state::ConnectionState;
pub use state_machine::{
    DeviceStateMachine, DeviceStateMachineEvent, DeviceStateMachineOutput, OutgoingPacket,
};
pub use transport_type::TransportType;

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
