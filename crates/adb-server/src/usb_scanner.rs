//! USB device scanner task that discovers USB ADB devices and registers them in `DeviceRegistry`.
//!
//! Ported from `original/client/usb_linux.cpp` and `original/transport.cpp` (`register_usb_transport`).

use adb_transport::{find_adb_interface, UsbBackend};
use std::sync::Arc;
use std::time::Duration;

use crate::registry::DeviceRegistry;
use crate::transport_type::TransportType;

/// Periodically queries a `UsbBackend` and registers matching ADB USB devices into `DeviceRegistry`.
pub struct UsbDeviceScanner {
    backend: Arc<dyn UsbBackend>,
    registry: Arc<DeviceRegistry>,
}

impl UsbDeviceScanner {
    /// Creates a new `UsbDeviceScanner`.
    pub fn new(backend: Arc<dyn UsbBackend>, registry: Arc<DeviceRegistry>) -> Self {
        Self { backend, registry }
    }

    /// Performs a single scan pass, registering newly discovered ADB devices.
    ///
    /// Ignores non-ADB USB devices and rejects/ignores duplicate serial numbers per C++ behavior.
    pub async fn scan_once(&self) -> Result<usize, adb_transport::UsbError> {
        let devices = self.backend.list_devices().await?;
        let mut registered_count = 0;

        for dev in devices {
            // Check if device has an ADB interface
            if find_adb_interface(&dev.interfaces).is_none() {
                continue;
            }

            let serial = match dev.serial.as_deref() {
                Some(s) if !s.is_empty() => s,
                _ => continue, // Without a serial, cannot register device
            };

            // Register with USB transport type and devpath
            if self
                .registry
                .register_with_devpath(serial, TransportType::Usb, Some(dev.device_path))
                .is_ok()
            {
                registered_count += 1;
            }
        }

        Ok(registered_count)
    }

    /// Runs the scanner loop until cancelled.
    pub async fn run(&self, interval: Duration) {
        loop {
            let _ = self.scan_once().await;
            tokio::time::sleep(interval).await;
        }
    }
}
