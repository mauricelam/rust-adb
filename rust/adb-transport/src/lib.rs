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

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::OwnedFd;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use adb_protocol::{ConnectionState, TransportType, A_VERSION_MIN, MAX_PAYLOAD};
use adb_sockets::{Socket, Transport};
use adb_types::{Amessage, Apacket, Block};
use rust_adb_crypto::Key;

/// Ported from original/adb.h: `using TransportId = uint64_t;`
pub type TransportId = u64;

static NEXT_TRANSPORT_ID: AtomicU64 = AtomicU64::new(1);

/// Ported from original/transport.cpp: `TransportId NextTransportId()`
pub fn next_transport_id() -> TransportId {
    NEXT_TRANSPORT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Ported from original/transport.h: `using FeatureSet = std::vector<std::string>;`
pub type FeatureSet = Vec<String>;

/// Ported from original/transport.h: `enum TrackerOutputType`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerOutputType {
    ShortText,
    LongText,
    Protobuf,
    TextProtobuf,
}

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

    disconnects: Mutex<Vec<(u64, Box<dyn DisconnectHandler>)>>,
    next_disconnect_id: AtomicU64,

    pub registry: Mutex<Option<Arc<adb_sockets::SocketRegistry>>>,
    pub fdevent: Mutex<Option<Arc<Mutex<fdevent::fdevent::Fdevent>>>>,
    pub service_creator: Mutex<Option<Box<dyn ServiceSocketCreator>>>,

    pub auth_keys: Mutex<VecDeque<rust_adb_crypto::Key>>,
    pub use_tls: AtomicBool,
    pub failed_auth_attempts: AtomicUsize,
    pub auth_key: Mutex<String>,
}

pub trait ServiceSocketCreator: Send + Sync {
    fn create_local_service_socket(
        &self,
        name: &str,
        transport: &Arc<ATransport>,
    ) -> Option<Arc<dyn adb_sockets::Socket>>;
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
            next_disconnect_id: AtomicU64::new(1),
            registry: Mutex::new(None),
            fdevent: Mutex::new(None),
            service_creator: Mutex::new(None),
            auth_keys: Mutex::new(VecDeque::new()),
            use_tls: AtomicBool::new(false),
            failed_auth_attempts: AtomicUsize::new(0),
            auth_key: Mutex::new(String::new()),
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

