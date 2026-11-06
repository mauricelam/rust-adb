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
pub mod diagnose;
pub mod router;
pub mod stream;
pub use diagnose::{usb_no_permissions_long_help_text, usb_no_permissions_short_help_text};
pub mod tracker;

pub use tracker::{format_devices_framed, format_devices_raw, TrackerFormat};
pub mod usb_scanner;
pub use usb_scanner::UsbDeviceScanner;
pub mod listener;
pub mod server;

pub use listener::{InstallStatus, ListenerManager, INSTALL_LISTENER_NO_REBIND};

pub use router::{
    OpenDecision, OpenHandler, OpenRequest, PacketRouter, RouterError, StreamAcker, StreamMessage,
    StreamReceiver, StreamSender, StreamWriter,
};
pub use server::{
    encode_fail, encode_okay, encode_okay_with_payload, parse_smart_socket_request, ServerError,
    SmartSocketRequest, SmartSocketServer, TransportTarget, ADB_SERVER_VERSION,
    DEFAULT_SERVER_PORT,
};
pub use stream::{StreamEntry, StreamError, StreamRegistry};

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_test() {
        assert_eq!(2 + 2, 4);
    }
}
