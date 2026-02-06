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
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use adb_protocol::{ConnectionState, TransportType, A_VERSION_MIN, MAX_PAYLOAD};
use adb_sockets::{Socket, Transport};
use adb_types::{calculate_apacket_checksum, Amessage, Apacket, Block};
use rust_adb_crypto::Key;
use sysdeps::AdbFd;

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

    pub fn start(self: &Arc<Self>) -> bool {
        let conn = self.connection.lock().unwrap().clone();
        if let Some(conn) = conn {
            conn.start(Arc::downgrade(self))
        } else {
            false
        }
    }

    pub fn write(&self, mut packet: Apacket) -> bool {
        packet.msg.magic = packet.msg.command ^ 0xffffffff;
        if self.get_protocol_version() >= adb_protocol::A_VERSION_SKIP_CHECKSUM as i32 {
            packet.msg.data_check = 0;
        } else {
            packet.msg.data_check = calculate_apacket_checksum(&packet);
        }

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

        handle_packet(packet, self);

        true
    }

    pub fn handle_error(&self, error: &str) {
        log::info!(
            "{}: connection terminated: {}",
            self.serial.lock().unwrap(),
            error
        );
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

pub fn transport_list() -> &'static Mutex<Vec<Arc<ATransport>>> {
    static LIST: OnceLock<Mutex<Vec<Arc<ATransport>>>> = OnceLock::new();
    LIST.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register_transport(transport: Arc<ATransport>) {
    transport_list().lock().unwrap().push(transport);
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

    result.ok_or_else(|| "device not found".to_string())
}

/// Parses a banner string and updates the transport's properties.
///
/// Ported from original/adb.cpp: `void parse_banner(const std::string& banner, atransport* t)`
pub fn parse_banner(banner: &str, t: &ATransport) {
    let pieces: Vec<&str> = banner.split(':').collect();

    t.set_features("");

    if pieces.len() > 2 {
        let props = pieces[2];
        for prop in props.split(';') {
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
            "bootloader" => ConnectionState::Bootloader,
            "device" => ConnectionState::Device,
            "recovery" => ConnectionState::Recovery,
            "sideload" => ConnectionState::Sideload,
            "rescue" => ConnectionState::Rescue,
            _ => ConnectionState::Host,
        };
        t.set_connection_state(state);
    }
}

pub fn send_connect(t: &Arc<ATransport>) {
    let mut p = Apacket::default();
    p.msg.command = adb_protocol::A_CNXN;
    p.msg.arg0 = adb_protocol::A_VERSION;
    p.msg.arg1 = adb_protocol::MAX_PAYLOAD as u32;
    let banner = format!(
        "host::features={}",
        feature_set_to_string(supported_features())
    );
    p.payload = Block::from_vec(banner.into_bytes());
    p.msg.data_length = p.payload.get_ref().len() as u32;
    t.write(p);
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
        _ => {}
    }
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
        result.push_str(&format!("{}\t{}\n", serial, state));
    } else {
        result.push_str(&format!("{:<22} {}", serial, state));

        let product = t.product.lock().unwrap();
        if !product.is_empty() {
            result.push_str(&format!(" product:{}", product));
        }
        let model = t.model.lock().unwrap();
        if !model.is_empty() {
            result.push_str(&format!(" model:{}", model));
        }
        let device = t.device.lock().unwrap();
        if !device.is_empty() {
            result.push_str(&format!(" device:{}", device));
        }

        result.push_str(&format!(" transport_id:{}\n", t.id));
    }
}

/// Lists all registered transports in the specified format.
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
        _ => "protobuf output not implemented".to_string(),
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
                    drop(keys);
                    handle_auth(t, p);
                }
            } else {
                send_auth_publickey(t);
            }
        }
        _ => {
            t.set_connection_state(ConnectionState::Offline);
        }
    }
}

