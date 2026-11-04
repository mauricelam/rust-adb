//! USB backend abstractions, descriptor models, and ADB interface matching logic.
//!
//! Ported and adapted from upstream C++ ADB USB subsystem:
//! - `original/client/usb.h`
//! - `original/client/transport_usb.cpp`
//! - `original/client/usb_libusb_device.cpp`
//! - `original/client/usb_linux.cpp`

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;

/// USB Class code for ADB (Vendor Specific: 0xFF).
pub const ADB_CLASS: u8 = 0xFF;

/// USB Subclass code for ADB (0x42).
pub const ADB_SUBCLASS: u8 = 0x42;

/// USB Protocol code for ADB (0x01).
pub const ADB_PROTOCOL: u8 = 0x01;

/// USB Class code for Intel DbC (Debug Device: 0xDC).
pub const ADB_DBC_CLASS: u8 = 0xDC;

/// USB Subclass code for Intel DbC (0x02).
pub const ADB_DBC_SUBCLASS: u8 = 0x02;

/// Errors that can occur during USB backend operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbError {
    /// Device was disconnected or disappeared from the bus.
    Disconnected,
    /// Operation timed out.
    Timeout,
    /// Endpoint stall condition encountered.
    Stall,
    /// Permission denied when attempting to access the USB device.
    PermissionDenied,
    /// An underlying I/O error occurred.
    Io(String),
    /// Other USB errors.
    Other(String),
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => write!(f, "device disconnected"),
            Self::Timeout => write!(f, "operation timed out"),
            Self::Stall => write!(f, "endpoint stalled"),
            Self::PermissionDenied => {
                write!(
                    f,
                    "permission denied: insufficient permissions to access USB device"
                )
            }
            Self::Io(err) => write!(f, "USB I/O error: {err}"),
            Self::Other(err) => write!(f, "USB error: {err}"),
        }
    }
}

impl std::error::Error for UsbError {}

/// Direction of USB endpoint transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointDirection {
    /// Host to Device (OUT).
    Out,
    /// Device to Host (IN).
    In,
}

/// Type of USB endpoint transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointType {
    /// Control transfer endpoint.
    Control,
    /// Isochronous transfer endpoint.
    Isochronous,
    /// Bulk transfer endpoint.
    Bulk,
    /// Interrupt transfer endpoint.
    Interrupt,
}

/// Information describing a USB endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbEndpointInfo {
    /// Endpoint address (including direction bit, e.g. 0x81 for IN 1, 0x01 for OUT 1).
    pub address: u8,
    /// Transfer direction (IN or OUT).
    pub direction: EndpointDirection,
    /// Transfer type (e.g. Bulk).
    pub transfer_type: EndpointType,
    /// Maximum packet size in bytes (e.g. 512 for HighSpeed, 1024 for SuperSpeed).
    pub max_packet_size: u16,
}

impl UsbEndpointInfo {
    /// Creates a new `UsbEndpointInfo`.
    pub fn new(
        address: u8,
        direction: EndpointDirection,
        transfer_type: EndpointType,
        max_packet_size: u16,
    ) -> Self {
        Self {
            address,
            direction,
            transfer_type,
            max_packet_size,
        }
    }
}

/// Information describing a USB interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbInterfaceInfo {
    /// Interface number.
    pub interface_number: u8,
    /// Interface class code.
    pub class: u8,
    /// Interface subclass code.
    pub subclass: u8,
    /// Interface protocol code.
    pub protocol: u8,
    /// List of endpoints belonging to this interface.
    pub endpoints: Vec<UsbEndpointInfo>,
}

/// Information describing a discovered USB device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDeviceInfo {
    /// USB bus number.
    pub bus_number: u8,
    /// USB device address on the bus.
    pub device_address: u8,
    /// OS-specific device path string (e.g., `usb:1-2.3` or `/dev/bus/usb/001/002`).
    pub device_path: String,
    /// USB Vendor ID.
    pub vendor_id: u16,
    /// USB Product ID.
    pub product_id: u16,
    /// Device serial number, if available.
    pub serial: Option<String>,
    /// Available interfaces.
    pub interfaces: Vec<UsbInterfaceInfo>,
}

