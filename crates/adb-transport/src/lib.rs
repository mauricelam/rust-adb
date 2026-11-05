//! adb-transport: Connection and transport interfaces for USB, TCP/IP, and mDNS.

pub mod emulator;
pub mod handshake;
pub mod tcp;
pub mod transport;
pub mod usb;
pub mod usb_transport;

#[cfg(feature = "usb")]
pub mod nusb_backend;

pub use adb_proto::{AdbPort, ConsolePort};
pub use emulator::{
    console_port_for_adb_port, emulator_serial, scan_for_emulators, scan_for_emulators_with_config,
    DiscoveredEmulator, EmulatorScanConfig, DEFAULT_ADB_LOCAL_TRANSPORT_MAX_PORT,
    DEFAULT_ADB_LOCAL_TRANSPORT_PORT,
};
pub use handshake::{perform_cnxn_handshake, HandshakeResult};
pub use tcp::TcpTransport;
pub use transport::{
    calculate_checksum, read_packet_from_stream, write_packet_to_stream, AsyncStreamTransport,
    Packet, Transport, TransportError,
};
pub use usb::{
    find_adb_interface, is_adb_interface, AdbInterfaceMatch, EndpointDirection, EndpointType,
    MockUsbBackend, MockUsbDeviceHandle, RecordedTransfer, UsbBackend, UsbDeviceHandle,
    UsbDeviceInfo, UsbEndpointInfo, UsbError, UsbHotplugEvent, UsbInterfaceInfo, ADB_CLASS,
    ADB_DBC_CLASS, ADB_DBC_SUBCLASS, ADB_PROTOCOL, ADB_SUBCLASS,
};
pub use usb_transport::UsbTransport;

#[cfg(feature = "usb")]
pub use nusb_backend::{NusbBackend, NusbDeviceHandle};