fn send_auth_publickey(t: &Arc<ATransport>) {
    if let Some(android_dir) = adb_utils::adb_get_android_dir_path() {
        let pubkey_path = android_dir.join("adbkey.pub");
        if let Ok(pubkey) = std::fs::read_to_string(pubkey_path) {
            let mut p = Apacket::default();
            p.msg.command = adb_protocol::A_AUTH;
            p.msg.arg0 = adb_auth::ADB_AUTH_RSAPUBLICKEY;
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
}

fn handle_open(t: &Arc<ATransport>, p: &Apacket) {
    if !t.get_connection_state().is_online() || p.msg.arg0 == 0 {
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
                s.ack(Some(p.msg.arg1 as i32));
                t.send_ready(
                    s.id(),
                    p.msg.arg0,
                    adb_protocol::INITIAL_DELAYED_ACK_BYTES as u32,
                );
            } else {
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
        if let Some(registry) = t.registry.lock().unwrap().as_ref() {
            if let Some(s) = registry.find_local_socket(p.msg.arg1, p.msg.arg0) {
                s.close();
            }
        }
    }
}

fn handle_stls(t: &Arc<ATransport>, _p: &Apacket) {
    t.use_tls.store(true, Ordering::SeqCst);
}

fn handle_sync(t: &Arc<ATransport>, p: &Apacket) {
    if p.msg.arg0 == 0 {
        t.kick();
    }
}

fn handle_write(t: &Arc<ATransport>, p: &Apacket) {
    if t.get_connection_state().is_online() && p.msg.arg0 != 0 && p.msg.arg1 != 0 {
        if let Some(registry) = t.registry.lock().unwrap().as_ref() {
            if let Some(s) = registry.find_local_socket(p.msg.arg1, p.msg.arg0) {
                if s.enqueue(bytes::Bytes::copy_from_slice(p.payload.get_ref())) == 0 {
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
    if let Some(observers) = TRANSPORT_OBSERVERS.get() {
        for observer in observers.lock().unwrap().iter() {
            observer();
        }
    }
}

/// Ported from original/transport.h: `struct Connection`
pub trait Connection: Send + Sync {
    fn write(&self, packet: Apacket) -> bool;
    fn start(&self, transport: Weak<ATransport>) -> bool;
    fn stop(&self);
    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool;
    fn reset(&self);
}

/// Ported from original/transport.h: `struct BlockingConnection`
pub trait BlockingConnection: Send + Sync {
    fn read(&self) -> std::io::Result<Apacket>;
    fn write(&self, packet: &Apacket) -> std::io::Result<()>;
    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool;
    fn close(&self);
    fn reset(&self);
}

/// Ported from original/transport.h: `struct BlockingConnectionAdapter`
pub struct BlockingConnectionAdapter {
    underlying: Arc<dyn BlockingConnection>,
    transport: Mutex<Weak<ATransport>>,
    write_queue: Arc<Mutex<VecDeque<Apacket>>>,
    cv: Arc<std::sync::Condvar>,
    stopped: Arc<AtomicBool>,
}

impl BlockingConnectionAdapter {
    pub fn new(underlying: Arc<dyn BlockingConnection>) -> Self {
        Self {
            underlying,
            transport: Mutex::new(Weak::new()),
            write_queue: Arc::new(Mutex::new(VecDeque::new())),
            cv: Arc::new(std::sync::Condvar::new()),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Connection for BlockingConnectionAdapter {
    fn write(&self, packet: Apacket) -> bool {
        self.write_queue.lock().unwrap().push_back(packet);
        self.cv.notify_one();
        true
    }

    fn start(&self, transport: Weak<ATransport>) -> bool {
        *self.transport.lock().unwrap() = transport.clone();

        let t_read = transport.clone();
        let u_read = self.underlying.clone();
        let s_read = self.stopped.clone();
        std::thread::spawn(move || {
            while let Some(t) = t_read.upgrade() {
                if s_read.load(Ordering::SeqCst) {
                    break;
                }
                match u_read.read() {
                    Ok(p) => {
                        if !t.handle_read(p) {
                            break;
                        }
                    }
                    Err(e) => {
                        if !s_read.load(Ordering::SeqCst) {
                            t.handle_error(&e.to_string());
                        }
                        break;
                    }
                }
            }
        });

        let t_write = transport.clone();
        let u_write = self.underlying.clone();
        let wq = self.write_queue.clone();
        let cv = self.cv.clone();
        let s_write = self.stopped.clone();
        std::thread::spawn(move || {
            while let Some(t) = t_write.upgrade() {
                let mut q = wq.lock().unwrap();
                while !s_write.load(Ordering::SeqCst) && q.is_empty() {
                    q = cv.wait(q).unwrap();
                }

                if s_write.load(Ordering::SeqCst) {
                    break;
                }

                if let Some(p) = q.pop_front() {
                    drop(q);
                    if let Err(e) = u_write.write(&p) {
                        if !s_write.load(Ordering::SeqCst) {
                            t.handle_error(&e.to_string());
                        }
                        break;
                    }
                }
            }
        });

        true
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.cv.notify_all();
        self.underlying.close();
    }

    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool {
        self.underlying.do_tls_handshake(key, auth_key)
    }

    fn reset(&self) {
        self.underlying.reset();
    }
}

/// Ported from original/transport.h: `struct FdConnection`
pub struct FdConnection {
    read_fd: Mutex<AdbFd>,
    write_fd: Mutex<AdbFd>,
}

impl FdConnection {
    pub fn new(fd: AdbFd) -> Self {
        Self {
            read_fd: Mutex::new(fd.try_clone().unwrap()),
            write_fd: Mutex::new(fd),
        }
    }
}

impl Connection for FdConnection {
    fn write(&self, packet: Apacket) -> bool {
        BlockingConnection::write(self, &packet).is_ok()
    }

    fn start(&self, _transport: Weak<ATransport>) -> bool {
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
        let mut h = [0u8; std::mem::size_of::<Amessage>()];
        self.read_fd.lock().unwrap().read_exact(&mut h)?;

        let msg: Amessage = unsafe { std::ptr::read_unaligned(h.as_ptr() as *const Amessage) };

        let mut p = Block::new(msg.data_length as usize);
        if msg.data_length > 0 {
            self.read_fd.lock().unwrap().read_exact(p.get_mut())?;
        }

        Ok(Apacket { msg, payload: p })
    }

    fn write(&self, packet: &Apacket) -> std::io::Result<()> {
        let h: [u8; std::mem::size_of::<Amessage>()] = unsafe { std::mem::transmute(packet.msg) };
        let mut f = self.write_fd.lock().unwrap();
        f.write_all(&h)?;
        if !packet.payload.is_empty() {
            f.write_all(packet.payload.get_ref())?;
        }
        Ok(())
    }

    fn do_tls_handshake(&self, _k: &Key, _ak: Option<&mut String>) -> bool {
        false
    }

    fn close(&self) {
        self.read_fd.lock().unwrap().close();
        self.write_fd.lock().unwrap().close();
    }

    fn reset(&self) {
        self.close();
    }
}