/// Information on a matched ADB interface and its bulk endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbInterfaceMatch {
    /// The interface number of the ADB interface.
    pub interface_number: u8,
    /// Bulk OUT endpoint address.
    pub bulk_out_address: u8,
    /// Bulk OUT endpoint maximum packet size.
    pub bulk_out_max_packet_size: u16,
    /// Bulk IN endpoint address.
    pub bulk_in_address: u8,
    /// Bulk IN endpoint maximum packet size.
    pub bulk_in_max_packet_size: u16,
}

/// Checks whether an interface's class, subclass, and protocol match the ADB specification.
///
/// Ported from `is_adb_interface` in `original/client/transport_usb.cpp`:
/// ```cpp
/// bool is_adb_interface(int usb_class, int usb_subclass, int usb_protocol) {
///     if (usb_protocol == ADB_PROTOCOL && ((usb_class == ADB_CLASS && usb_subclass == ADB_SUBCLASS) ||
///             (usb_class == ADB_DBC_CLASS && usb_subclass == ADB_DBC_SUBCLASS)))
///         return true;
///     else
///         return false;
/// }
/// ```
pub fn is_adb_interface(class: u8, subclass: u8, protocol: u8) -> bool {
    protocol == ADB_PROTOCOL
        && ((class == ADB_CLASS && subclass == ADB_SUBCLASS)
            || (class == ADB_DBC_CLASS && subclass == ADB_DBC_SUBCLASS))
}

/// Inspects a list of interfaces on a device and finds the first valid ADB interface with
/// bulk IN and OUT endpoints.
///
/// Ported from `LibUsbDevice::FindAdbInterface` in `original/client/usb_libusb_device.cpp`
/// and endpoint verification in `original/client/usb_linux.cpp`.
pub fn find_adb_interface(interfaces: &[UsbInterfaceInfo]) -> Option<AdbInterfaceMatch> {
    for iface in interfaces {
        if !is_adb_interface(iface.class, iface.subclass, iface.protocol) {
            continue;
        }

        let mut bulk_in = None;
        let mut bulk_out = None;

        for ep in &iface.endpoints {
            if ep.transfer_type != EndpointType::Bulk {
                continue;
            }

            match ep.direction {
                EndpointDirection::In if bulk_in.is_none() => {
                    bulk_in = Some((ep.address, ep.max_packet_size));
                }
                EndpointDirection::Out if bulk_out.is_none() => {
                    bulk_out = Some((ep.address, ep.max_packet_size));
                }
                _ => {}
            }
        }

        if let (Some((in_addr, in_max)), Some((out_addr, out_max))) = (bulk_in, bulk_out) {
            return Some(AdbInterfaceMatch {
                interface_number: iface.interface_number,
                bulk_out_address: out_addr,
                bulk_out_max_packet_size: out_max,
                bulk_in_address: in_addr,
                bulk_in_max_packet_size: in_max,
            });
        }
    }

    None
}

/// Handle to an opened USB device interface for bulk transfers.
pub trait UsbDeviceHandle: Send + Sync + fmt::Debug {
    /// Reads up to `buf.len()` bytes from the bulk IN endpoint.
    fn bulk_read<'a>(
        &'a self,
        endpoint: u8,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, UsbError>> + Send + 'a>>;

    /// Writes data to the bulk OUT endpoint.
    fn bulk_write<'a>(
        &'a self,
        endpoint: u8,
        data: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, UsbError>> + Send + 'a>>;

    /// Clears endpoint halt / stall status.
    fn clear_halt<'a>(
        &'a self,
        endpoint: u8,
    ) -> Pin<Box<dyn Future<Output = Result<(), UsbError>> + Send + 'a>>;

    /// Closes and releases the USB interface.
    fn close<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), UsbError>> + Send + 'a>>;
}

