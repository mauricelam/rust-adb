//! Thread-safe device registry tracking active transports, serials, and transport IDs.
//!
//! Ported from `original/transport.cpp`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::state::ConnectionState;
use crate::tracker::{format_devices_framed, TrackerFormat};
use crate::transport_type::TransportType;

/// Monotonically increasing transport ID, starting at 1.
pub type TransportId = u64;

/// Error occurring during device registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationError {
    /// A device with this serial number is already registered.
    ///
    /// Ported from `validate_transport_list` in `original/transport.cpp` returning `EALREADY`.
    AlreadyRegistered,
}

impl std::fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyRegistered => write!(f, "device is already registered"),
        }
    }
}

impl std::error::Error for RegistrationError {}

/// Representation of a registered device / transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    /// Unique monotonic transport ID.
    pub transport_id: TransportId,
    /// Device serial number (e.g. `emulator-5554`, `HT44...`, `127.0.0.1:5555`).
    pub serial: String,
    /// Physical or virtual transport type.
    pub transport_type: TransportType,
    /// Current connection / authorization state.
    pub state: ConnectionState,
    /// Product name (e.g., `walleye`), if available.
    pub product: Option<String>,
    /// Model name (e.g., `Pixel 2`), if available.
    pub model: Option<String>,
    /// Device name (e.g., `walleye`), if available.
    pub device: Option<String>,
    /// Device path, if available.
    pub devpath: Option<String>,
    /// Negotiated features list.
    pub features: Vec<String>,
}

impl DeviceEntry {
    /// Creates a new `DeviceEntry` with default `Offline` state.
    pub fn new(
        transport_id: TransportId,
        serial: impl Into<String>,
        transport_type: TransportType,
    ) -> Self {
        Self {
            transport_id,
            serial: serial.into(),
            transport_type,
            state: ConnectionState::Offline,
            product: None,
            model: None,
            device: None,
            devpath: None,
            features: Vec::new(),
        }
    }

    /// Checks if this transport entry matches the given target string.
    ///
    /// Ported from `atransport::MatchesTarget` in `original/transport.cpp`.
    pub fn matches_target(&self, target: &str) -> bool {
        if !self.serial.is_empty() {
            if target == self.serial {
                return true;
            } else if self.transport_type == TransportType::Local {
                let mut local_target = target;
                if let Some(rest) = target.strip_prefix("tcp:") {
                    local_target = rest;
                } else if let Some(rest) = target.strip_prefix("udp:") {
                    local_target = rest;
                }

                // Check host/port match
                if let Some((serial_host, serial_port)) = parse_host_port(&self.serial) {
                    if let Some((target_host, target_port)) = parse_host_port(local_target) {
                        let effective_target_port = if target_port == 0 {
                            serial_port
                        } else {
                            target_port
                        };
                        if serial_host == target_host && serial_port == effective_target_port {
                            return true;
                        }
                    }
                }
            }
        }

        if let Some(ref dp) = self.devpath {
            if target == dp {
                return true;
            }
        }

        if let Some(rest) = target.strip_prefix("product:") {
            if let Some(ref p) = self.product {
                if rest == p {
                    return true;
                }
            }
        }

        if let Some(rest) = target.strip_prefix("model:") {
            if let Some(ref m) = self.model {
                let sanitized: String = m
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                if rest == sanitized {
                    return true;
                }
            }
        }

        if let Some(rest) = target.strip_prefix("device:") {
            if let Some(ref d) = self.device {
                if rest == d {
                    return true;
                }
            }
        }

        false
    }
}

fn parse_host_port(s: &str) -> Option<(String, u16)> {
    if s.is_empty() {
        return None;
    }
    if let Some(colon) = s.rfind(':') {
        let host = &s[..colon];
        let port_str = &s[colon + 1..];
        if let Ok(port) = port_str.parse::<u16>() {
            Some((host.to_string(), port))
        } else {
            None
        }
    } else {
        Some((s.to_string(), 0))
    }
}

#[derive(Debug)]
struct Subscriber {
    format: TrackerFormat,
    sender: mpsc::UnboundedSender<Vec<u8>>,
}

/// Central thread-safe registry of connected devices and transports.
///
/// Ported from `transport_list`, `NextTransportId()`, and `update_transports()` in `original/transport.cpp`.
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    next_id: AtomicU64,
    next_subscriber_id: AtomicU64,
    inner: RwLock<RegistryInner>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    by_id: HashMap<TransportId, Arc<DeviceEntry>>,
    by_serial: HashMap<String, TransportId>,
    subscribers: HashMap<u64, Subscriber>,
}

