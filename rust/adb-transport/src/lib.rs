/*
 * Copyright (C) 2007 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! ADB Transport Layer
//!
//! Ported from:
//! - original/transport.h
//! - original/transport.cpp
//! - original/adb.cpp (parse_banner)

use std::sync::atomic::{AtomicU64, AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::os::unix::io::OwnedFd;
use std::io::{Read, Write};
use std::fs::File;

use adb_types::{Amessage, Apacket, Block};
use rust_adb_crypto::Key;
use adb_protocol::{TransportType, ConnectionState, MAX_PAYLOAD, A_VERSION_MIN};

/// Ported from original/adb.h: `using TransportId = uint64_t;`
pub type TransportId = u64;

static NEXT_TRANSPORT_ID: AtomicU64 = AtomicU64::new(1);

/// Ported from original/transport.cpp: `TransportId NextTransportId()`
pub fn next_transport_id() -> TransportId {
    NEXT_TRANSPORT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Ported from original/transport.h: `using FeatureSet = std::vector<std::string>;`
pub type FeatureSet = Vec<String>;

pub const FEATURE_SHELL2: &str = "shell_v2";
pub const FEATURE_CMD: &str = "cmd";
pub const FEATURE_STAT2: &str = "stat_v2";
pub const FEATURE_LS2: &str = "ls_v2";
pub const FEATURE_LIBUSB: &str = "libusb";
pub const FEATURE_PUSH_SYNC: &str = "push_sync";
pub const FEATURE_APEX: &str = "apex";
pub const FEATURE_FIXED_PUSH_MKDIR: &str = "fixed_push_mkdir";
pub const FEATURE_ABB: &str = "abb";
pub const FEATURE_FIXED_PUSH_SYMLINK_TIMESTAMP: &str = "fixed_push_symlink_timestamp";
pub const FEATURE_ABB_EXEC: &str = "abb_exec";
pub const FEATURE_REMOUNT_SHELL: &str = "remount_shell";
pub const FEATURE_TRACK_APP: &str = "track_app";
pub const FEATURE_SENDRECV_V2: &str = "sendrecv_v2";
pub const FEATURE_SENDRECV_V2_BROTLI: &str = "sendrecv_v2_brotli";
pub const FEATURE_SENDRECV_V2_LZ4: &str = "sendrecv_v2_lz4";
pub const FEATURE_SENDRECV_V2_ZSTD: &str = "sendrecv_v2_zstd";
pub const FEATURE_SENDRECV_V2_DRY_RUN_SEND: &str = "sendrecv_v2_dry_run_send";
pub const FEATURE_DELAYED_ACK: &str = "delayed_ack";
pub const FEATURE_OPENSCREEN_MDNS: &str = "openscreen_mdns";
pub const FEATURE_DEVICE_TRACKER_PROTO_FORMAT: &str = "devicetracker_proto_format";
pub const FEATURE_DEVRAW: &str = "devraw";
pub const FEATURE_APP_INFO: &str = "app_info";
pub const FEATURE_SERVER_STATUS: &str = "server_status";

/// Ported from original/transport.cpp: `const FeatureSet& supported_features()`
pub fn supported_features() -> &'static FeatureSet {
    static FEATURES: OnceLock<FeatureSet> = OnceLock::new();
    FEATURES.get_or_init(|| {
        let mut result = vec![
            FEATURE_SHELL2.to_string(),
            FEATURE_CMD.to_string(),
            FEATURE_STAT2.to_string(),
            FEATURE_LS2.to_string(),
            FEATURE_FIXED_PUSH_MKDIR.to_string(),
            FEATURE_APEX.to_string(),
            FEATURE_ABB.to_string(),
            FEATURE_FIXED_PUSH_SYMLINK_TIMESTAMP.to_string(),
            FEATURE_ABB_EXEC.to_string(),
            FEATURE_REMOUNT_SHELL.to_string(),
            FEATURE_TRACK_APP.to_string(),
            FEATURE_SENDRECV_V2.to_string(),
            FEATURE_SENDRECV_V2_BROTLI.to_string(),
            FEATURE_SENDRECV_V2_LZ4.to_string(),
            FEATURE_SENDRECV_V2_ZSTD.to_string(),
            FEATURE_SENDRECV_V2_DRY_RUN_SEND.to_string(),
            FEATURE_OPENSCREEN_MDNS.to_string(),
            FEATURE_DEVICE_TRACKER_PROTO_FORMAT.to_string(),
            FEATURE_DEVRAW.to_string(),
            FEATURE_APP_INFO.to_string(),
            FEATURE_SERVER_STATUS.to_string(),
        ];

        // ADB_HOST check - using a simplified logic for now
        result.push(FEATURE_DELAYED_ACK.to_string());

        result
    })
}

/// Ported from original/transport.cpp: `std::string FeatureSetToString(const FeatureSet& features)`
pub fn feature_set_to_string(features: &FeatureSet) -> String {
    features.join(",")
}

/// Ported from original/transport.cpp: `FeatureSet StringToFeatureSet(const std::string& features_string)`
pub fn string_to_feature_set(features_string: &str) -> FeatureSet {
    if features_string.is_empty() {
        return FeatureSet::new();
    }
    features_string.split(',').map(|s| s.to_string()).collect()
}

/// Ported from original/transport.cpp: `bool CanUseFeature(const FeatureSet& feature_set, const std::string& feature)`
pub fn can_use_feature(feature_set: &FeatureSet, feature: &str) -> bool {
    feature_set.iter().any(|f| f == feature) && supported_features().iter().any(|f| f == feature)
}

/// Ported from original/transport.h: `enum class ReconnectResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectResult {
    Retry,
    Success,
    Abort,
}

pub type ReconnectCallback = Box<dyn Fn(&ATransport) -> ReconnectResult + Send + Sync>;

pub trait DisconnectHandler: Send + Sync {
    fn on_disconnect(&self, transport: &ATransport);
}

/// Ported from original/transport.h: `class atransport`
pub struct ATransport {
    pub id: TransportId,
    pub serial: Mutex<String>,
    pub product: Mutex<String>,
    pub model: Mutex<String>,
    pub device: Mutex<String>,
    pub devpath: Mutex<String>,
    pub transport_type: TransportType,

    kicked: AtomicBool,
    connection_state: AtomicI32,
    connection: Mutex<Option<Arc<dyn Connection>>>,
    reconnect: ReconnectCallback,

    features: Mutex<FeatureSet>,
    protocol_version: AtomicI32,
    max_payload: Mutex<usize>,

    disconnects: Mutex<Vec<Box<dyn DisconnectHandler>>>,
}

impl ATransport {
    pub fn new(
        transport_type: TransportType,
        reconnect: ReconnectCallback,
        state: ConnectionState,
    ) -> Self {
        Self {
            id: next_transport_id(),
            serial: Mutex::new(String::new()),
            product: Mutex::new(String::new()),
            model: Mutex::new(String::new()),
            device: Mutex::new(String::new()),
            devpath: Mutex::new(String::new()),
            transport_type,
            kicked: AtomicBool::new(false),
            connection_state: AtomicI32::new(state as i32),
            connection: Mutex::new(None),
            reconnect,
            features: Mutex::new(Vec::new()),
            protocol_version: AtomicI32::new(A_VERSION_MIN as i32),
            max_payload: Mutex::new(MAX_PAYLOAD),
            disconnects: Mutex::new(Vec::new()),
        }
    }

    pub fn new_offline(transport_type: TransportType) -> Self {
        Self::new(
            transport_type,
            Box::new(|_| ReconnectResult::Abort),
            ConnectionState::Offline,
        )
    }

    pub fn write(&self, packet: Apacket) -> bool {
        if let Some(conn) = self.connection.lock().unwrap().as_ref() {
            conn.write(packet)
        } else {
            false
        }
    }

    pub fn kick(&self) {
        if !self.kicked.swap(true, Ordering::SeqCst) {
            if let Some(conn) = self.connection.lock().unwrap().as_ref() {
                conn.stop();
            }
            self.run_disconnects();
        }
    }

    pub fn is_kicked(&self) -> bool {
        self.kicked.load(Ordering::SeqCst)
    }

    pub fn get_connection_state(&self) -> ConnectionState {
        ConnectionState::try_from(self.connection_state.load(Ordering::SeqCst))
            .unwrap_or(ConnectionState::Offline)
    }

    pub fn set_connection_state(&self, state: ConnectionState) {
        self.connection_state.store(state as i32, Ordering::SeqCst);
        update_transports();
    }

    pub fn set_connection(&self, connection: Arc<dyn Connection>) {
        let mut conn_lock = self.connection.lock().unwrap();
        *conn_lock = Some(connection);
    }

    /// Ported from original/transport.cpp: `bool atransport::HandleRead(std::unique_ptr<apacket> p)`
    pub fn handle_read(&self, packet: Apacket) -> bool {
        log::debug!(
            target: "transport",
            "{} remote read: {} {}",
            self.serial.lock().unwrap(),
            adb_protocol::command_to_string(packet.msg.command),
            packet.msg.data_length
        );

        // TODO: This should run on the looper thread in the full implementation.
        handle_packet(packet, self);

        true
    }

    pub fn handle_error(&self, _error: &str) {
        self.kick();
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.lock().unwrap().contains(&feature.to_string())
    }

    pub fn set_features(&self, features_string: &str) {
        let mut features = self.features.lock().unwrap();
        *features = string_to_feature_set(features_string);
    }

    pub fn add_disconnect(&self, handler: Box<dyn DisconnectHandler>) {
        self.disconnects.lock().unwrap().push(handler);
    }

    pub fn run_disconnects(&self) {
        let mut handlers = self.disconnects.lock().unwrap();
        for handler in handlers.drain(..) {
            handler.on_disconnect(self);
        }
    }

    pub fn get_protocol_version(&self) -> i32 {
        self.protocol_version.load(Ordering::SeqCst)
    }

    pub fn get_max_payload(&self) -> usize {
        *self.max_payload.lock().unwrap()
    }

    pub fn matches_target(&self, target: &str) -> bool {
        let serial = self.serial.lock().unwrap();
        if !serial.is_empty() {
            if target == *serial {
                return true;
            }
            // TODO: handle local transport network address matching
        }

        let devpath = self.devpath.lock().unwrap();
        if target == *devpath {
            return true;
        }

        if let Some(product) = target.strip_prefix("product:") {
            return *self.product.lock().unwrap() == product;
        }
        if let Some(model) = target.strip_prefix("model:") {
            return *self.model.lock().unwrap() == model;
        }
        if let Some(device) = target.strip_prefix("device:") {
            return *self.device.lock().unwrap() == device;
        }

        false
    }

    pub fn reconnect(&self) -> ReconnectResult {
        (self.reconnect)(self)
    }
}

pub fn pending_list() -> &'static Mutex<Vec<Arc<ATransport>>> {
    static LIST: OnceLock<Mutex<Vec<Arc<ATransport>>>> = OnceLock::new();
    LIST.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn transport_list() -> &'static Mutex<Vec<Arc<ATransport>>> {
    static LIST: OnceLock<Mutex<Vec<Arc<ATransport>>>> = OnceLock::new();
    LIST.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register_transport(transport: Arc<ATransport>) {
    transport_list().lock().unwrap().push(transport);
}

pub fn kick_transport(transport: &ATransport) {
    transport.kick();
}

pub fn kick_all_transports() {
    let list = transport_list().lock().unwrap();
    for t in list.iter() {
        t.kick();
    }
}

pub fn acquire_one_transport(
    transport_type: TransportType,
    serial: Option<&str>,
    transport_id: TransportId,
) -> Result<Arc<ATransport>, String> {
    let list = transport_list().lock().unwrap();
    let mut result: Option<Arc<ATransport>> = None;
    let mut ambiguous = false;

    for t in list.iter() {
        if transport_id != 0 {
            if t.id == transport_id {
                result = Some(t.clone());
                break;
            }
        } else if let Some(s) = serial {
            if t.matches_target(s) {
                if result.is_some() {
                    ambiguous = true;
                    result = None;
                    break;
                }
                result = Some(t.clone());
            }
        } else {
            // Match by transport type
            let matches = match (transport_type, t.transport_type) {
                (TransportType::Any, _) => true,
                (TransportType::Usb, TransportType::Usb) => true,
                (TransportType::Local, TransportType::Local) => true,
                _ => false,
            };

            if matches {
                if result.is_some() {
                    ambiguous = true;
                    result = None;
                    break;
                }
                result = Some(t.clone());
            }
        }
    }

    if ambiguous {
        return Err("more than one device/emulator".to_string());
    }

    match result {
        Some(t) => Ok(t),
        None => Err("device not found".to_string()),
    }
}

/// Parses a banner string and updates the transport's properties.
///
/// Ported from original/adb.cpp: `void parse_banner(const std::string& banner, atransport* t)`
///
/// # Arguments
/// * `banner` - The banner string sent by the remote end (e.g., "device::ro.product.name=x;...").
/// * `t` - The transport to update.
///
/// Example banner string:
/// "device::ro.product.name=x;ro.product.model=y;ro.product.device=z;features=shell_v2,cmd"
pub fn parse_banner(banner: &str, t: &ATransport) {
    log::debug!(target: "transport", "parse_banner: {}", banner);

    let pieces: Vec<&str> = banner.split(':').collect();

    // Reset the features list or else if the server sends no features we may
    // keep the existing feature set (http://b/24405971).
    t.set_features("");

    if pieces.len() > 2 {
        let props = pieces[2];
        for prop in props.split(';') {
            // The list of properties was traditionally ;-terminated rather than ;-separated.
            if prop.is_empty() {
                continue;
            }
            let kv: Vec<&str> = prop.split('=').collect();
            if kv.len() != 2 {
                continue;
            }
            let key = kv[0];
            let value = kv[1];
            match key {
                "ro.product.name" => *t.product.lock().unwrap() = value.to_string(),
                "ro.product.model" => *t.model.lock().unwrap() = value.to_string(),
                "ro.product.device" => *t.device.lock().unwrap() = value.to_string(),
                "features" => t.set_features(value),
                _ => {}
            }
        }
    }

    if let Some(&type_str) = pieces.get(0) {
        let state = match type_str {
            "bootloader" => {
                log::debug!(target: "transport", "setting connection_state to kCsBootloader");
                ConnectionState::Bootloader
            }
            "device" => {
                log::debug!(target: "transport", "setting connection_state to kCsDevice");
                ConnectionState::Device
            }
            "recovery" => {
                log::debug!(target: "transport", "setting connection_state to kCsRecovery");
                ConnectionState::Recovery
            }
            "sideload" => {
                log::debug!(target: "transport", "setting connection_state to kCsSideload");
                ConnectionState::Sideload
            }
            "rescue" => {
                log::debug!(target: "transport", "setting connection_state to kCsRescue");
                ConnectionState::Rescue
            }
            _ => {
                log::debug!(target: "transport", "setting connection_state to kCsHost");
                ConnectionState::Host
            }
        };
        t.set_connection_state(state);
    }
}

pub fn handle_packet(_packet: Apacket, _t: &ATransport) {
    // TODO: implement handle_packet logic (A_OPEN, A_CLSE, etc.)
}

pub fn update_transports() {
    log::debug!(target: "transport", "update_transports");
    // TODO: Notify `adb track-devices` clients once device_tracker is ported.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_transport_id() {
        let id1 = next_transport_id();
        let id2 = next_transport_id();
        assert!(id1 > 0);
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn test_feature_set_to_string() {
        let features = vec!["foo".to_string(), "bar".to_string()];
        assert_eq!(feature_set_to_string(&features), "foo,bar");
    }

    #[test]
    fn test_string_to_feature_set() {
        let features = string_to_feature_set("foo,bar");
        assert_eq!(features.len(), 2);
        assert_eq!(features[0], "foo");
        assert_eq!(features[1], "bar");

        let empty = string_to_feature_set("");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_can_use_feature() {
        let features = vec!["shell_v2".to_string(), "cmd".to_string()];
        assert!(can_use_feature(&features, "shell_v2"));
        assert!(!can_use_feature(&features, "unknown_feature"));
    }

    struct Counter {
        count: Mutex<i32>,
    }
    impl DisconnectHandler for Counter {
        fn on_disconnect(&self, _transport: &ATransport) {
            let mut count = self.count.lock().unwrap();
            *count += 1;
        }
    }

    #[test]
    fn test_run_disconnects() {
        let t = ATransport::new_offline(TransportType::Local);
        let counter = Arc::new(Counter {
            count: Mutex::new(0),
        });

        struct Wrapper(Arc<Counter>);
        impl DisconnectHandler for Wrapper {
            fn on_disconnect(&self, t: &ATransport) {
                self.0.on_disconnect(t);
            }
        }

        t.add_disconnect(Box::new(Wrapper(counter.clone())));
        t.run_disconnects();
        assert_eq!(*counter.count.lock().unwrap(), 1);

        // Disconnects should have been removed.
        t.run_disconnects();
        assert_eq!(*counter.count.lock().unwrap(), 1);
    }

    #[test]
    fn test_set_features() {
        let t = ATransport::new_offline(TransportType::Local);
        assert!(!t.has_feature("foo"));

        t.set_features("foo,bar");
        assert!(t.has_feature("foo"));
        assert!(t.has_feature("bar"));
        assert!(!t.has_feature("baz"));
    }

    #[test]
    fn test_parse_banner_no_features() {
        let t = ATransport::new_offline(TransportType::Local);
        parse_banner("host::", &t);

        assert_eq!(t.get_connection_state(), ConnectionState::Host);
        assert!(t.features.lock().unwrap().is_empty());
    }

    #[test]
    fn test_parse_banner_product_features() {
        let t = ATransport::new_offline(TransportType::Local);
        let banner = "host::ro.product.name=foo;ro.product.model=bar;ro.product.device=baz;";
        parse_banner(banner, &t);

        assert_eq!(t.get_connection_state(), ConnectionState::Host);
        assert_eq!(*t.product.lock().unwrap(), "foo");
        assert_eq!(*t.model.lock().unwrap(), "bar");
        assert_eq!(*t.device.lock().unwrap(), "baz");
    }

    #[test]
    fn test_parse_banner_features() {
        let t = ATransport::new_offline(TransportType::Local);
        let banner = "host::ro.product.name=foo;features=woodly,doodly";
        parse_banner(banner, &t);

        assert_eq!(t.get_connection_state(), ConnectionState::Host);
        assert_eq!(*t.product.lock().unwrap(), "foo");
        assert!(t.has_feature("woodly"));
        assert!(t.has_feature("doodly"));
    }

    #[test]
    fn test_matches_target() {
        let t = ATransport::new_offline(TransportType::Usb);
        *t.serial.lock().unwrap() = "foo".to_string();
        *t.devpath.lock().unwrap() = "/path/to/bar".to_string();
        *t.product.lock().unwrap() = "test_product".to_string();
        *t.model.lock().unwrap() = "test_model".to_string();
        *t.device.lock().unwrap() = "test_device".to_string();

        assert!(t.matches_target("foo"));
        assert!(t.matches_target("/path/to/bar"));
        assert!(t.matches_target("product:test_product"));
        assert!(t.matches_target("model:test_model"));
        assert!(t.matches_target("device:test_device"));

        assert!(!t.matches_target("test_product"));
        assert!(!t.matches_target("bar"));
    }

    struct MockConnection {
        stopped: AtomicBool,
    }

    impl MockConnection {
        fn new() -> Self {
            Self {
                stopped: AtomicBool::new(false),
            }
        }
    }

    impl Connection for MockConnection {
        fn write(&self, _packet: Apacket) -> bool {
            true
        }
        fn start(&self) -> bool {
            true
        }
        fn stop(&self) {
            self.stopped.store(true, Ordering::SeqCst);
        }
        fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
            true
        }
        fn reset(&self) {}
    }

    #[test]
    fn test_state_transitions() {
        let t = Arc::new(ATransport::new_offline(TransportType::Usb));
        assert_eq!(t.get_connection_state(), ConnectionState::Offline);

        t.set_connection_state(ConnectionState::Connecting);
        assert_eq!(t.get_connection_state(), ConnectionState::Connecting);

        t.set_connection_state(ConnectionState::Authorizing);
        assert_eq!(t.get_connection_state(), ConnectionState::Authorizing);

        t.set_connection_state(ConnectionState::Device);
        assert_eq!(t.get_connection_state(), ConnectionState::Device);

        let conn = Arc::new(MockConnection::new());
        t.set_connection(conn.clone());

        t.kick();
        assert!(t.is_kicked());
        assert!(conn.stopped.load(Ordering::SeqCst));
    }
}

/// Ported from original/transport.h: `struct Connection`
pub trait Connection: Send + Sync {
    fn write(&self, packet: Apacket) -> bool;
    fn start(&self) -> bool;
    fn stop(&self);
    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool;
    fn reset(&self);
    fn supports_detach(&self) -> bool { false }
    fn attach(&self) -> Result<(), String> { Err("transport type doesn't support attach".to_string()) }
    fn detach(&self) -> Result<(), String> { Err("transport type doesn't support detach".to_string()) }
    fn negotiated_speed_mbps(&self) -> u64 { 0 }
    fn max_speed_mbps(&self) -> u64 { 0 }
}

/// Ported from original/transport.h: `struct BlockingConnection`
pub trait BlockingConnection: Send + Sync {
    fn read(&self) -> std::io::Result<Apacket>;
    fn write(&self, packet: &Apacket) -> std::io::Result<()>;
    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool;
    fn close(&self);
    fn reset(&self);
}

/// Ported from original/transport.h: `struct FdConnection`
pub struct FdConnection {
    read_file: Mutex<File>,
    write_file: Mutex<File>,
}

impl FdConnection {
    pub fn new(fd: OwnedFd) -> Self {
        let read_file = File::from(fd.try_clone().expect("Failed to clone fd"));
        let write_file = File::from(fd);
        Self {
            read_file: Mutex::new(read_file),
            write_file: Mutex::new(write_file),
        }
    }
}

impl BlockingConnection for FdConnection {
    fn read(&self) -> std::io::Result<Apacket> {
        let mut header_buf = [0u8; std::mem::size_of::<Amessage>()];
        let mut file = self.read_file.lock().unwrap();
        file.read_exact(&mut header_buf)?;

        // SAFETY: Amessage is repr(C) and we've read the correct number of bytes.
        let msg: Amessage = unsafe { std::ptr::read_unaligned(header_buf.as_ptr() as *const Amessage) };

        let mut payload = Block::new(msg.data_length as usize);
        if msg.data_length > 0 {
            file.read_exact(payload.get_mut())?;
        }

        Ok(Apacket { msg, payload })
    }

    fn write(&self, packet: &Apacket) -> std::io::Result<()> {
        let mut file = self.write_file.lock().unwrap();
        // SAFETY: Amessage is repr(C).
        let header_bytes: [u8; std::mem::size_of::<Amessage>()] = unsafe { std::mem::transmute(packet.msg) };
        file.write_all(&header_bytes)?;
        if !packet.payload.is_empty() {
            file.write_all(packet.payload.get_ref())?;
        }
        Ok(())
    }

    fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
        // TODO: Implement TLS handshake
        false
    }

    fn close(&self) {
        // Files are closed when dropped.
    }

    fn reset(&self) {
        self.close();
    }
}
