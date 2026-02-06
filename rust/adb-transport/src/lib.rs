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
use std::net::TcpStream;
use std::os::unix::io::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::SystemTime;

use adb_protocol::{ConnectionState, TransportType, A_VERSION_MIN, MAX_PAYLOAD};
use adb_sockets::{Socket, Transport};
use adb_types::{calculate_apacket_checksum, Amessage, Apacket, Block};
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

/// Callback for when a new public key needs authorization.
pub type AuthPromptCallback = Arc<dyn Fn(&Arc<ATransport>, &str) + Send + Sync>;

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

    pub use_tls: AtomicBool,
    pub tls_version: AtomicI32,
    pub keys: Mutex<VecDeque<Arc<Key>>>,

    pub registry: Mutex<Option<Arc<adb_sockets::SocketRegistry>>>,
    pub fdevent: Mutex<Option<Arc<Mutex<fdevent::fdevent::Fdevent>>>>,
    pub service_creator: Mutex<Option<Box<dyn ServiceSocketCreator>>>,

    pub auth_keys: Mutex<VecDeque<rust_adb_crypto::Key>>,
    pub failed_auth_attempts: AtomicUsize,
    pub auth_key: Mutex<String>,
    pub auth_token: Mutex<[u8; adb_auth::TOKEN_SIZE]>,
    pub auth_prompt: Mutex<Option<AuthPromptCallback>>,
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
            use_tls: AtomicBool::new(false),
            tls_version: AtomicI32::new(adb_protocol::A_STLS_VERSION as i32),
            keys: Mutex::new(VecDeque::new()),
            registry: Mutex::new(None),
            fdevent: Mutex::new(None),
            service_creator: Mutex::new(None),
            auth_keys: Mutex::new(VecDeque::new()),
            failed_auth_attempts: AtomicUsize::new(0),
            auth_key: Mutex::new(String::new()),
            auth_token: Mutex::new([0u8; adb_auth::TOKEN_SIZE]),
            auth_prompt: Mutex::new(None),
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

    pub fn set_connection(self: &Arc<Self>, connection: Arc<dyn Connection>) {
        connection.set_transport(Arc::downgrade(self));
        let mut conn_lock = self.connection.lock().unwrap();
        *conn_lock = Some(connection);
    }

    pub fn get_key(&self) -> Option<Arc<Key>> {
        self.keys.lock().unwrap().front().cloned()
    }

    /// Ported from original/transport.cpp: `bool atransport::HandleRead(std::unique_ptr<apacket> p)`
    pub fn handle_read(self: &Arc<Self>, packet: Apacket) -> bool {
        println!("Handle read: cmd={:?}", packet.msg.command);
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
        let handlers: Vec<(u64, Box<dyn DisconnectHandler>)> =
            self.disconnects.lock().unwrap().drain(..).collect();
        for (_, handler) in handlers {
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
        TrackerOutputType::Protobuf | TrackerOutputType::TextProtobuf => {
            // TODO: Implement protobuf output if needed.
            "protobuf output not implemented".to_string()
        }
    }
}

fn handle_stls(t: &Arc<ATransport>, _p: &Apacket) {
    if t.use_tls.swap(true, Ordering::SeqCst) {
        // Already in TLS mode, nothing to do.
        return;
    }

    // In a real host implementation, we'd send the request if we haven't already.
    send_tls_request(t);

    let t_clone = t.clone();
    std::thread::spawn(move || {
        let conn = t_clone.connection.lock().unwrap().as_ref().cloned();
        if let Some(conn) = conn {
            let key = t_clone.get_key().unwrap_or_else(|| {
                Arc::new(rust_adb_crypto::new_rsa_2048().expect("failed to generate temp key"))
            });
            if !conn.do_tls_handshake(&key, None) {
                log::error!(target: "transport", "TLS handshake failed");
                t_clone.kick();
            }
        }
    });
}

fn send_tls_request(t: &Arc<ATransport>) {
    let mut p = Apacket::default();
    p.msg.command = adb_protocol::A_STLS;
    p.msg.arg0 = adb_protocol::A_STLS_VERSION;
    p.msg.update_magic();
    t.write(p);
}