impl DeviceRegistry {
    /// Creates an empty `DeviceRegistry`.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            next_subscriber_id: AtomicU64::new(1),
            inner: RwLock::new(RegistryInner::default()),
        }
    }

    /// Allocates the next monotonic transport ID.
    ///
    /// Ported from `NextTransportId()` in `original/transport.cpp`:
    /// ```cpp
    /// TransportId NextTransportId() {
    ///     static std::atomic<TransportId> next(1);
    ///     return next++;
    /// }
    /// ```
    pub fn next_transport_id(&self) -> TransportId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Subscribes to device change notifications in the specified `TrackerFormat`.
    ///
    /// The receiver receives an initial notification immediately with current devices,
    /// followed by framed update payloads whenever devices are registered, unregistered,
    /// or change state/properties.
    ///
    /// Ported from `create_device_tracker` and `device_tracker_ready` in `original/transport.cpp`.
    pub fn subscribe(&self, format: TrackerFormat) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let sub_id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);

        {
            let mut inner = self.inner.write();
            let devices: Vec<Arc<DeviceEntry>> = inner.by_id.values().cloned().collect();
            let payload = format_devices_framed(&devices, format);
            // Send initial state while holding write lock to prevent reordering with concurrent updates
            let _ = tx.send(payload);
            inner
                .subscribers
                .insert(sub_id, Subscriber { format, sender: tx });
        }

        rx
    }

    /// Internal helper to notify all active subscribers.
    ///
    /// Cleans up any subscribers whose channel receiver has been dropped.
    /// Ported from `update_transports()` in `original/transport.cpp`.
    fn notify_subscribers(&self, inner: &mut RegistryInner) {
        if inner.subscribers.is_empty() {
            return;
        }

        let devices: Vec<Arc<DeviceEntry>> = inner.by_id.values().cloned().collect();

        // Cached formats to avoid re-formatting for multiple subscribers of the same format
        let mut short_text: Option<Vec<u8>> = None;
        let mut long_text: Option<Vec<u8>> = None;
        let mut proto_bin: Option<Vec<u8>> = None;
        let mut proto_text: Option<Vec<u8>> = None;

        inner.subscribers.retain(|_, sub| {
            let payload =
                match sub.format {
                    TrackerFormat::ShortText => short_text
                        .get_or_insert_with(|| format_devices_framed(&devices, sub.format)),
                    TrackerFormat::LongText => {
                        long_text.get_or_insert_with(|| format_devices_framed(&devices, sub.format))
                    }
                    TrackerFormat::ProtoBinary => {
                        proto_bin.get_or_insert_with(|| format_devices_framed(&devices, sub.format))
                    }
                    TrackerFormat::ProtoText => proto_text
                        .get_or_insert_with(|| format_devices_framed(&devices, sub.format)),
                };
            sub.sender.send(payload.clone()).is_ok()
        });
    }

    /// Registers a new device transport with the given serial and transport type.
    ///
    /// Rejects duplicate registrations if a device with the same serial is already registered.
    /// Returns the allocated `TransportId` on success.
    pub fn register(
        &self,
        serial: &str,
        transport_type: TransportType,
    ) -> Result<TransportId, RegistrationError> {
        self.register_with_devpath(serial, transport_type, None)
    }

    /// Registers a new device transport with serial, transport type, and optional devpath.
    ///
    /// Rejects duplicate registrations if a device with the same serial is already registered.
    /// Returns the allocated `TransportId` on success.
    pub fn register_with_devpath(
        &self,
        serial: &str,
        transport_type: TransportType,
        devpath: Option<String>,
    ) -> Result<TransportId, RegistrationError> {
        let mut inner = self.inner.write();

        if inner.by_serial.contains_key(serial) {
            return Err(RegistrationError::AlreadyRegistered);
        }

        let transport_id = self.next_transport_id();
        let mut entry = DeviceEntry::new(transport_id, serial, transport_type);
        entry.devpath = devpath;
        let entry = Arc::new(entry);

        inner.by_id.insert(transport_id, Arc::clone(&entry));
        inner.by_serial.insert(serial.to_string(), transport_id);

        self.notify_subscribers(&mut inner);

        Ok(transport_id)
    }

    /// Unregisters a device by its transport ID.
    pub fn unregister_by_id(&self, transport_id: TransportId) -> Option<Arc<DeviceEntry>> {
        let mut inner = self.inner.write();
        if let Some(entry) = inner.by_id.remove(&transport_id) {
            inner.by_serial.remove(&entry.serial);
            self.notify_subscribers(&mut inner);
            Some(entry)
        } else {
            None
        }
    }

    /// Unregisters a device by its serial number.
    pub fn unregister_by_serial(&self, serial: &str) -> Option<Arc<DeviceEntry>> {
        let mut inner = self.inner.write();
        if let Some(id) = inner.by_serial.remove(serial) {
            let entry = inner.by_id.remove(&id);
            self.notify_subscribers(&mut inner);
            entry
        } else {
            None
        }
    }

    /// Retrieves a device entry by its transport ID.
    pub fn get_by_id(&self, transport_id: TransportId) -> Option<Arc<DeviceEntry>> {
        let inner = self.inner.read();
        inner.by_id.get(&transport_id).cloned()
    }

    /// Retrieves a device entry by its serial number.
    pub fn get_by_serial(&self, serial: &str) -> Option<Arc<DeviceEntry>> {
        let inner = self.inner.read();
        let id = inner.by_serial.get(serial)?;
        inner.by_id.get(id).cloned()
    }

    /// Returns a snapshot list of all currently registered devices.
    pub fn list_devices(&self) -> Vec<Arc<DeviceEntry>> {
        let inner = self.inner.read();
        inner.by_id.values().cloned().collect()
    }

    /// Updates the connection state of a device by its transport ID.
    /// Returns `true` if the device was found and updated.
    pub fn update_state(&self, transport_id: TransportId, state: ConnectionState) -> bool {
        let mut inner = self.inner.write();
        if let Some(entry) = inner.by_id.get_mut(&transport_id) {
            let mut updated = (**entry).clone();
            updated.state = state;
            *entry = Arc::new(updated);
            self.notify_subscribers(&mut inner);
            true
        } else {
            false
        }
    }

    /// Updates device property attributes (product, model, device).
    /// Returns `true` if the device was found and updated.
    pub fn set_device_properties(
        &self,
        transport_id: TransportId,
        product: Option<String>,
        model: Option<String>,
        device: Option<String>,
    ) -> bool {
        let mut inner = self.inner.write();
        if let Some(entry) = inner.by_id.get_mut(&transport_id) {
            let mut updated = (**entry).clone();
            updated.product = product;
            updated.model = model;
            updated.device = device;
            *entry = Arc::new(updated);
            self.notify_subscribers(&mut inner);
            true
        } else {
            false
        }
    }
}
