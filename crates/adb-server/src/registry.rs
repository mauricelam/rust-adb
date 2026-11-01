//! Thread-safe device registry tracking active transports, serials, and transport IDs.
//!
//! Ported from `original/transport.cpp`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::state::ConnectionState;
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
}

/// Central thread-safe registry of connected devices and transports.
///
/// Ported from `transport_list` and `NextTransportId()` in `original/transport.cpp`.
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    next_id: AtomicU64,
    inner: RwLock<RegistryInner>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    by_id: HashMap<TransportId, Arc<DeviceEntry>>,
    by_serial: HashMap<String, TransportId>,
}

impl DeviceRegistry {
    /// Creates an empty `DeviceRegistry`.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
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

    /// Registers a new device transport with the given serial and transport type.
    ///
    /// Rejects duplicate registrations if a device with the same serial is already registered.
    /// Returns the allocated `TransportId` on success.
    pub fn register(
        &self,
        serial: &str,
        transport_type: TransportType,
    ) -> Result<TransportId, RegistrationError> {
        let mut inner = self.inner.write();

        if inner.by_serial.contains_key(serial) {
            return Err(RegistrationError::AlreadyRegistered);
        }

        let transport_id = self.next_transport_id();
        let entry = Arc::new(DeviceEntry::new(transport_id, serial, transport_type));

        inner.by_id.insert(transport_id, Arc::clone(&entry));
        inner.by_serial.insert(serial.to_string(), transport_id);

        Ok(transport_id)
    }

    /// Unregisters a device by its transport ID.
    pub fn unregister_by_id(&self, transport_id: TransportId) -> Option<Arc<DeviceEntry>> {
        let mut inner = self.inner.write();
        if let Some(entry) = inner.by_id.remove(&transport_id) {
            inner.by_serial.remove(&entry.serial);
            Some(entry)
        } else {
            None
        }
    }

    /// Unregisters a device by its serial number.
    pub fn unregister_by_serial(&self, serial: &str) -> Option<Arc<DeviceEntry>> {
        let mut inner = self.inner.write();
        if let Some(id) = inner.by_serial.remove(serial) {
            inner.by_id.remove(&id)
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
            true
        } else {
            false
        }
    }
}