            if let Some(registry) = self.registry.lock().unwrap().as_ref() {
                registry.close_all_sockets(self.id);
            }
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
    pub fn handle_read(self: &Arc<Self>, packet: Apacket) -> bool {
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

    pub fn add_disconnect(&self, handler: Box<dyn DisconnectHandler>) -> u64 {
        let mut disconnects = self.disconnects.lock().unwrap();
        let id = self.next_disconnect_id.fetch_add(1, Ordering::SeqCst);
        disconnects.push((id, handler));
        id
    }

    pub fn remove_disconnect(&self, id: u64) {
        let mut disconnects = self.disconnects.lock().unwrap();
        disconnects.retain(|(hid, _)| *hid != id);
    }

    pub fn run_disconnects(&self) {
        let mut handlers = self.disconnects.lock().unwrap();
        for (_, handler) in handlers.drain(..) {
            handler.on_disconnect(self);
        }
    }

    pub fn get_protocol_version(&self) -> i32 {
        self.protocol_version.load(Ordering::SeqCst)
    }

    pub fn get_max_payload(&self) -> usize {
        *self.max_payload.lock().unwrap()
    }

    /// Checks if this transport matches the given target string.
    ///
    /// Ported from original/transport.cpp: `bool atransport::MatchesTarget(const std::string& target) const`
    pub fn matches_target(&self, target: &str) -> bool {
        let serial = self.serial.lock().unwrap();
        if !serial.is_empty() {
            if target == *serial {
                return true;
            }

            if self.transport_type == TransportType::Local {
                // Local transports can match [tcp:|udp:]<hostname>[:port].
                let local_target = target
                    .strip_prefix("tcp:")
                    .or_else(|| target.strip_prefix("udp:"))
                    .unwrap_or(target);

                // Simple address matching: check if serial and target match without port if necessary.
                // In C++ this uses ParseNetAddress. Here we do a basic split for now.
                if let Some((serial_host, serial_port)) = serial.rsplit_once(':') {
                    let (target_host, target_port) = match local_target.rsplit_once(':') {
                        Some((h, p)) => (h, p),
                        None => (local_target, serial_port),
                    };

                    if serial_host == target_host && serial_port == target_port {
                        return true;
                    }
                }
            }
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

    pub fn update_version(&self, version: u32, max_payload: u32) {
        let version = std::cmp::min(version, adb_protocol::A_VERSION);
        self.protocol_version
            .store(version as i32, Ordering::SeqCst);
        let max_payload = std::cmp::min(max_payload as usize, adb_protocol::MAX_PAYLOAD);
        *self.max_payload.lock().unwrap() = max_payload;
    }
}

impl adb_sockets::Transport for ATransport {
    fn id(&self) -> u64 {
        self.id
    }

    fn send_packet(&self, packet: Apacket) {
        self.write(packet);
    }

    fn send_ready(&self, local: u32, remote: u32, ack_bytes: u32) {
        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_OKAY;
        p.msg.arg0 = local;
        p.msg.arg1 = remote;

        if self.has_feature(FEATURE_DELAYED_ACK) && ack_bytes > 0 {
            p.msg.data_length = 4;
            p.payload = Block::from_vec(ack_bytes.to_le_bytes().to_vec());
        }

        self.send_packet(p);
    }

    fn get_max_payload(&self) -> usize {
        self.get_max_payload()
    }

    fn supports_delayed_ack(&self) -> bool {
        self.has_feature(FEATURE_DELAYED_ACK)
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

pub fn handle_packet(packet: Apacket, t: &Arc<ATransport>) {
    match packet.msg.command {
        adb_protocol::A_CNXN => {
            handle_new_connection(t, &packet);
        }
        adb_protocol::A_AUTH => {
            handle_auth(t, &packet);
        }
        adb_protocol::A_OPEN => {
            handle_open(t, &packet);
        }
        adb_protocol::A_OKAY => {
            handle_okay(t, &packet);
        }
        adb_protocol::A_CLSE => {
            handle_close(t, &packet);
        }
        adb_protocol::A_WRTE => {
            handle_write(t, &packet);
        }
        adb_protocol::A_STLS => {
            handle_stls(t, &packet);
        }
        adb_protocol::A_SYNC => {
            handle_sync(t, &packet);
        }
        _ => {
            log::warn!(target: "transport", "Unknown command: {:08x}", packet.msg.command);
        }
    }
}

fn sanitize(s: &str, alphanumeric: bool) -> String {
    s.chars()
        .map(|c| {
            if alphanumeric {
                if c.is_alphanumeric() {
                    c
                } else {
                    '_'
                }
            } else {
                if c == '\n' {
                    '_'
                } else {
                    c
                }
            }
        })
        .collect()
}

fn append_transport_info(result: &mut String, key: &str, value: &str, alphanumeric: bool) {
    if value.is_empty() {
        return;
    }

    result.push(' ');
    result.push_str(key);
    result.push_str(&sanitize(value, alphanumeric));
}

fn append_transport(t: &ATransport, result: &mut String, long_listing: bool) {
    let serial = t.serial.lock().unwrap();
    let serial = if serial.is_empty() {
        "(no serial number)"
    } else {
        &serial
    };

    let state = t.get_connection_state().to_string();

    if !long_listing {
        result.push_str(serial);
        result.push('\t');
        result.push_str(&state);
    } else {
        result.push_str(&format!("{:<22} {}", serial, state));

        append_transport_info(result, "", &t.devpath.lock().unwrap(), false);
        append_transport_info(result, "product:", &t.product.lock().unwrap(), false);
        append_transport_info(result, "model:", &t.model.lock().unwrap(), true);
        append_transport_info(result, "device:", &t.device.lock().unwrap(), false);

        result.push_str(" transport_id:");
        result.push_str(&t.id.to_string());
    }
    result.push('\n');
}

/// Lists all registered transports in the specified format.
/// Ported from original/transport.cpp: `std::string list_transports(TrackerOutputType outputType)`
pub fn list_transports(output_type: TrackerOutputType) -> String {
    let mut list = transport_list().lock().unwrap().clone();
    list.sort_by(|a, b| {
        if a.transport_type != b.transport_type {
            (a.transport_type as i32).cmp(&(b.transport_type as i32))
        } else {
            a.serial.lock().unwrap().cmp(&b.serial.lock().unwrap())
        }
    });

    match output_type {
        TrackerOutputType::ShortText | TrackerOutputType::LongText => {
            let long_listing = output_type == TrackerOutputType::LongText;
            let mut result = String::new();
            for t in list {
                append_transport(&t, &mut result, long_listing);
            }
            result
        }
        TrackerOutputType::Protobuf | TrackerOutputType::TextProtobuf => {
            // TODO: Implement protobuf output if needed.
            "protobuf output not implemented".to_string()
        }
    }
}

fn handle_new_connection(t: &Arc<ATransport>, p: &Apacket) {
    t.set_connection_state(ConnectionState::Offline);
    t.update_version(p.msg.arg0, p.msg.arg1);
    let banner = String::from_utf8_lossy(p.payload.get_ref());
    parse_banner(&banner, t);
}

fn handle_auth(t: &Arc<ATransport>, p: &Apacket) {
    if t.use_tls.load(Ordering::SeqCst) {
        // All AUTH commands are ignored in TLS mode.
        return;
    }

    match p.msg.arg0 {
        adb_auth::ADB_AUTH_TOKEN => {
            if t.get_connection_state() != ConnectionState::Authorizing {
                t.set_connection_state(ConnectionState::Authorizing);
            }

            let mut keys = t.auth_keys.lock().unwrap();
            if let Some(key) = keys.pop_front() {
                if let Ok(response) = adb_auth::send_auth_response(p.payload.get_ref(), &key) {
                    t.write(response);
                } else {
                    log::error!(target: "transport", "Failed to sign auth token");
                    // If signing failed, try the next key or send public key
                    drop(keys);
                    handle_auth(t, p);
                }
            } else {
                send_auth_publickey(t);
            }
        }
        adb_auth::ADB_AUTH_SIGNATURE => {
            // Daemon-side: verify signature
            let signature = p.payload.get_ref();
            // In a real adbd, we'd verify against the token we sent.
            // For now, we just log and accept it for testing purposes if desired,
            // or re-request if it fails.
            log::debug!(target: "transport", "Received AUTH SIGNATURE (len={})", signature.len());
            // TODO: Implement verification and call adbd_auth_verified(t)
        }
        adb_auth::ADB_AUTH_RSAPUBLICKEY => {
            // Daemon-side: handle public key
            let pubkey = String::from_utf8_lossy(p.payload.get_ref()).to_string();
            log::debug!(target: "transport", "Received AUTH RSAPUBLICKEY");
            *t.auth_key.lock().unwrap() = pubkey;
            // TODO: Trigger UI confirmation and then call adbd_auth_verified(t)
        }
        _ => {
            t.set_connection_state(ConnectionState::Offline);
        }
    }
}

fn send_auth_publickey(t: &Arc<ATransport>) {
    log::debug!(target: "transport", "Sending AUTH RSAPUBLICKEY");
    if let Some(android_dir) = adb_utils::adb_get_android_dir_path() {
        let pubkey_path = android_dir.join("adbkey.pub");
        if let Ok(pubkey) = std::fs::read_to_string(pubkey_path) {
            let mut p = Apacket::default();
            p.msg.command = adb_protocol::A_AUTH;
            p.msg.arg0 = adb_auth::ADB_AUTH_RSAPUBLICKEY;
            // The protocol expects the public key as a null-terminated string.
            let mut bytes = pubkey.into_bytes();
            if !bytes.ends_with(&[0]) {
                bytes.push(0);
            }
            p.payload = Block::from_vec(bytes);
            p.msg.data_length = p.payload.get_ref().len() as u32;
            t.write(p);
            return;
        }
    }
    log::error!(target: "transport", "Could not find adbkey.pub to send");
}

fn handle_open(t: &Arc<ATransport>, p: &Apacket) {
    if !t.get_connection_state().is_online() || p.msg.arg0 == 0 {
        return;
    }

    let send_bytes = p.msg.arg1;
    if t.has_feature(FEATURE_DELAYED_ACK) != (send_bytes != 0) {
        log::error!(target: "transport", "unexpected value of A_OPEN arg1: {} (delayed acks = {})",
            send_bytes, t.has_feature(FEATURE_DELAYED_ACK));
        send_close(0, p.msg.arg0, t);
        return;
    }

    let address = String::from_utf8_lossy(p.payload.get_ref());
    let address = address.trim_end_matches('\0');

    let service_creator = t.service_creator.lock().unwrap();
    let s = service_creator
        .as_ref()
        .and_then(|sc| sc.create_local_service_socket(address, t));

    if let Some(s) = s {
        let registry = t.registry.lock().unwrap();
        if let Some(registry) = registry.as_ref() {
            let peer = adb_sockets::create_remote_socket(
                p.msg.arg0,
                t.clone() as Arc<dyn Transport>,
                registry.clone(),
            );
            s.set_peer(peer.clone() as Arc<dyn adb_sockets::Socket>);
            peer.set_peer(s.clone() as Arc<dyn adb_sockets::Socket>);

            if t.has_feature(FEATURE_DELAYED_ACK) {
                s.ack(Some(send_bytes as i32));
                t.send_ready(
                    s.id(),
                    p.msg.arg0,
                    adb_protocol::INITIAL_DELAYED_ACK_BYTES as u32,
                );
            } else {
                // In C++, s->ready(s) for a local socket calls s->peer->ready(s->peer)
                // which sends A_OKAY.
                if let Some(peer_id) = s.peer_id() {
                    t.send_ready(s.id(), peer_id, 0);
                }
                s.ready();
            }
        }
    } else {
        send_close(0, p.msg.arg0, t);
    }
}

fn handle_okay(t: &Arc<ATransport>, p: &Apacket) {
    if t.get_connection_state().is_online() && p.msg.arg0 != 0 && p.msg.arg1 != 0 {
        let registry = t.registry.lock().unwrap();
        if let Some(registry) = registry.as_ref() {
            if let Some(s) = registry.find_local_socket(p.msg.arg1, 0) {
                let mut acked_bytes: Option<i32> = None;
                if p.payload.get_ref().len() == 4 {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(p.payload.get_ref());
                    acked_bytes = Some(i32::from_le_bytes(bytes));
                } else if !p.payload.is_empty() {
                    log::error!(target: "transport", "invalid A_OKAY payload size: {}", p.payload.get_ref().len());
                    return;
                }

                if s.peer_id().is_none() {
                    let peer = adb_sockets::create_remote_socket(
                        p.msg.arg0,
                        t.clone() as Arc<dyn Transport>,
                        registry.clone(),
                    );
                    s.set_peer(peer.clone() as Arc<dyn adb_sockets::Socket>);
                    peer.set_peer(s.clone() as Arc<dyn adb_sockets::Socket>);
                }

                s.ack(acked_bytes);
            } else {
                send_close(p.msg.arg1, p.msg.arg0, t);
            }
        }
    }
}

fn handle_close(t: &Arc<ATransport>, p: &Apacket) {
    if t.get_connection_state().is_online() && p.msg.arg1 != 0 {
        let registry = t.registry.lock().unwrap();
        if let Some(registry) = registry.as_ref() {
            if let Some(s) = registry.find_local_socket(p.msg.arg1, p.msg.arg0) {
                s.close();
            }
        }
    }
}

fn handle_stls(t: &Arc<ATransport>, _p: &Apacket) {
    t.use_tls.store(true, Ordering::SeqCst);
    log::info!(target: "transport", "TLS requested, but not yet implemented");
}

fn handle_sync(t: &Arc<ATransport>, p: &Apacket) {
    if p.msg.arg0 == 0 {
        t.kick();
    }
}

fn handle_write(t: &Arc<ATransport>, p: &Apacket) {
    if t.get_connection_state().is_online() && p.msg.arg0 != 0 && p.msg.arg1 != 0 {
        let registry = t.registry.lock().unwrap();
        if let Some(registry) = registry.as_ref() {
            if let Some(s) = registry.find_local_socket(p.msg.arg1, p.msg.arg0) {
                let data = bytes::Bytes::copy_from_slice(p.payload.get_ref());
                if s.enqueue(data) == 0 {
                    t.send_ready(s.id(), p.msg.arg0, 0);
                }
            }
        }
    }
}

fn send_close(local: u32, remote: u32, t: &Arc<ATransport>) {
    let mut p = Apacket::default();
    p.msg.command = adb_protocol::A_CLSE;
    p.msg.arg0 = local;
    p.msg.arg1 = remote;
    t.write(p);
}

type TransportObserver = Box<dyn Fn() + Send + Sync>;
static TRANSPORT_OBSERVERS: OnceLock<Mutex<Vec<TransportObserver>>> = OnceLock::new();

pub fn register_transport_observer(observer: TransportObserver) {
    TRANSPORT_OBSERVERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(observer);
}

pub fn update_transports() {
    log::debug!(target: "transport", "update_transports");
    if let Some(observers) = TRANSPORT_OBSERVERS.get() {
        let observers = observers.lock().unwrap();
        for observer in observers.iter() {
            observer();
        }
    }
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
    fn test_connection_state_to_string() {
        assert_eq!(ConnectionState::Offline.to_string(), "offline");
        assert_eq!(ConnectionState::Bootloader.to_string(), "bootloader");
        assert_eq!(ConnectionState::Device.to_string(), "device");
        assert_eq!(ConnectionState::Host.to_string(), "host");
        assert_eq!(ConnectionState::Recovery.to_string(), "recovery");
        assert_eq!(ConnectionState::Rescue.to_string(), "rescue");
        assert_eq!(ConnectionState::Sideload.to_string(), "sideload");
        assert_eq!(ConnectionState::Unauthorized.to_string(), "unauthorized");
        assert_eq!(ConnectionState::Authorizing.to_string(), "authorizing");
        assert_eq!(ConnectionState::Connecting.to_string(), "connecting");
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

        let _id = t.add_disconnect(Box::new(Wrapper(counter.clone())));
        t.run_disconnects();
        assert_eq!(*counter.count.lock().unwrap(), 1);

        // Disconnects should have been removed.
        t.run_disconnects();
        assert_eq!(*counter.count.lock().unwrap(), 1);

        // Test remove_disconnect
        let id = t.add_disconnect(Box::new(Wrapper(counter.clone())));
        t.remove_disconnect(id);
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
        let banner = "host::ro.product.name=foo;ro.product.model=bar;ro.product.device=baz;features=woodly,doodly";
        parse_banner(banner, &t);

        assert_eq!(t.get_connection_state(), ConnectionState::Host);
        assert_eq!(*t.product.lock().unwrap(), "foo");
        assert_eq!(*t.model.lock().unwrap(), "bar");
        assert_eq!(*t.device.lock().unwrap(), "baz");
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

    #[test]
    fn test_matches_target_local() {
        let t = ATransport::new_offline(TransportType::Local);
        *t.serial.lock().unwrap() = "100.100.100.100:5555".to_string();

        assert!(t.matches_target("100.100.100.100"));
        assert!(t.matches_target("100.100.100.100:5555"));
        assert!(t.matches_target("tcp:100.100.100.100"));
        assert!(t.matches_target("tcp:100.100.100.100:5555"));
        assert!(t.matches_target("udp:100.100.100.100"));
        assert!(t.matches_target("udp:100.100.100.100:5555"));

        assert!(!t.matches_target("100.100.100.100:5554"));
        assert!(!t.matches_target("100.100.100.101"));
    }

    struct MockConnection {
        stopped: AtomicBool,
        written: Mutex<Vec<Apacket>>,
    }

    impl MockConnection {
        fn new() -> Self {
            Self {
                stopped: AtomicBool::new(false),
                written: Mutex::new(Vec::new()),
            }
        }
    }

    impl Connection for MockConnection {
        fn write(&self, packet: Apacket) -> bool {
            self.written.lock().unwrap().push(packet);
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

    #[test]
    fn test_handle_cnxn() {
        let t = Arc::new(ATransport::new_offline(TransportType::Usb));
        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_CNXN;
        p.msg.arg0 = adb_protocol::A_VERSION;
        p.msg.arg1 = 1024 * 1024;
        p.payload = Block::from_vec(b"device::ro.product.name=foo;features=shell_v2".to_vec());
        p.msg.data_length = p.payload.get_ref().len() as u32;

        handle_packet(p, &t);

        assert_eq!(t.get_connection_state(), ConnectionState::Device);
        assert_eq!(t.get_protocol_version(), adb_protocol::A_VERSION as i32);
        assert_eq!(t.get_max_payload(), 1024 * 1024);
        assert!(t.has_feature("shell_v2"));
    }

    struct MockSocket {
        id: u32,
        peer_id: Mutex<Option<u32>>,
        enqueued: Mutex<Vec<bytes::Bytes>>,
        readied: AtomicBool,
        closed: AtomicBool,
    }

    impl MockSocket {
        fn new(id: u32) -> Self {
            Self {
                id,
                peer_id: Mutex::new(None),
                enqueued: Mutex::new(Vec::new()),
                readied: AtomicBool::new(false),
                closed: AtomicBool::new(false),
            }
        }
    }

    impl Socket for MockSocket {
        fn id(&self) -> u32 {
            self.id
        }
        fn enqueue(&self, data: bytes::Bytes) -> i32 {
            self.enqueued.lock().unwrap().push(data);
            0
        }
        fn ready(&self) {
            self.readied.store(true, Ordering::SeqCst);
        }
        fn shutdown(&self) {}
        fn close(&self) {
            self.closed.store(true, Ordering::SeqCst);
        }
        fn peer_id(&self) -> Option<u32> {
            *self.peer_id.lock().unwrap()
        }
        fn transport_id(&self) -> Option<u64> {
            None
        }
        fn set_peer(&self, peer: Arc<dyn Socket>) {
            *self.peer_id.lock().unwrap() = Some(peer.id());
        }
        fn ack(&self, _acked_bytes: Option<i32>) {
            self.ready();
        }
    }

    struct MockServiceCreator {
        socket: Arc<MockSocket>,
    }

    impl ServiceSocketCreator for MockServiceCreator {
        fn create_local_service_socket(
            &self,
            _name: &str,
            transport: &Arc<ATransport>,
        ) -> Option<Arc<dyn Socket>> {
            let registry = transport.registry.lock().unwrap();
            if let Some(registry) = registry.as_ref() {
                registry.install(self.socket.clone());
            }
            Some(self.socket.clone())
        }
    }

    #[test]
    fn test_handle_open() {
        let t = Arc::new(ATransport::new(
            TransportType::Usb,
            Box::new(|_| ReconnectResult::Abort),
            ConnectionState::Device,
        ));
        let registry = Arc::new(adb_sockets::SocketRegistry::new());
        *t.registry.lock().unwrap() = Some(registry.clone());
        let mock_socket = Arc::new(MockSocket::new(100));
        *t.service_creator.lock().unwrap() = Some(Box::new(MockServiceCreator {
            socket: mock_socket.clone(),
        }));

        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_OPEN;
        p.msg.arg0 = 200; // remote id
        p.payload = Block::from_vec(b"shell:echo hello".to_vec());
        p.msg.data_length = p.payload.get_ref().len() as u32;

        handle_packet(p, &t);

        assert_eq!(mock_socket.peer_id(), Some(200));
        assert!(mock_socket.readied.load(Ordering::SeqCst));
        assert!(registry.find(100).is_some());
    }

    #[test]
    fn test_handle_write() {
        let t = Arc::new(ATransport::new(
            TransportType::Usb,
            Box::new(|_| ReconnectResult::Abort),
            ConnectionState::Device,
        ));
        let registry = Arc::new(adb_sockets::SocketRegistry::new());
        *t.registry.lock().unwrap() = Some(registry.clone());
        let mock_socket = Arc::new(MockSocket::new(100));
        *mock_socket.peer_id.lock().unwrap() = Some(200);
        registry.install(mock_socket.clone());

        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_WRTE;
        p.msg.arg0 = 200; // remote id
        p.msg.arg1 = 100; // local id
        p.payload = Block::from_vec(b"hello".to_vec());
        p.msg.data_length = p.payload.get_ref().len() as u32;

        handle_packet(p, &t);

        assert_eq!(mock_socket.enqueued.lock().unwrap().len(), 1);
        assert_eq!(mock_socket.enqueued.lock().unwrap()[0], &b"hello"[..]);
    }

    #[test]
    fn test_handle_okay_delayed_ack() {
        let t = Arc::new(ATransport::new(
            TransportType::Usb,
            Box::new(|_| ReconnectResult::Abort),
            ConnectionState::Device,
        ));
        let registry = Arc::new(adb_sockets::SocketRegistry::new());
        *t.registry.lock().unwrap() = Some(registry.clone());
        let mock_socket = Arc::new(MockSocket::new(100));
        *mock_socket.peer_id.lock().unwrap() = Some(200);
        registry.install(mock_socket.clone());

        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_OKAY;
        p.msg.arg0 = 200; // remote id
        p.msg.arg1 = 100; // local id
        p.payload = Block::from_vec(1024u32.to_le_bytes().to_vec());
        p.msg.data_length = 4;

        handle_packet(p, &t);

        assert!(mock_socket.readied.load(Ordering::SeqCst));
    }

    #[test]
    fn test_handle_sync() {
        let t = Arc::new(ATransport::new_offline(TransportType::Usb));
        let conn = Arc::new(MockConnection::new());
        t.set_connection(conn.clone());

        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_SYNC;
        p.msg.arg0 = 0;

        handle_packet(p, &t);

        assert!(t.is_kicked());
        assert!(conn.stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn test_handle_open_delayed_ack() {
        let t = Arc::new(ATransport::new(
            TransportType::Usb,
            Box::new(|_| ReconnectResult::Abort),
            ConnectionState::Device,
        ));
        t.set_features(FEATURE_DELAYED_ACK);
        let registry = Arc::new(adb_sockets::SocketRegistry::new());
        *t.registry.lock().unwrap() = Some(registry.clone());
        let mock_socket = Arc::new(MockSocket::new(100));
        *t.service_creator.lock().unwrap() = Some(Box::new(MockServiceCreator {
            socket: mock_socket.clone(),
        }));
        let conn = Arc::new(MockConnection::new());
        t.set_connection(conn.clone());

        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_OPEN;
        p.msg.arg0 = 200; // remote id
        p.msg.arg1 = 1024; // send_bytes
        p.payload = Block::from_vec(b"shell:echo hello".to_vec());
        p.msg.data_length = p.payload.get_ref().len() as u32;

        handle_packet(p, &t);

        // Should have sent A_OKAY with INITIAL_DELAYED_ACK_BYTES
        let written = conn.written.lock().unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].msg.command, adb_protocol::A_OKAY);
        assert_eq!(written[0].msg.arg0, 100);
        assert_eq!(written[0].msg.arg1, 200);
        assert_eq!(written[0].msg.data_length, 4);
        let ack_bytes = u32::from_le_bytes(written[0].payload.get_ref()[..4].try_into().unwrap());
        assert_eq!(ack_bytes, adb_protocol::INITIAL_DELAYED_ACK_BYTES as u32);
    }
}

/// Ported from original/transport.h: `struct Connection`
pub trait Connection: Send + Sync {
    fn write(&self, packet: Apacket) -> bool;
    fn start(&self) -> bool;
    fn stop(&self);
    /// Performs the TLS handshake for secure ADB connections.
    ///
    /// Note: Implementation is pending the porting of the `adb::tls::TlsConnection` library.
    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool;
    fn reset(&self);
    fn supports_detach(&self) -> bool {
        false
    }
    fn attach(&self) -> Result<(), String> {
        Err("transport type doesn't support attach".to_string())
    }
    fn detach(&self) -> Result<(), String> {
        Err("transport type doesn't support detach".to_string())
    }
    fn negotiated_speed_mbps(&self) -> u64 {
        0
    }
    fn max_speed_mbps(&self) -> u64 {
        0
    }
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

impl Connection for FdConnection {
    fn write(&self, packet: Apacket) -> bool {
        BlockingConnection::write(self, &packet).is_ok()
    }

    fn start(&self) -> bool {
        true
    }

    fn stop(&self) {
        self.close();
    }

    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool {
        BlockingConnection::do_tls_handshake(self, key, auth_key)
    }

    fn reset(&self) {
        BlockingConnection::reset(self);
    }
}

impl BlockingConnection for FdConnection {
    fn read(&self) -> std::io::Result<Apacket> {
        let mut header_buf = [0u8; std::mem::size_of::<Amessage>()];
        let mut file = self.read_file.lock().unwrap();
        file.read_exact(&mut header_buf)?;

        // SAFETY: Amessage is repr(C) and we've read the correct number of bytes.
        let msg: Amessage =
            unsafe { std::ptr::read_unaligned(header_buf.as_ptr() as *const Amessage) };

        let mut payload = Block::new(msg.data_length as usize);
        if msg.data_length > 0 {
            file.read_exact(payload.get_mut())?;
        }

        Ok(Apacket { msg, payload })
    }

    fn write(&self, packet: &Apacket) -> std::io::Result<()> {
        let mut file = self.write_file.lock().unwrap();
        // SAFETY: Amessage is repr(C).
        let header_bytes: [u8; std::mem::size_of::<Amessage>()] =
            unsafe { std::mem::transmute(packet.msg) };
        file.write_all(&header_bytes)?;
        if !packet.payload.is_empty() {
            file.write_all(packet.payload.get_ref())?;
        }
        Ok(())
    }

    fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
        log::warn!(target: "transport", "TLS handshake not yet implemented");
        false
    }

    fn close(&self) {
        // Files are closed when dropped.
    }

    fn reset(&self) {
        self.close();
    }
}