fn handle_new_connection(t: &Arc<ATransport>, p: &Apacket) {
    t.set_connection_state(ConnectionState::Offline);
    t.update_version(p.msg.arg0, p.msg.arg1);
    let banner = String::from_utf8_lossy(p.payload.get_ref());
    parse_banner(&banner, t);

    if t.get_connection_state() == ConnectionState::Host && !t.use_tls.load(Ordering::SeqCst) {
        // We are the daemon, and the remote is a host.
        // For now, let's assume auth is always required.
        adb_auth::ensure_authorized_keys_loaded();
        t.set_connection_state(ConnectionState::Authorizing);
        let (auth_packet, token) = adb_auth::send_auth_request();
        *t.auth_token.lock().unwrap() = token;
        t.write(auth_packet);
    }
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
        adb_auth::ADB_AUTH_SIGNATURE => {
            let token = t.auth_token.lock().unwrap();
            if adb_auth::adbd_auth_verify_all(&*token, p.payload.get_ref()) {
                t.set_connection_state(ConnectionState::Device);
                send_connect(t);
            } else {
                t.set_connection_state(ConnectionState::Unauthorized);
            }
        }
        adb_auth::ADB_AUTH_RSAPUBLICKEY => {
            let public_key = String::from_utf8_lossy(p.payload.get_ref());
            let public_key = public_key.trim_end_matches('\0');
            log::info!("Received new public key: {}", public_key);

            let prompt = t.auth_prompt.lock().unwrap().clone();
            if let Some(prompt) = prompt {
                prompt(t, public_key);
            } else {
                // If no prompt callback, just stay in Unauthorized state.
                t.set_connection_state(ConnectionState::Unauthorized);
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
        for transport_type in [TransportType::Any, TransportType::Local, TransportType::Usb] {
            let t = ATransport::new_offline(transport_type);
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
    }

    #[test]
    fn test_matches_target_local() {
        for transport_type in [TransportType::Any, TransportType::Local] {
            let t = ATransport::new_offline(transport_type);
            *t.serial.lock().unwrap() = "100.100.100.100:5555".to_string();

            let should_match = transport_type == TransportType::Local;
            assert_eq!(should_match, t.matches_target("100.100.100.100"));
            assert_eq!(should_match, t.matches_target("tcp:100.100.100.100"));
            assert_eq!(should_match, t.matches_target("tcp:100.100.100.100:5555"));
            assert_eq!(should_match, t.matches_target("udp:100.100.100.100"));
            assert_eq!(should_match, t.matches_target("udp:100.100.100.100:5555"));

            assert!(!t.matches_target("100.100.100.100:5554"));
            assert!(!t.matches_target("100.100.100.101"));
        }
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
        fn set_transport(&self, _transport: Weak<ATransport>) {}
        fn write(&self, packet: Apacket) -> bool {
            self.written.lock().unwrap().push(packet);
            true
        }
        fn start(&self, _transport: Weak<ATransport>) -> bool {
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
    fn test_blocking_connection_adapter() {
        struct MockBlockingConnection {
            read_packets: Mutex<VecDeque<Apacket>>,
            written_packets: Arc<Mutex<Vec<Apacket>>>,
            cv: Arc<std::sync::Condvar>,
        }

        impl BlockingConnection for MockBlockingConnection {
            fn read(&self) -> std::io::Result<Apacket> {
                let mut queue = self.read_packets.lock().unwrap();
                while queue.is_empty() {
                    queue = self.cv.wait(queue).unwrap();
                }
                Ok(queue.pop_front().unwrap())
            }
            fn write(&self, packet: &Apacket) -> std::io::Result<()> {
                self.written_packets.lock().unwrap().push(packet.clone());
                Ok(())
            }
            fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
                true
            }
            fn close(&self) {}
            fn reset(&self) {}
        }

        let written = Arc::new(Mutex::new(Vec::new()));
        let cv = Arc::new(std::sync::Condvar::new());
        let mock = Arc::new(MockBlockingConnection {
            read_packets: Mutex::new(VecDeque::new()),
            written_packets: written.clone(),
            cv: cv.clone(),
        });

        let adapter = BlockingConnectionAdapter::new(mock.clone());
        let transport = Arc::new(ATransport::new_offline(TransportType::Usb));
        transport.set_connection_state(ConnectionState::Device);
        let mock_socket = Arc::new(MockSocket::new(100));
        let registry = Arc::new(adb_sockets::SocketRegistry::new());
        *transport.registry.lock().unwrap() = Some(registry.clone());
        registry.install(mock_socket.clone());

        adapter.start(Arc::downgrade(&transport));

        // Test write
        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_WRTE;
        p.msg.arg0 = 200;
        p.msg.arg1 = 100;
        adapter.write(p);

        // Give it a moment to process the write
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(written.lock().unwrap().len(), 1);
        assert_eq!(written.lock().unwrap()[0].msg.command, adb_protocol::A_WRTE);

        // Test read
        let mut p2 = Apacket::default();
        p2.msg.command = adb_protocol::A_OKAY;
        p2.msg.arg0 = 200;
        p2.msg.arg1 = 100;
        p2.msg.update_magic();
        {
            let mut queue = mock.read_packets.lock().unwrap();
            queue.push_back(p2);
            cv.notify_one();
        }

        // Give it a moment to process the read
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(mock_socket.readied.load(Ordering::SeqCst));

        adapter.stop();
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

    #[test]
    fn test_tls_handshake() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
        let addr = listener.local_addr().expect("Failed to get address");

        let server_key = rust_adb_crypto::new_rsa_2048().expect("Failed to generate server key");
        let server_cert = rust_adb_crypto::generate_x509_certificate(&server_key)
            .expect("Failed to generate server cert");

        let server_thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("Failed to accept");

            // 1. Read STLS packet
            let mut buf = [0u8; 24];
            stream.read_exact(&mut buf).expect("Failed to read STLS");

            // 2. Respond with STLS packet
            stream.write_all(&buf).expect("Failed to write STLS");

            // 3. Perform TLS handshake as server
            let key = server_key;
            let cert = server_cert;
            let cert_pem = rust_adb_crypto::x509_to_pem_string(&cert).expect("Failed to PEM cert");
            let key_pem = key.to_pem_string().expect("Failed to PEM key");

            let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
                .expect("Failed to parse certs")
                .into_iter()
                .map(rustls::Certificate)
                .collect::<Vec<_>>();
            let priv_key = rustls_pemfile::pkcs8_private_keys(&mut key_pem.as_bytes())
                .expect("Failed to parse keys")
                .into_iter()
                .map(rustls::PrivateKey)
                .next()
                .expect("No private key");

            let config = rustls::ServerConfig::builder()
                .with_safe_defaults()
                .with_client_cert_verifier(Arc::new(AdbClientCertVerifier))
                .with_single_cert(certs, priv_key)
                .expect("Failed to build server config");

            let mut conn = rustls::ServerConnection::new(Arc::new(config))
                .expect("Failed to create server connection");
            while conn.is_handshaking() {
                while conn.wants_write() {
                    conn.write_tls(&mut stream)
                        .expect("Server write TLS failed");
                }
                if conn.is_handshaking() && conn.wants_read() {
                    if conn.read_tls(&mut stream).expect("Server read TLS failed") == 0 {
                        break;
                    }
                    if let Err(_) = conn.process_new_packets() {
                        break;
                    }
                }
            }

            // 4. Send some encrypted data (Apacket)
            let mut p = Apacket::default();
            p.msg.command = adb_protocol::A_CNXN;
            p.payload = Block::from_vec(b"secure hello".to_vec());
            p.msg.data_length = p.payload.get_ref().len() as u32;

            let header_bytes: [u8; 24] = unsafe { std::mem::transmute(p.msg) };
            conn.writer()
                .write_all(&header_bytes)
                .expect("Server write header failed");
            conn.writer()
                .write_all(p.payload.get_ref())
                .expect("Server write payload failed");

            while conn.wants_write() {
                conn.write_tls(&mut stream)
                    .expect("Server flush TLS failed");
            }
        });

        let stream = TcpStream::connect(addr).expect("Failed to connect");
        let fd = OwnedFd::from(stream);
        let fd_conn = Arc::new(FdConnection::new(fd));
        let adapter = Arc::new(BlockingConnectionAdapter::new(fd_conn.clone()));
        let transport = Arc::new(ATransport::new(
            TransportType::Local,
            Box::new(|_| ReconnectResult::Abort),
            ConnectionState::Connecting,
        ));
        transport.set_connection(adapter.clone());

        adapter.start(Arc::downgrade(&transport));

        // Initiate upgrade
        handle_stls(&transport, &Apacket::default());

        // Wait for handshake to complete with a timeout.
        let start = std::time::Instant::now();
        while fd_conn.tls.lock().unwrap().is_none() {
            if start.elapsed().as_secs() > 10 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Verify TLS is active
        let active = fd_conn.tls.lock().unwrap().is_some();
        assert!(active);

        // Try reading secure data. The background thread should pick it up.
        let start = std::time::Instant::now();
        while transport.get_connection_state() != ConnectionState::Host {
            if start.elapsed().as_secs() > 10 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        assert_eq!(transport.get_connection_state(), ConnectionState::Host);
        assert_eq!(*transport.product.lock().unwrap(), ""); // Default for A_CNXN with my payload

        adapter.stop();
        server_thread.join().expect("Server thread panicked");
    }

    #[test]
    fn test_daemon_auth_flow() {
        use base64::Engine;

        let dir = tempfile::tempdir().unwrap();
        let adb_keys_path = dir.path().join("adb_keys");
        std::env::set_var("ADB_VENDOR_KEYS", &adb_keys_path);
        adb_auth::load_authorized_keys().unwrap();

        let t = Arc::new(ATransport::new_offline(TransportType::Local));
        let conn = Arc::new(MockConnection::new());
        t.set_connection(conn.clone());

        // 1. Receive A_CNXN from host
        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_CNXN;
        p.payload = Block::from_vec(b"host::features=shell_v2".to_vec());
        p.msg.data_length = p.payload.get_ref().len() as u32;

        handle_packet(p, &t);

        // Should be Authorizing and have sent a TOKEN request
        assert_eq!(t.get_connection_state(), ConnectionState::Authorizing);
        {
            let written = conn.written.lock().unwrap();
            assert_eq!(written.len(), 1);
            assert_eq!(written[0].msg.command, adb_protocol::A_AUTH);
            assert_eq!(written[0].msg.arg0, adb_auth::ADB_AUTH_TOKEN);
        }

        let token = *t.auth_token.lock().unwrap();
        assert_ne!(token, [0u8; adb_auth::TOKEN_SIZE]);

        // 2. Receive A_AUTH (SIGNATURE)
        let key = rust_adb_crypto::new_rsa_2048().unwrap();
        let sig = adb_auth::adb_auth_sign(&key, &token).unwrap();

        // Add the public key to authorized keys
        let pubkey_struct = key.android_pubkey().unwrap();
        let pubkey_bytes: [u8; 524] = unsafe { std::mem::transmute(pubkey_struct) };
        let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(&pubkey_bytes);
        adb_auth::save_authorized_key(&pubkey_b64).unwrap();

        let mut p_sig = Apacket::default();
        p_sig.msg.command = adb_protocol::A_AUTH;
        p_sig.msg.arg0 = adb_auth::ADB_AUTH_SIGNATURE;
        p_sig.payload = Block::from_vec(sig);
        p_sig.msg.data_length = p_sig.payload.get_ref().len() as u32;

        handle_packet(p_sig, &t);

        // Should be Device (online) and have sent A_CNXN
        assert_eq!(t.get_connection_state(), ConnectionState::Device);
        {
            let written = conn.written.lock().unwrap();
            assert_eq!(written.len(), 2);
            assert_eq!(written[1].msg.command, adb_protocol::A_CNXN);
        }

        std::env::remove_var("ADB_VENDOR_KEYS");
    }

    #[test]
    fn test_daemon_auth_flow_with_prompt() {
        use base64::Engine;

        let dir = tempfile::tempdir().unwrap();
        let adb_keys_path = dir.path().join("adb_keys");
        std::env::set_var("ADB_VENDOR_KEYS", &adb_keys_path);

        let t = Arc::new(ATransport::new_offline(TransportType::Local));
        let conn = Arc::new(MockConnection::new());
        t.set_connection(conn.clone());

        // Set prompt callback
        let prompt_called = Arc::new(AtomicBool::new(false));
        let prompt_called_clone = prompt_called.clone();
        *t.auth_prompt.lock().unwrap() = Some(Arc::new(move |t_inner, key| {
            prompt_called_clone.store(true, Ordering::SeqCst);
            // Auto-authorize for the test
            adb_auth::save_authorized_key(key).unwrap();
            t_inner.set_connection_state(ConnectionState::Device);
            send_connect(t_inner);
        }));

        // 1. Receive A_CNXN from host
        let mut p = Apacket::default();
        p.msg.command = adb_protocol::A_CNXN;
        p.payload = Block::from_vec(b"host::features=shell_v2".to_vec());
        p.msg.data_length = p.payload.get_ref().len() as u32;

        handle_packet(p, &t);

        // 2. Receive A_AUTH (RSAPUBLICKEY)
        let key = rust_adb_crypto::new_rsa_2048().unwrap();
        let pubkey_struct = key.android_pubkey().unwrap();
        let pubkey_bytes: [u8; std::mem::size_of::<rust_adb_crypto::AndroidPubkey>()] =
            unsafe { std::mem::transmute(pubkey_struct) };
        let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(&pubkey_bytes);

        let mut p_pub = Apacket::default();
        p_pub.msg.command = adb_protocol::A_AUTH;
        p_pub.msg.arg0 = adb_auth::ADB_AUTH_RSAPUBLICKEY;
        p_pub.payload = Block::from_vec(pubkey_b64.into_bytes());
        p_pub.msg.data_length = p_pub.payload.get_ref().len() as u32;

        handle_packet(p_pub, &t);

        assert!(prompt_called.load(Ordering::SeqCst));
        assert_eq!(t.get_connection_state(), ConnectionState::Device);

        std::env::remove_var("ADB_VENDOR_KEYS");
    }
}

/// Ported from original/transport.h: `struct Connection`
pub trait Connection: Send + Sync {
    fn set_transport(&self, transport: Weak<ATransport>);
    fn write(&self, packet: Apacket) -> bool;
    fn start(&self, transport: Weak<ATransport>) -> bool;
    fn stop(&self);
    /// Performs the TLS handshake for secure ADB connections.
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

/// Ported from original/transport.h: `struct BlockingConnectionAdapter`
pub struct BlockingConnectionAdapter {
    underlying: Arc<dyn BlockingConnection>,
    transport: Mutex<Option<Weak<ATransport>>>,
    read_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    write_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    write_queue: Arc<(Mutex<VecDeque<Apacket>>, std::sync::Condvar)>,
    stopped: Arc<AtomicBool>,
}

impl BlockingConnectionAdapter {
    pub fn new(underlying: Arc<dyn BlockingConnection>) -> Self {
        Self {
            underlying,
            transport: Mutex::new(None),
            read_thread: Mutex::new(None),
            write_thread: Mutex::new(None),
            write_queue: Arc::new((Mutex::new(VecDeque::new()), std::sync::Condvar::new())),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Connection for BlockingConnectionAdapter {
    fn set_transport(&self, transport: Weak<ATransport>) {
        *self.transport.lock().unwrap() = Some(transport);
    }

    fn write(&self, packet: Apacket) -> bool {
        let (lock, cv) = &*self.write_queue;
        let mut queue = lock.lock().unwrap();
        queue.push_back(packet);
        cv.notify_one();
        true
    }

    fn start(&self, transport: Weak<ATransport>) -> bool {
        println!("Adapter start called");
        *self.transport.lock().unwrap() = Some(transport.clone());
        let stopped = self.stopped.clone();
        let underlying = self.underlying.clone();
        let transport_weak = Some(transport);
        let write_queue = self.write_queue.clone();

        stopped.store(false, Ordering::SeqCst);

        let mut read_thread_lock = self.read_thread.lock().unwrap();
        let should_start_read = match &*read_thread_lock {
            None => true,
            Some(h) => h.is_finished(),
        };
        println!("Should start read: {}", should_start_read);

        if should_start_read {
            if let Some(h) = read_thread_lock.take() {
                let _ = h.join();
            }
            let t_weak_read = transport_weak.clone();
            let stopped_read = stopped.clone();
            let underlying_read = underlying.clone();

            *read_thread_lock = Some(std::thread::spawn(move || {
                println!("Read thread started");
                while !stopped_read.load(Ordering::SeqCst) {
                    match underlying_read.read() {
                        Ok(packet) => {
                            let got_stls_cmd = packet.msg.command == adb_protocol::A_STLS;
                            if let Some(t_weak) = &t_weak_read {
                                if let Some(t) = t_weak.upgrade() {
                                    t.handle_read(packet);
                                }
                            }
                            if got_stls_cmd {
                                break;
                            }
                        }
                        Err(e) => {
                            println!("Read thread error: {:?}", e);
                            break;
                        }
                    }
                }
                println!("Read thread exiting");
            }));
        }

        let mut write_thread_lock = self.write_thread.lock().unwrap();
        let should_start_write = match &*write_thread_lock {
            None => true,
            Some(h) => h.is_finished(),
        };

        if should_start_write {
            if let Some(h) = write_thread_lock.take() {
                let _ = h.join();
            }
            let stopped_write = stopped;
            let underlying_write = underlying;
            let write_queue_write = write_queue;

            *write_thread_lock = Some(std::thread::spawn(move || {
                let (lock, cv) = &*write_queue_write;
                while !stopped_write.load(Ordering::SeqCst) {
                    let mut queue = match lock.lock() {
                        Ok(q) => q,
                        Err(_) => break,
                    };
                    while queue.is_empty() && !stopped_write.load(Ordering::SeqCst) {
                        queue = match cv.wait(queue) {
                            Ok(q) => q,
                            Err(_) => return,
                        };
                    }
                    if stopped_write.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Some(packet) = queue.pop_front() {
                        drop(queue);
                        if underlying_write.write(&packet).is_err() {
                            break;
                        }
                    }
                }
            }));
        }

        true
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let (_, cv) = &*self.write_queue;
        cv.notify_all();
        self.underlying.close();
    }

    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool {
        println!("Adapter do_tls_handshake");
        let handle = self.read_thread.lock().unwrap().take();
        if let Some(h) = handle {
            let _ = h.join();
        }

        if self.underlying.do_tls_handshake(key, auth_key) {
            let t_weak = self.transport.lock().unwrap().clone();
            if let Some(t_weak) = t_weak {
                self.start(t_weak);
            } else {
                println!("No transport to restart adapter with");
            }
            return true;
        }
        false
    }

    fn reset(&self) {
        self.underlying.reset();
    }
}

/// Ported from original/transport.h: `struct FdConnection`
pub struct FdConnection {
    file: TcpStream,
    tls: Mutex<Option<rustls::Connection>>,
}

impl FdConnection {
    pub fn new(fd: OwnedFd) -> Self {
        let file = TcpStream::from(fd);
        let _ = file.set_nonblocking(true);
        Self {
            file,
            tls: Mutex::new(None),
        }
    }

    fn wait_for_ready(&self, events: i16) -> std::io::Result<()> {
        let pfd = sysdeps::poll::AdbPollFd {
            fd: self.file.as_raw_fd(),
            events,
            revents: 0,
        };
        let res = sysdeps::poll::adb_poll(&mut [pfd], 5000); // 5s timeout
        if res == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if res == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "poll timed out",
            ));
        }
        Ok(())
    }

    fn read_fully(&self, buf: &mut [u8]) -> std::io::Result<()> {
        let mut pos = 0;
        while pos < buf.len() {
            match (&self.file).read(&mut buf[pos..]) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "EOF",
                    ))
                }
                Ok(n) => pos += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.wait_for_ready(libc::POLLIN)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn write_fully(&self, buf: &[u8]) -> std::io::Result<()> {
        let mut pos = 0;
        while pos < buf.len() {
            match (&self.file).write(&buf[pos..]) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "Write zero",
                    ))
                }
                Ok(n) => pos += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.wait_for_ready(libc::POLLOUT)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl FdConnection {
    fn read_tls_blocking(&self, buf: &mut [u8]) -> std::io::Result<()> {
        let mut pos = 0;
        println!("read_tls_blocking: want {}", buf.len());
        while pos < buf.len() {
            let mut tls_lock = self.tls.lock().unwrap();
            let tls = tls_lock
                .as_mut()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "TLS not active"))?;

            // 1. Try to read from the decrypted buffer
            match tls.reader().read(&mut buf[pos..]) {
                Ok(n) if n > 0 => {
                    pos += n;
                    continue;
                }
                _ => {
                    // Fall through to read from network
                }
            }

            // 2. Need more data from the network
            match tls.read_tls(&mut &self.file) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "TLS EOF",
                    ))
                }
                Ok(_) => {
                    if let Err(e) = tls.process_new_packets() {
                        return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    drop(tls_lock);
                    self.wait_for_ready(libc::POLLIN)?;
                }
                Err(e) => {
                    println!("Tls read error: {:?}", e);
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn write_tls_blocking(&self, buf: &[u8]) -> std::io::Result<()> {
        {
            let mut tls_lock = self.tls.lock().unwrap();
            let tls = tls_lock
                .as_mut()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "TLS not active"))?;
            tls.writer().write_all(buf)?;
        }

        loop {
            let mut tls_lock = self.tls.lock().unwrap();
            let tls = tls_lock
                .as_mut()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "TLS not active"))?;

            if !tls.wants_write() {
                return Ok(());
            }

            match tls.write_tls(&mut &self.file) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    drop(tls_lock);
                    self.wait_for_ready(libc::POLLOUT)?;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Connection for FdConnection {
    fn set_transport(&self, _transport: Weak<ATransport>) {}

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
        let is_tls = self.tls.lock().unwrap().is_some();
        if is_tls {
            let mut header_buf = [0u8; std::mem::size_of::<Amessage>()];
            self.read_tls_blocking(&mut header_buf)?;

            // SAFETY: Amessage is repr(C) and we've read the correct number of bytes.
            let msg: Amessage =
                unsafe { std::ptr::read_unaligned(header_buf.as_ptr() as *const Amessage) };

            let mut payload = Block::new(msg.data_length as usize);
            if msg.data_length > 0 {
                self.read_tls_blocking(payload.get_mut())?;
            }

            Ok(Apacket { msg, payload })
        } else {
            let mut header_buf = [0u8; std::mem::size_of::<Amessage>()];
            self.read_fully(&mut header_buf)?;

            // SAFETY: Amessage is repr(C) and we've read the correct number of bytes.
            let msg: Amessage =
                unsafe { std::ptr::read_unaligned(header_buf.as_ptr() as *const Amessage) };

            let mut payload = Block::new(msg.data_length as usize);
            if msg.data_length > 0 {
                self.read_fully(payload.get_mut())?;
            }

            Ok(Apacket { msg, payload })
        }
    }

    fn write(&self, packet: &Apacket) -> std::io::Result<()> {
        let is_tls = self.tls.lock().unwrap().is_some();
        if is_tls {
            // SAFETY: Amessage is repr(C).
            let header_bytes: [u8; std::mem::size_of::<Amessage>()] =
                unsafe { std::mem::transmute(packet.msg) };
            self.write_tls_blocking(&header_bytes)?;
            if !packet.payload.is_empty() {
                self.write_tls_blocking(packet.payload.get_ref())?;
            }
        } else {
            // SAFETY: Amessage is repr(C).
            let header_bytes: [u8; std::mem::size_of::<Amessage>()] =
                unsafe { std::mem::transmute(packet.msg) };
            self.write_fully(&header_bytes)?;
            if !packet.payload.is_empty() {
                self.write_fully(packet.payload.get_ref())?;
            }
        }
        Ok(())
    }

    fn do_tls_handshake(&self, key: &Key, auth_key: Option<&mut String>) -> bool {
        let cert = match rust_adb_crypto::generate_x509_certificate(key) {
            Ok(c) => c,
            Err(e) => {
                log::error!(target: "transport", "failed to generate x509 cert: {}", e);
                return false;
            }
        };

        let cert_pem = match rust_adb_crypto::x509_to_pem_string(&cert) {
            Ok(s) => s,
            Err(e) => {
                log::error!(target: "transport", "failed to PEM encode cert: {}", e);
                return false;
            }
        };
        let key_pem = match key.to_pem_string() {
            Ok(s) => s,
            Err(e) => {
                log::error!(target: "transport", "failed to PEM encode key: {}", e);
                return false;
            }
        };

        let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
            .unwrap()
            .into_iter()
            .map(rustls::Certificate)
            .collect::<Vec<_>>();
        let priv_key = rustls_pemfile::pkcs8_private_keys(&mut key_pem.as_bytes())
            .unwrap()
            .into_iter()
            .map(rustls::PrivateKey)
            .next()
            .expect("No private key");

        let mut conn = if auth_key.is_none() {
            // Client role (Host)
            let config = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(Arc::new(AdbServerCertVerifier))
                .with_client_auth_cert(certs, priv_key)
                .expect("Failed to build client config");
            rustls::Connection::Client(
                rustls::ClientConnection::new(Arc::new(config), "adb".try_into().unwrap())
                    .expect("Failed to create client connection"),
            )
        } else {
            // Server role (Daemon)
            let config = rustls::ServerConfig::builder()
                .with_safe_defaults()
                .with_client_cert_verifier(Arc::new(AdbClientCertVerifier))
                .with_single_cert(certs, priv_key)
                .expect("Failed to build server config");
            rustls::Connection::Server(
                rustls::ServerConnection::new(Arc::new(config))
                    .expect("Failed to create server connection"),
            )
        };

        loop {
            while conn.wants_write() {
                match conn.write_tls(&mut &self.file) {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Err(e) = self.wait_for_ready(libc::POLLOUT) {
                            log::error!(target: "transport", "wait_for_ready (OUT) failed: {}", e);
                            return false;
                        }
                    }
                    Err(e) => {
                        log::error!(target: "transport", "TLS write failed: {}", e);
                        return false;
                    }
                }
            }

            if !conn.is_handshaking() {
                break;
            }

            if conn.wants_read() {
                match conn.read_tls(&mut &self.file) {
                    Ok(0) => {
                        log::error!(target: "transport", "TLS read EOF during handshake");
                        return false;
                    }
                    Ok(_) => {
                        if let Err(e) = conn.process_new_packets() {
                            log::error!(target: "transport", "TLS process packets failed: {}", e);
                            return false;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Err(e) = self.wait_for_ready(libc::POLLIN) {
                            log::error!(target: "transport", "wait_for_ready (IN) failed: {}", e);
                            return false;
                        }
                    }
                    Err(e) => {
                        log::error!(target: "transport", "TLS read failed: {}", e);
                        return false;
                    }
                }
            }
        }

        *self.tls.lock().unwrap() = Some(conn);
        true
    }

    fn close(&self) {
        // TcpStream handles close on drop.
    }

    fn reset(&self) {
        self.close();
    }
}

struct AdbServerCertVerifier;

impl rustls::client::ServerCertVerifier for AdbServerCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

struct AdbClientCertVerifier;

impl rustls::server::ClientCertVerifier for AdbClientCertVerifier {
    fn client_auth_root_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _now: SystemTime,
    ) -> Result<rustls::server::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::ClientCertVerified::assertion())
    }
}

/// Ported from original/client/usb.h: `struct UsbConnection`
///
/// Current implementation is a stub. Full implementation will be done in Step 17.
pub struct UsbConnection {}

impl UsbConnection {
    pub fn new() -> Self {
        Self {}
    }
}

impl BlockingConnection for UsbConnection {
    fn read(&self) -> std::io::Result<Apacket> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "UsbConnection::read not implemented",
        ))
    }

    fn write(&self, _packet: &Apacket) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "UsbConnection::write not implemented",
        ))
    }

    fn do_tls_handshake(&self, _key: &Key, _auth_key: Option<&mut String>) -> bool {
        false
    }

    fn close(&self) {}

    fn reset(&self) {}
}

impl Connection for UsbConnection {
    fn set_transport(&self, _transport: Weak<ATransport>) {}

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
