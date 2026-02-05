/*
 * Copyright (C) 2023 The Android Open Source Project
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

//! ADB Services implementation.
//! Ported from original/services.h and original/services.cpp.

use adb_protocol::{ConnectionState, TransportType};
use adb_sockets::{create_local_socket, Socket, SocketRegistry};
use adb_transport::{
    acquire_one_transport, list_transports, ATransport, FdConnection, TrackerOutputType,
    TransportId,
};
use sysdeps::poll::{adb_poll, AdbPollFd};
use adb_io::{send_fail, send_okay, send_protocol_string};
use fdevent::fdevent::Fdevent;
use adb_socket_spec::{is_socket_spec, socket_spec_connect};
use std::os::unix::io::{AsRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

pub const K_SHELL_SERVICE_ARG_RAW: &str = "raw";
pub const K_SHELL_SERVICE_ARG_PTY: &str = "pty";
pub const K_SHELL_SERVICE_ARG_SHELL_PROTOCOL: &str = "v2";

pub const K_MINADBD_SERVICES_EXIT_SUCCESS: &str = "DONEDONE";
pub const K_MINADBD_SERVICES_EXIT_FAILURE: &str = "FAILFAIL";

/// Creates a socket pair, starts a new thread with the provided function,
/// and returns one end of the socket pair as an `OwnedFd`.
/// Ported from `create_service_thread` in `original/services.cpp`.
pub fn create_service_thread<F>(service_name: &str, func: F) -> std::io::Result<OwnedFd>
where
    F: FnOnce(OwnedFd) + Send + 'static,
{
    let (s0, s1) = UnixStream::pair()?;

    let service_name = service_name.to_string();
    std::thread::Builder::new()
        .name(format!("{} svc", service_name))
        .spawn(move || {
            func(s1.into());
        })?;

    Ok(s0.into())
}

/// Service that waits for a transport to reach a certain state.
/// Ported from `wait_service` in `original/services.cpp`.
pub fn wait_service(
    fd: OwnedFd,
    serial: String,
    transport_id: TransportId,
    spec: String,
) {
    let mut file = std::fs::File::from(fd);
    let components: Vec<&str> = spec.split('-').collect();
    if components.len() < 2 {
        let _ = send_fail(&mut file, &format!("short wait-for-: {}", spec));
        return;
    }

    let transport_type = match components[0] {
        "local" => TransportType::Local,
        "usb" => TransportType::Usb,
        "any" => TransportType::Any,
        _ => {
            let _ = send_fail(&mut file, &format!("bad wait-for- transport: {}", spec));
            return;
        }
    };

    let mut states = Vec::new();
    for component in &components[1..] {
        match *component {
            "device" => states.push(ConnectionState::Device),
            "recovery" => states.push(ConnectionState::Recovery),
            "rescue" => states.push(ConnectionState::Rescue),
            "sideload" => states.push(ConnectionState::Sideload),
            "bootloader" => states.push(ConnectionState::Bootloader),
            "any" => states.push(ConnectionState::Any),
            "disconnect" => states.push(ConnectionState::Offline),
            _ => {
                let _ = send_fail(&mut file, &format!("bad wait-for- state: {}", spec));
                return;
            }
        }
    }

    loop {
        let serial_ptr = if serial.is_empty() {
            None
        } else {
            Some(serial.as_str())
        };

        let t_result = acquire_one_transport(transport_type, serial_ptr, transport_id);

        let mut matched = false;
        match t_result {
            Ok(t) => {
                for state in &states {
                    if *state == ConnectionState::Any || *state == t.get_connection_state() {
                        matched = true;
                        break;
                    }
                }
            }
            Err(e) => {
                if e.contains("more than one device/emulator") {
                    let _ = send_fail(&mut file, &e);
                    return;
                }
                // device not found
                for state in &states {
                    if *state == ConnectionState::Offline {
                        matched = true;
                        break;
                    }
                }
            }
        }

        if matched {
            let _ = send_okay(&mut file);
            return;
        }

        // Sleep before retrying, or bail if client closed.
        let pfd = AdbPollFd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        match adb_poll(&mut [pfd], 100) {
            n if n > 0 => {
                // Client closed or sent data (we don't expect data).
                return;
            }
            0 => {}
            _ => return,
        }
    }
}

/// Service that handles device connection.
/// Ported from `connect_service` in `original/services.cpp`.
pub fn connect_service(fd: OwnedFd, host: String) {
    let mut file = std::fs::File::from(fd);
    let mut response = String::new();
    if host.starts_with("emu:") {
        connect_emulator(&host[4..], &mut response);
    } else {
        connect_device(host, &mut response);
    }
    let _ = send_protocol_string(&mut file, &response);
}

/// Connects to an emulator.
/// Ported from `connect_emulator` in `original/services.cpp`.
pub fn connect_emulator(port_spec: &str, response: &mut String) {
    let pieces: Vec<&str> = port_spec.split(',').collect();
    if pieces.len() != 2 {
        *response = format!("unable to parse '{}' as <console port>,<adb port>", port_spec);
        return;
    }

    let console_port = pieces[0].parse::<i32>().unwrap_or(0);
    let adb_port = pieces[1].parse::<i32>().unwrap_or(0);
    if console_port <= 0 || adb_port <= 0 {
        *response = format!("Invalid port numbers: {}", port_spec);
        return;
    }

    // Preconditions met, try to connect to the emulator.
    // In Rust we just use connect_device with the adb port.
    connect_device(format!("localhost:{}", adb_port), response);
    if response.contains("connected to") {
        *response = format!("connected to emulator on ports {},{}", console_port, adb_port);
    }
}

/// Connects to a device via its address.
/// Ported from `connect_device` in `original/client/transport_emulator.cpp`.
pub fn connect_device(address: String, response: &mut String) {
    if address.is_empty() {
        *response = "empty address".to_string();
        return;
    }

    let prefix_addr = if address.starts_with("vsock:") || address.starts_with("localfilesystem:") || address.starts_with("tcp:") {
        address.clone()
    } else {
        format!("tcp:{}", address)
    };

    let mut port = 0;
    let mut serial = String::new();
    match socket_spec_connect(&prefix_addr, Some(&mut port), Some(&mut serial)) {
        Ok(fd) => {
            let t = Arc::new(ATransport::new_offline(TransportType::Local));
            *t.serial.lock().unwrap() = serial;

            let connection = Arc::new(FdConnection::new(fd));
            t.set_connection(connection);

            // TODO: In a full implementation, we would start the transport packet loop here.
            adb_transport::register_transport(t);
            *response = format!("connected to {}", address);
        }
        Err(e) => {
            *response = format!("failed to connect to {}: {}", address, e);
        }
    }
}

/// Service that handles device pairing.
/// Ported from `pair_service` in `original/services.cpp`.
pub fn pair_service(fd: OwnedFd, _host: String, _password: String) {
    let mut file = std::fs::File::from(fd);
    // TODO: implement adb_wifi_pair_device
    let _ = send_fail(&mut file, "pairing not implemented in Rust yet");
}

/// Dispatches a host-side service to a socket.
/// Ported from `host_service_to_socket` in `original/services.cpp`.
pub fn host_service_to_socket(
    name: &str,
    serial: &str,
    transport_id: TransportId,
    registry: Arc<SocketRegistry>,
    fdevent: &mut Fdevent,
) -> Option<Arc<dyn Socket>> {
    if name == "track-devices" {
        return Some(create_device_tracker(TrackerOutputType::ShortText, registry));
    }
    if name == "track-devices-l" {
        return Some(create_device_tracker(TrackerOutputType::LongText, registry));
    }
    if name == "track-devices-proto" {
        return Some(create_device_tracker(TrackerOutputType::Protobuf, registry));
    }

    if let Some(spec) = name.strip_prefix("wait-for-") {
        let serial = serial.to_string();
        let spec = spec.to_string();
        let fd = create_service_thread("wait", move |fd| {
            wait_service(fd, serial, transport_id, spec);
        })
        .ok()?;
        return Some(create_local_socket(fd.into_raw_fd(), registry, fdevent));
    }

    if let Some(host) = name.strip_prefix("connect:") {
        let host = host.to_string();
        let fd = create_service_thread("connect", move |fd| {
            connect_service(fd, host);
        })
        .ok()?;
        return Some(create_local_socket(fd.into_raw_fd(), registry, fdevent));
    }

    if let Some(pair_spec) = name.strip_prefix("pair:") {
        if let Some(colon_idx) = pair_spec.find(':') {
            let password = pair_spec[..colon_idx].to_string();
            let host = pair_spec[colon_idx + 1..].to_string();
            let fd = create_service_thread("pair", move |fd| {
                pair_service(fd, host, password);
            })
            .ok()?;
            return Some(create_local_socket(fd.into_raw_fd(), registry, fdevent));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use adb_protocol::TransportType;
    use adb_transport::{register_transport, ATransport};

    #[test]
    fn test_create_service_thread() {
        let fd = create_service_thread("test", |fd| {
            let mut file = std::fs::File::from(fd);
            file.write_all(b"hello").unwrap();
        })
        .unwrap();

        let mut file = std::fs::File::from(fd);
        let mut buf = [0u8; 5];
        file.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn test_wait_service_device_found() {
        let t = Arc::new(ATransport::new(
            TransportType::Usb,
            Box::new(|_| adb_transport::ReconnectResult::Abort),
            ConnectionState::Device,
        ));
        *t.serial.lock().unwrap() = "test_serial".to_string();
        register_transport(t);

        let (s1, s2) = UnixStream::pair().unwrap();
        let s2_fd = OwnedFd::from(s2);

        let spec = "usb-device".to_string();
        let serial = "test_serial".to_string();

        let handle = std::thread::spawn(move || {
            wait_service(s2_fd, serial, 0, spec);
        });

        let mut s1_file = std::fs::File::from(OwnedFd::from(s1));
        let mut buf = [0u8; 4];
        s1_file.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"OKAY");

        handle.join().unwrap();
    }

    #[test]
    fn test_wait_service_disconnect() {
        // We don't register any transport, so acquire_one_transport will fail (device not found).
        // wait-for-disconnect should succeed.

        let (s1, s2) = UnixStream::pair().unwrap();
        let s2_fd = OwnedFd::from(s2);

        let spec = "any-disconnect".to_string();
        let serial = "non_existent".to_string();

        let handle = std::thread::spawn(move || {
            wait_service(s2_fd, serial, 0, spec);
        });

        let mut s1_file = std::fs::File::from(OwnedFd::from(s1));
        let mut buf = [0u8; 4];
        s1_file.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"OKAY");

        handle.join().unwrap();
    }

    #[test]
    fn test_shell_service() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let s2_fd = OwnedFd::from(s2);

        let handle = std::thread::spawn(move || {
            shell_service(s2_fd, ":echo hello");
        });

        let mut s1_file = std::fs::File::from(OwnedFd::from(s1));
        let mut buf = [0u8; 64];
        let n = s1_file.read(&mut buf).unwrap();
        assert_eq!(String::from_utf8_lossy(&buf[..n]).trim(), "hello");

        handle.join().unwrap();
    }

    #[test]
    fn test_device_tracker() {
        let registry = Arc::new(SocketRegistry::new());
        let tracker = create_device_tracker(TrackerOutputType::ShortText, registry.clone());

        struct MockSocket {
            id: u32,
            data: std::sync::Mutex<Vec<bytes::Bytes>>,
        }
        impl Socket for MockSocket {
            fn id(&self) -> u32 { self.id }
            fn enqueue(&self, data: bytes::Bytes) -> i32 {
                self.data.lock().unwrap().push(data);
                0
            }
            fn ready(&self) {}
            fn shutdown(&self) {}
            fn close(&self) {}
            fn peer_id(&self) -> Option<u32> { None }
            fn transport_id(&self) -> Option<u64> { None }
            fn set_peer(&self, _peer: Arc<dyn Socket>) {}
        }

        let mock_peer = Arc::new(MockSocket {
            id: 200,
            data: std::sync::Mutex::new(Vec::new()),
        });

        tracker.set_peer(mock_peer.clone());
        tracker.ready();

        let data = mock_peer.data.lock().unwrap();
        assert_eq!(data.len(), 1);
        // Check that it starts with 4 hex digits representing the length.
        let len_str = std::str::from_utf8(&data[0][0..4]).unwrap();
        let len = usize::from_str_radix(len_str, 16).unwrap();
        assert_eq!(len, data[0].len() - 4);
    }

    #[test]
    fn test_shell_service_args() {
        let (s1, s2) = UnixStream::pair().unwrap();
        let s2_fd = OwnedFd::from(s2);

        // Test with raw argument
        let handle = std::thread::spawn(move || {
            shell_service(s2_fd, "raw:echo hello");
        });

        let mut s1_file = std::fs::File::from(OwnedFd::from(s1));
        let mut buf = [0u8; 64];
        let n = s1_file.read(&mut buf).unwrap();
        assert_eq!(String::from_utf8_lossy(&buf[..n]).trim(), "hello");

        handle.join().unwrap();
    }
}

/// Dispatches a service to a file descriptor.
/// Ported from `service_to_fd` in `original/services.cpp`.
pub fn service_to_fd(name: &str, _transport: Option<&Arc<ATransport>>) -> std::io::Result<OwnedFd> {
    if is_socket_spec(name) {
        match socket_spec_connect(name, None, None) {
            Ok(fd) => Ok(fd),
            Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
        }
    } else {
        // TODO: handle daemon services via daemon_service_to_fd
        if let Some(_transport) = _transport {
            if let Some(fd) = daemon_service_to_fd(name, _transport) {
                return Ok(fd);
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("unknown service: {}", name),
        ))
    }
}

/// Dispatches a service to a socket on the daemon side.
/// Ported from `daemon_service_to_socket` in `original/daemon/services.cpp`.
pub fn daemon_service_to_socket(
    name: &str,
    _transport: &Arc<ATransport>,
) -> Option<Arc<dyn Socket>> {
    if name == "jdwp" {
        // TODO: implement create_jdwp_service_socket
        return None;
    }
    if name == "track-jdwp" {
        // TODO: implement create_jdwp_tracker_service_socket
        return None;
    }
    if name == "track-app" {
        // TODO: implement create_app_tracker_service_socket
        return None;
    }

    if let Some(count_str) = name.strip_prefix("sink:") {
        if let Ok(count) = count_str.parse::<u64>() {
            let registry = _transport.registry.lock().unwrap().as_ref()?.clone();
            let id = registry.alloc_id();
            let socket = Arc::new(SinkSocket {
                id,
                registry: registry.clone(),
                peer: std::sync::Mutex::new(None),
                bytes_left: std::sync::atomic::AtomicU64::new(count),
            });
            registry.install(socket.clone());
            return Some(socket);
        }
    }

    if let Some(count_str) = name.strip_prefix("source:") {
        if let Ok(count) = count_str.parse::<u64>() {
            let registry = _transport.registry.lock().unwrap().as_ref()?.clone();
            let id = registry.alloc_id();
            let socket = Arc::new(SourceSocket {
                id,
                registry: registry.clone(),
                peer: std::sync::Mutex::new(None),
                bytes_left: std::sync::atomic::AtomicU64::new(count),
            });
            registry.install(socket.clone());
            return Some(socket);
        }
    }

    None
}

/// Service that runs a shell command.
/// Simplified version of `ShellService` in `original/daemon/services.cpp`.
pub fn shell_service(fd: OwnedFd, args: &str) {
    let mut command = "";
    let mut _subprocess_type = "pty";
    let mut _protocol = "none";
    let mut _terminal_type = "dumb";

    if let Some(colon_idx) = args.find(':') {
        let service_args = &args[..colon_idx];
        command = &args[colon_idx + 1..];

        _subprocess_type = if command.is_empty() { "pty" } else { "raw" };

        for arg in service_args.split(',') {
            match arg {
                K_SHELL_SERVICE_ARG_RAW => _subprocess_type = "raw",
                K_SHELL_SERVICE_ARG_PTY => _subprocess_type = "pty",
                K_SHELL_SERVICE_ARG_SHELL_PROTOCOL => _protocol = "v2",
                _ if arg.starts_with("TERM=") => _terminal_type = &arg[5..],
                _ => {}
            }
        }
    }

    let mut cmd = if command.is_empty() {
        std::process::Command::new("/bin/sh")
    } else {
        let mut c = std::process::Command::new("/bin/sh");
        c.arg("-c").arg(command);
        c
    };

    let fd_clone = fd.try_clone().expect("failed to clone fd");
    let fd_clone2 = fd.try_clone().expect("failed to clone fd");

    let mut child = cmd
        .stdin(std::process::Stdio::from(fd))
        .stdout(std::process::Stdio::from(fd_clone))
        .stderr(std::process::Stdio::from(fd_clone2))
        .spawn()
        .expect("failed to spawn shell");

    let _ = child.wait();
}

/// Dispatches a service to a file descriptor on the daemon side.
/// Ported from `daemon_service_to_fd` in `original/daemon/services.cpp`.
pub fn daemon_service_to_fd(name: &str, _transport: &Arc<ATransport>) -> Option<OwnedFd> {
    if let Some(args) = name.strip_prefix("shell") {
        let args = args.to_string();
        return create_service_thread("shell", move |fd| {
            shell_service(fd, &args);
        })
        .ok();
    }
    if let Some(cmd) = name.strip_prefix("exec:") {
        let cmd = cmd.to_string();
        return create_service_thread("exec", move |fd| {
            shell_service(fd, &format!(":{}", cmd));
        })
        .ok();
    }
    if name.starts_with("sync:") {
        return create_service_thread("sync", |fd| {
            let mut file = std::fs::File::from(fd);
            // We can't easily implement the full sync protocol here yet,
            // but we can at least send a FAIL message if the protocol allows it at this stage.
            let _ = send_fail(&mut file, "sync service not implemented in Rust yet");
        })
        .ok();
    }
    if let Some(cmd) = name.strip_prefix("reverse:") {
        return reverse_service(cmd, _transport);
    }
    if name == "reconnect" {
        let t = Arc::downgrade(_transport);
        return create_service_thread("reconnect", move |fd| {
            reconnect_service(fd, t.upgrade().as_ref());
        })
        .ok();
    }

    None
}

struct DeviceTracker {
    id: u32,
    output_type: TrackerOutputType,
    registry: Arc<SocketRegistry>,
    peer: std::sync::Mutex<Option<Arc<dyn Socket>>>,
}

impl DeviceTracker {
    fn new(id: u32, output_type: TrackerOutputType, registry: Arc<SocketRegistry>) -> Arc<Self> {
        let tracker = Arc::new(Self {
            id,
            output_type,
            registry,
            peer: std::sync::Mutex::new(None),
        });

        let tracker_weak = Arc::downgrade(&tracker);
        adb_transport::register_transport_observer(Box::new(move || {
            if let Some(tracker) = tracker_weak.upgrade() {
                tracker.update();
            }
        }));

        tracker
    }

    fn update(&self) {
        let list = list_transports(self.output_type);
        if let Some(peer) = self.peer.lock().unwrap().as_ref() {
            let mut data = Vec::with_capacity(list.len() + 4);
            data.extend_from_slice(format!("{:04x}", list.len()).as_bytes());
            data.extend_from_slice(list.as_bytes());
            peer.enqueue(bytes::Bytes::from(data));
        }
    }
}

impl Socket for DeviceTracker {
    fn id(&self) -> u32 {
        self.id
    }

    fn enqueue(&self, _data: bytes::Bytes) -> i32 {
        // Tracker doesn't accept data from peer.
        self.close();
        -1
    }

    fn ready(&self) {
        self.update();
    }

    fn shutdown(&self) {}

    fn close(&self) {
        if let Some(peer) = self.peer.lock().unwrap().take() {
            peer.close();
        }
        self.registry.remove(self.id);
    }

    fn peer_id(&self) -> Option<u32> {
        self.peer.lock().unwrap().as_ref().map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        None
    }

    fn set_peer(&self, peer: Arc<dyn Socket>) {
        *self.peer.lock().unwrap() = Some(peer);
    }
}

/// Creates a device tracker socket.
/// Ported from `create_device_tracker` in `original/transport.cpp`.
pub fn create_device_tracker(
    output_type: TrackerOutputType,
    registry: Arc<SocketRegistry>,
) -> Arc<dyn Socket> {
    let id = registry.alloc_id();
    let tracker = DeviceTracker::new(id, output_type, registry.clone());
    registry.install(tracker.clone());
    tracker
}

/// Service that handles reverse forwarding requests.
/// Ported from `reverse_service` in `original/daemon/services.cpp`.
pub fn reverse_service(_command: &str, _transport: &Arc<ATransport>) -> Option<OwnedFd> {
    // TODO: Implement reverse_service by porting handle_forward_request from adb_listeners.cpp.
    None
}

/// Service that handles reconnection.
pub fn reconnect_service(fd: OwnedFd, transport: Option<&Arc<ATransport>>) {
    let mut file = std::fs::File::from(fd);
    let _ = send_protocol_string(&mut file, "done");
    if let Some(t) = transport {
        t.kick();
    } else {
        adb_transport::kick_all_transports();
    }
}

struct SinkSocket {
    id: u32,
    registry: Arc<SocketRegistry>,
    peer: std::sync::Mutex<Option<Arc<dyn Socket>>>,
    bytes_left: std::sync::atomic::AtomicU64,
}

impl Socket for SinkSocket {
    fn id(&self) -> u32 {
        self.id
    }

    fn enqueue(&self, data: bytes::Bytes) -> i32 {
        let left = self.bytes_left.load(std::sync::atomic::Ordering::Relaxed);
        if left <= data.len() as u64 {
            self.close();
            return -1;
        }
        self.bytes_left
            .fetch_sub(data.len() as u64, std::sync::atomic::Ordering::Relaxed);
        0
    }

    fn ready(&self) {}

    fn shutdown(&self) {}

    fn close(&self) {
        if let Some(peer) = self.peer.lock().unwrap().take() {
            peer.close();
        }
        self.registry.remove(self.id);
    }

    fn peer_id(&self) -> Option<u32> {
        self.peer.lock().unwrap().as_ref().map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        None
    }

    fn set_peer(&self, peer: Arc<dyn Socket>) {
        *self.peer.lock().unwrap() = Some(peer);
    }
}

struct SourceSocket {
    id: u32,
    registry: Arc<SocketRegistry>,
    peer: std::sync::Mutex<Option<Arc<dyn Socket>>>,
    bytes_left: std::sync::atomic::AtomicU64,
}

impl Socket for SourceSocket {
    fn id(&self) -> u32 {
        self.id
    }

    fn enqueue(&self, _data: bytes::Bytes) -> i32 {
        -1
    }

    fn ready(&self) {
        let left = self.bytes_left.load(std::sync::atomic::Ordering::Relaxed);
        if left == 0 {
            self.close();
            return;
        }

        if let Some(peer) = self.peer.lock().unwrap().as_ref() {
            let len = std::cmp::min(left, 1024 * 1024) as usize;
            if len == 0 {
                self.close();
                return;
            }
            let data = vec![0u8; len];
            if peer.enqueue(bytes::Bytes::from(data)) == 0 {
                self.bytes_left
                    .fetch_sub(len as u64, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    fn shutdown(&self) {}

    fn close(&self) {
        if let Some(peer) = self.peer.lock().unwrap().take() {
            peer.close();
        }
        self.registry.remove(self.id);
    }

    fn peer_id(&self) -> Option<u32> {
        self.peer.lock().unwrap().as_ref().map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        None
    }

    fn set_peer(&self, peer: Arc<dyn Socket>) {
        *self.peer.lock().unwrap() = Some(peer);
    }
}