/// Abstract USB backend for enumerating and opening USB devices.
pub trait UsbBackend: Send + Sync {
    /// Lists all attached USB devices.
    fn list_devices<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<UsbDeviceInfo>, UsbError>> + Send + 'a>>;

    /// Opens the specified USB device.
    #[allow(clippy::type_complexity)]
    fn open<'a>(
        &'a self,
        device: &'a UsbDeviceInfo,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn UsbDeviceHandle>, UsbError>> + Send + 'a>>;
}

/// In-memory mock implementation of `UsbDeviceHandle`.
#[derive(Debug, Default)]
pub struct MockUsbDeviceHandle {
    pub read_data: Mutex<Vec<u8>>,
    pub written_data: Mutex<Vec<u8>>,
    pub should_fail_read: Mutex<Option<UsbError>>,
    pub should_fail_write: Mutex<Option<UsbError>>,
    pub halted_endpoints: Mutex<Vec<u8>>,
    pub closed: Mutex<bool>,
}

impl UsbDeviceHandle for MockUsbDeviceHandle {
    fn bulk_read<'a>(
        &'a self,
        _endpoint: u8,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(err) = self.should_fail_read.lock().await.clone() {
                return Err(err);
            }
            let mut data = self.read_data.lock().await;
            if data.is_empty() {
                return Ok(0);
            }
            let to_copy = std::cmp::min(buf.len(), data.len());
            buf[..to_copy].copy_from_slice(&data[..to_copy]);
            data.drain(..to_copy);
            Ok(to_copy)
        })
    }

    fn bulk_write<'a>(
        &'a self,
        _endpoint: u8,
        data: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(err) = self.should_fail_write.lock().await.clone() {
                return Err(err);
            }
            let mut written = self.written_data.lock().await;
            written.extend_from_slice(data);
            Ok(data.len())
        })
    }

    fn clear_halt<'a>(
        &'a self,
        endpoint: u8,
    ) -> Pin<Box<dyn Future<Output = Result<(), UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let mut halted = self.halted_endpoints.lock().await;
            halted.retain(|&ep| ep != endpoint);
            Ok(())
        })
    }

    fn close<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), UsbError>> + Send + 'a>> {
        Box::pin(async move {
            let mut closed = self.closed.lock().await;
            *closed = true;
            Ok(())
        })
    }
}

/// In-memory mock implementation of `UsbBackend`.
#[derive(Debug, Default, Clone)]
pub struct MockUsbBackend {
    devices: Arc<Mutex<Vec<UsbDeviceInfo>>>,
    list_error: Arc<Mutex<Option<UsbError>>>,
    open_error: Arc<Mutex<Option<UsbError>>>,
}

impl MockUsbBackend {
    /// Creates a new empty `MockUsbBackend`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the devices returned by `list_devices`.
    pub async fn set_devices(&self, devices: Vec<UsbDeviceInfo>) {
        let mut lock = self.devices.lock().await;
        *lock = devices;
    }

    /// Injects an error to be returned by `list_devices`.
    pub async fn set_list_error(&self, error: Option<UsbError>) {
        let mut lock = self.list_error.lock().await;
        *lock = error;
    }

    /// Injects an error to be returned by `open`.
    pub async fn set_open_error(&self, error: Option<UsbError>) {
        let mut lock = self.open_error.lock().await;
        *lock = error;
    }
}

impl UsbBackend for MockUsbBackend {
    fn list_devices<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<UsbDeviceInfo>, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(err) = self.list_error.lock().await.clone() {
                return Err(err);
            }
            let devices = self.devices.lock().await;
            Ok(devices.clone())
        })
    }

    fn open<'a>(
        &'a self,
        _device: &'a UsbDeviceInfo,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn UsbDeviceHandle>, UsbError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(err) = self.open_error.lock().await.clone() {
                return Err(err);
            }
            Ok(Box::new(MockUsbDeviceHandle::default()) as Box<dyn UsbDeviceHandle>)
        })
    }
}
