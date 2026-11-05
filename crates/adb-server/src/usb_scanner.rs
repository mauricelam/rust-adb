//! USB device scanner task that discovers USB ADB devices and registers them in `DeviceRegistry`.
//!
//! Ported from `original/client/usb_linux.cpp`, `original/client/usb_libusb.cpp`,
//! and `original/transport.cpp` (`register_usb_transport`, `device_disconnected`).

use adb_transport::{find_adb_interface, UsbBackend, UsbDeviceInfo, UsbHotplugEvent};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::registry::DeviceRegistry;
use crate::transport_type::TransportType;

/// Periodically queries a `UsbBackend` and listens for hotplug events, registering matching ADB
/// USB devices into `DeviceRegistry` and unregistering detached devices.
pub struct UsbDeviceScanner {
    backend: Arc<dyn UsbBackend>,
    registry: Arc<DeviceRegistry>,
    debounce_duration: Duration,
}

impl UsbDeviceScanner {
    /// Creates a new `UsbDeviceScanner` with default debounce settings.
    pub fn new(backend: Arc<dyn UsbBackend>, registry: Arc<DeviceRegistry>) -> Self {
        Self {
            backend,
            registry,
            debounce_duration: Duration::from_millis(50),
        }
    }

    /// Creates a new `UsbDeviceScanner` with a specified debounce duration.
    pub fn with_debounce(
        backend: Arc<dyn UsbBackend>,
        registry: Arc<DeviceRegistry>,
        debounce_duration: Duration,
    ) -> Self {
        Self {
            backend,
            registry,
            debounce_duration,
        }
    }

    /// Handles attaching a discovered USB device.
    pub fn handle_device_attached(&self, dev: &UsbDeviceInfo) -> bool {
        if find_adb_interface(&dev.interfaces).is_none() {
            return false;
        }

        let serial = match dev.serial.as_deref() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };

        self.registry
            .register_with_devpath(serial, TransportType::Usb, Some(dev.device_path.clone()))
            .is_ok()
    }

    /// Handles detaching a USB device by path or serial.
    pub fn handle_device_detached(&self, path_or_serial: &str) -> bool {
        if self
            .registry
            .unregister_by_devpath(path_or_serial)
            .is_some()
        {
            return true;
        }
        self.registry.unregister_by_serial(path_or_serial).is_some()
    }

    /// Performs a single scan pass, registering newly discovered ADB devices and pruning absent ones.
    ///
    /// Ignores non-ADB USB devices and rejects duplicate serial numbers per C++ behavior.
    pub async fn scan_once(&self) -> Result<usize, adb_transport::UsbError> {
        let devices = self.backend.list_devices().await?;
        let mut registered_count = 0;
        let mut current_paths = HashSet::new();
        let mut current_serials = HashSet::new();

        for dev in devices {
            if find_adb_interface(&dev.interfaces).is_none() {
                continue;
            }

            let serial = match dev.serial.as_deref() {
                Some(s) if !s.is_empty() => s,
                _ => continue,
            };

            current_paths.insert(dev.device_path.clone());
            current_serials.insert(serial.to_string());

            if self
                .registry
                .register_with_devpath(serial, TransportType::Usb, Some(dev.device_path))
                .is_ok()
            {
                registered_count += 1;
            }
        }

        // For polling mode: unregister any previously registered USB devices that disappeared
        for entry in self.registry.list_devices() {
            if entry.transport_type == TransportType::Usb {
                let path_present = entry
                    .devpath
                    .as_ref()
                    .map(|p| current_paths.contains(p))
                    .unwrap_or(false);
                let serial_present = current_serials.contains(&entry.serial);

                if !path_present && !serial_present {
                    let _ = self.registry.unregister_by_id(entry.transport_id);
                }
            }
        }

        Ok(registered_count)
    }

    /// Processes an incoming hotplug event, applying debounce windowing when configured.
    pub async fn process_hotplug_event(&self, event: UsbHotplugEvent) {
        match event {
            UsbHotplugEvent::Attached(dev) => {
                if !self.debounce_duration.is_zero() {
                    tokio::time::sleep(self.debounce_duration).await;
                    // Check if device is still present after debounce duration
                    if let Ok(current_devs) = self.backend.list_devices().await {
                        let still_present = current_devs.iter().any(|d| {
                            d.device_path == dev.device_path
                                || (dev.serial.is_some() && d.serial == dev.serial)
                        });
                        if !still_present {
                            return;
                        }
                    }
                }
                let _ = self.handle_device_attached(&dev);
            }
            UsbHotplugEvent::Detached(path_or_serial) => {
                let _ = self.handle_device_detached(&path_or_serial);
            }
        }
    }

    /// Runs the scanner loop, processing hotplug events and periodic polling passes.
    pub async fn run(&self, interval: Duration) {
        let mut event_rx = match self.backend.subscribe_hotplug_events().await {
            Ok(rx) => Some(rx),
            Err(_) => None,
        };

        // Initial scan pass
        let _ = self.scan_once().await;

        loop {
            tokio::select! {
                Some(event) = async {
                    match event_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    self.process_hotplug_event(event).await;
                }
                _ = tokio::time::sleep(interval) => {
                    let _ = self.scan_once().await;
                }
            }
        }
    }
}
