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

//! This crate provides a Rust implementation of ADB's socket management logic.
//! It is ported from `original/socket.h` and `original/sockets.cpp`.

use adb_protocol::{
    A_CLSE, A_OKAY, A_OPEN, A_WRTE, INITIAL_DELAYED_ACK_BYTES, MAX_PAYLOAD,
};
use adb_types::{Apacket, Block, IoVector};
use bytes::Bytes;
use fdevent::fdevent::{Fdevent, FdeventHandler};
use mio::{event::Event, unix::SourceFd, Interest, Token};
use std::collections::HashMap;
use std::os::unix::io::RawFd;
use std::sync::{Arc, Mutex, Weak};

/// Trait representing a generic socket in the ADB system.
/// This mirrors the `asocket` struct functionality in `original/socket.h`.
pub trait Socket: Send + Sync {
    /// Returns the unique ID of the socket.
    fn id(&self) -> u32;
    /// Enqueues data to be sent through the socket.
    /// Returns 0 if more data can be accepted, 1 if blocked, and -1 on error.
    fn enqueue(&self, data: Bytes) -> i32;
    /// Called when the peer is ready to receive more data.
    fn ready(&self);
    /// Notifies the socket that it should stop sending data.
    fn shutdown(&self);
    /// Closes the socket.
    fn close(&self);
    /// Returns the ID of the peer socket, if any.
    fn peer_id(&self) -> Option<u32>;
    /// Returns the transport ID associated with the socket, if any.
    fn transport_id(&self) -> Option<u64>;
    /// Returns the socket as a `LocalSocket`, if it is one.
    fn as_local_socket(&self) -> Option<&LocalSocket> {
        None
    }
    /// Detaches the peer from this socket and returns it.
    fn take_peer(&self) -> Option<Arc<dyn Socket>> {
        None
    }
}

/// Trait representing a transport that can send ADB packets.
/// Ported from the `atransport` class in `original/transport.h`.
pub trait Transport: Send + Sync {
    /// Returns the unique ID of the transport.
    fn id(&self) -> u64;
    /// Sends an ADB packet through the transport.
    fn send_packet(&self, packet: Apacket);
    /// Sends a READY signal to the peer.
    fn send_ready(&self, local: u32, remote: u32, ack_bytes: u32);
    /// Returns the maximum payload supported by the transport.
    fn get_max_payload(&self) -> usize;
    /// Returns whether the transport supports delayed acknowledgements.
    fn supports_delayed_ack(&self) -> bool;
}

/// Inner state of the socket registry.
struct SocketRegistryInner {
    sockets: HashMap<u32, Arc<dyn Socket>>,
    closing_sockets: HashMap<u32, Arc<dyn Socket>>,
    next_id: u32,
}

/// Manages the lifecycle and identification of ADB sockets.
/// Ported from global socket management in `original/sockets.cpp`.
pub struct SocketRegistry {
    inner: Mutex<SocketRegistryInner>,
}

impl SocketRegistry {
    /// Creates a new, empty `SocketRegistry`.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SocketRegistryInner {
                sockets: HashMap::new(),
                closing_sockets: HashMap::new(),
                next_id: 1,
            }),
        }
    }

    /// Allocates a new, unique socket ID.
    /// Ported from `local_socket_next_id` in `original/sockets.cpp`.
    pub fn alloc_id(&self) -> u32 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id = inner.next_id.checked_add(1).expect("Socket ID overflow");
        id
    }

    /// Installs a socket into the registry.
    /// Ported from `install_local_socket` in `original/sockets.cpp`.
    pub fn install(&self, socket: Arc<dyn Socket>) {
        let mut inner = self.inner.lock().unwrap();
        inner.sockets.insert(socket.id(), socket);
    }

    /// Finds a socket by its ID.
    /// Ported from `find_local_socket` in `original/sockets.cpp`.
    pub fn find(&self, id: u32) -> Option<Arc<dyn Socket>> {
        let inner = self.inner.lock().unwrap();
        inner.sockets.get(&id).cloned()
    }

    /// Finds a local socket by its ID and optionally its peer's ID.
    pub fn find_local_socket(&self, local_id: u32, peer_id: u32) -> Option<Arc<dyn Socket>> {
        let inner = self.inner.lock().unwrap();
        if let Some(s) = inner.sockets.get(&local_id) {
            if peer_id == 0 || s.peer_id() == Some(peer_id) {
                return Some(s.clone());
            }
        }
        None
    }

    /// Removes a socket from the registry.
    /// Ported from `remove_socket` in `original/sockets.cpp`.
    pub fn remove(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.sockets.remove(&id);
        inner.closing_sockets.remove(&id);
    }

    /// Moves a socket to the closing list.
    /// Ported from `local_socket_closing_list` logic in `original/sockets.cpp`.
    pub fn move_to_closing(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(s) = inner.sockets.remove(&id) {
            inner.closing_sockets.insert(id, s);
        }
    }

    /// Closes all sockets associated with a specific transport.
    /// Ported from `close_all_sockets` in `original/sockets.cpp`.
    pub fn close_all_sockets(&self, transport_id: u64) {
        let ids: Vec<u32> = {
            let inner = self.inner.lock().unwrap();
            inner
                .sockets
                .values()
                .filter(|s| s.transport_id() == Some(transport_id))
                .map(|s| s.id())
                .collect()
        };

        for id in ids {
            if let Some(s) = self.find(id) {
                s.close();
            }
        }
    }
}

/// A local socket bound to a file descriptor.
/// Ported from `asocket` with local socket fields in `original/socket.h`.
#[derive(Clone)]
pub struct LocalSocket {
    inner: Arc<Mutex<LocalSocketInner>>,
}

/// Inner state of a [`LocalSocket`].
struct LocalSocketInner {
    id: u32,
    fd: RawFd,
    packet_queue: IoVector,
    peer: Option<Weak<dyn Socket>>,
    transport: Option<Arc<dyn Transport>>,
    closing: bool,
    has_write_error: bool,
    registry: Weak<SocketRegistry>,
    mio_registry: mio::Registry,
    token: Token,
    current_interests: Option<Interest>,
    available_send_bytes: Option<i64>,
    read_buffer: Vec<u8>,
}

impl LocalSocket {
    /// Creates a new `LocalSocket`.
    pub fn new(
        id: u32,
        fd: RawFd,
        registry: Arc<SocketRegistry>,
        mio_registry: mio::Registry,
        token: Token,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LocalSocketInner {
                id,
                fd,
                packet_queue: IoVector::new(),
                peer: None,
                transport: None,
                closing: false,
                has_write_error: false,
                registry: Arc::downgrade(&registry),
                mio_registry,
                token,
                current_interests: Some(Interest::READABLE),
                available_send_bytes: None,
                read_buffer: vec![0u8; MAX_PAYLOAD],
            })),
        }
    }

    /// Sets the peer socket.
    pub fn set_peer(&self, peer: Arc<dyn Socket>) {
        let mut inner = self.inner.lock().unwrap();
        inner.peer = Some(Arc::downgrade(&peer));
    }

    /// Sets the associated transport.
    pub fn set_transport(&self, transport: Arc<dyn Transport>) {
        let mut inner = self.inner.lock().unwrap();
        inner.transport = Some(transport);
    }

    /// Returns the associated transport, if any.
    pub fn get_transport(&self) -> Option<Arc<dyn Transport>> {
        let inner = self.inner.lock().unwrap();
        inner.transport.clone()
    }

    /// Returns the file descriptor associated with the socket.
    pub fn fd(&self) -> RawFd {
        self.inner.lock().unwrap().fd
    }

    /// Returns the socket registry associated with the socket.
    pub fn get_registry(&self) -> Option<Arc<SocketRegistry>> {
        let inner = self.inner.lock().unwrap();
        inner.registry.upgrade()
    }
}

/// Connects a local socket to a remote service.
/// Ported from `connect_to_remote` in `original/sockets.cpp`.
pub fn connect_to_remote(socket: &LocalSocket, destination: &str) {
    let inner = socket.inner.lock().unwrap();
    if let Some(transport) = &inner.transport {
        log::debug!("LS({}): connect({})", inner.id, destination);
        let mut p = Apacket::default();
        p.msg.command = A_OPEN;
        p.msg.arg0 = inner.id;

        if transport.supports_delayed_ack() {
            p.msg.arg1 = INITIAL_DELAYED_ACK_BYTES as u32;
        }

        // adbd used to expect a null-terminated string.
        // Keep doing so to maintain backward compatibility.
        let mut payload = destination.as_bytes().to_vec();
        payload.push(0);
        p.msg.data_length = payload.len() as u32;
        p.payload = Block(std::io::Cursor::new(payload));

        transport.send_packet(p);
    }
}

impl Socket for LocalSocket {
    fn id(&self) -> u32 {
        self.inner.lock().unwrap().id
    }

    fn enqueue(&self, data: Bytes) -> i32 {
        let mut inner = self.inner.lock().unwrap();
        inner.packet_queue.append(data);
        let flush_res = inner.flush_incoming();
        match flush_res {
            FlushResult::Destroyed => -1,
            FlushResult::TryAgain => 1,
            FlushResult::Completed => 0,
        }
    }

    fn ready(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.add_interest(Interest::READABLE);
    }

    fn shutdown(&self) {
        // Optional in C++
    }

    fn close(&self) {
        let peer = {
            let mut inner = self.inner.lock().unwrap();
            inner.peer.take().and_then(|p| p.upgrade())
        };

        if let Some(peer) = peer {
            peer.shutdown();
            peer.close();
        }

        let mut inner = self.inner.lock().unwrap();
        if inner.closing || inner.has_write_error || inner.packet_queue.is_empty() {
            inner.destroy();
        } else {
            inner.closing = true;
            inner.update_interests(Some(Interest::WRITABLE));
            if let Some(registry) = inner.registry.upgrade() {
                registry.move_to_closing(inner.id);
            }
        }
    }

    fn peer_id(&self) -> Option<u32> {
        let inner = self.inner.lock().unwrap();
        inner
            .peer
            .as_ref()
            .and_then(|p| p.upgrade())
            .map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        inner.transport.as_ref().map(|t| t.id())
    }

    fn as_local_socket(&self) -> Option<&LocalSocket> {
        Some(self)
    }

    fn take_peer(&self) -> Option<Arc<dyn Socket>> {
        let mut inner = self.inner.lock().unwrap();
        inner.peer.take().and_then(|p| p.upgrade())
    }
}

/// Result of a socket flush operation.
enum FlushResult {
    Destroyed,
    TryAgain,
    Completed,
}

impl LocalSocketInner {
    /// Updates the `mio` registration with new interests.
    /// This handles transitions between None (deregister) and Some (register/reregister).
    fn update_interests(&mut self, new_interests: Option<Interest>) {
        if self.current_interests == new_interests {
            return;
        }

        let mut source = SourceFd(&self.fd);
        match (self.current_interests, new_interests) {
            (Some(_), Some(new)) => {
                self.mio_registry
                    .reregister(&mut source, self.token, new)
                    .ok();
            }
            (Some(_), None) => {
                self.mio_registry.deregister(&mut source).ok();
            }
            (None, Some(new)) => {
                self.mio_registry
                    .register(&mut source, self.token, new)
                    .ok();
            }
            (None, None) => {}
        }
        self.current_interests = new_interests;
    }

    /// Adds an interest to the current set.
    fn add_interest(&mut self, interest: Interest) {
        let new = match self.current_interests {
            Some(cur) => Some(cur | interest),
            None => Some(interest),
        };
        self.update_interests(new);
    }

    /// Removes an interest from the current set.
    fn remove_interest(&mut self, interest: Interest) {
        let cur = match self.current_interests {
            Some(c) => c,
            None => return,
        };

        let new = if interest == Interest::READABLE {
            if cur.is_writable() {
                Some(Interest::WRITABLE)
            } else {
                None
            }
        } else if interest == Interest::WRITABLE {
            if cur.is_readable() {
                Some(Interest::READABLE)
            } else {
                None
            }
        } else {
            Some(cur)
        };

        self.update_interests(new);
    }

    /// Flushes incoming data from the peer to the local file descriptor.
    /// Ported from `local_socket_flush_incoming` in `original/sockets.cpp`.
    fn flush_incoming(&mut self) -> FlushResult {
        let mut bytes_flushed = 0;
        if !self.packet_queue.is_empty() {
            let data = self.packet_queue.coalesce();
            match nix::unistd::write(self.fd, &data) {
                Ok(n) => {
                    bytes_flushed = n as u32;
                    self.packet_queue.drop_front(n);
                }
                Err(e) if e == nix::errno::Errno::EAGAIN => {
                    // fd full
                }
                Err(_) => {
                    self.has_write_error = true;
                }
            }
        }

        if let (Some(transport), Some(peer)) = (&self.transport, &self.peer) {
            if let Some(peer) = peer.upgrade() {
                if self.available_send_bytes.is_some() {
                    transport.send_ready(self.id, peer.id(), bytes_flushed);
                } else {
                    if bytes_flushed != 0 && self.packet_queue.size() < MAX_PAYLOAD {
                        transport.send_ready(self.id, peer.id(), 0);
                    }
                }
            }
        }

        let fd_full = !self.packet_queue.is_empty() && !self.has_write_error;
        if self.closing && !fd_full {
            self.destroy();
            return FlushResult::Destroyed;
        }

        if fd_full {
            self.add_interest(Interest::WRITABLE);
            FlushResult::TryAgain
        } else {
            self.remove_interest(Interest::WRITABLE);
            FlushResult::Completed
        }
    }

    /// Destroys the socket and its file descriptor.
    /// Ported from `local_socket_destroy` in `original/sockets.cpp`.
    fn destroy(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.remove(self.id);
        }
        self.update_interests(None);
        let _ = nix::unistd::close(self.fd);
    }
}

impl FdeventHandler for LocalSocket {
    /// Handles events from the `fdevent` looper.
    /// Ported from `local_socket_event_func` in `original/sockets.cpp`.
    fn on_event(&mut self, event: &Event, _registry: &mio::Registry) {
        if event.is_writable() {
            let mut inner = self.inner.lock().unwrap();
            inner.flush_incoming();
        }
        if event.is_readable() {
            let (bytes_to_enqueue, is_eof) = {
                let mut inner = self.inner.lock().unwrap();
                match nix::unistd::read(inner.fd, &mut inner.read_buffer) {
                    Ok(0) => (None, true),
                    Ok(n) => (Some(Bytes::copy_from_slice(&inner.read_buffer[..n])), false),
                    Err(e) if e == nix::errno::Errno::EAGAIN => (None, false),
                    Err(_) => (None, true),
                }
            };

            if let Some(bytes) = bytes_to_enqueue {
                let peer = {
                    let inner = self.inner.lock().unwrap();
                    inner.peer.as_ref().and_then(|p| p.upgrade())
                };
                if let Some(peer) = peer {
                    let r = peer.enqueue(bytes);
                    if r > 0 {
                        // Peer is full, stop reading.
                        let mut inner = self.inner.lock().unwrap();
                        inner.remove_interest(Interest::READABLE);
                    } else if r < 0 {
                        // Peer closed us.
                        return;
                    }
                }
            }
            if is_eof {
                self.close();
            }
        }
    }

    fn on_timeout(&mut self) {}
}

/// A remote socket bound to a transport.
/// Ported from `asocket` with remote socket fields in `original/socket.h`.
pub struct RemoteSocket {
    id: u32,
    inner: Mutex<RemoteSocketInner>,
}

/// Inner state of a [`RemoteSocket`].
struct RemoteSocketInner {
    peer: Option<Weak<dyn Socket>>,
    transport: Arc<dyn Transport>,
    registry: Weak<SocketRegistry>,
}

impl RemoteSocket {
    /// Creates a new `RemoteSocket`.
    pub fn new(id: u32, transport: Arc<dyn Transport>, registry: Arc<SocketRegistry>) -> Self {
        Self {
            id,
            inner: Mutex::new(RemoteSocketInner {
                peer: None,
                transport,
                registry: Arc::downgrade(&registry),
            }),
        }
    }

    /// Sets the peer socket.
    pub fn set_peer(&self, peer: Arc<dyn Socket>) {
        let mut inner = self.inner.lock().unwrap();
        inner.peer = Some(Arc::downgrade(&peer));
    }
}

impl Socket for RemoteSocket {
    fn id(&self) -> u32 {
        self.id
    }

    fn enqueue(&self, data: Bytes) -> i32 {
        let inner = self.inner.lock().unwrap();
        let mut p = Apacket::default();
        p.msg.command = A_WRTE;
        if let Some(peer) = inner.peer.as_ref().and_then(|p| p.upgrade()) {
            p.msg.arg0 = peer.id();
        }
        p.msg.arg1 = self.id;
        p.msg.data_length = data.len() as u32;
        // Use Bytes directly to avoid extra copy.
        p.payload = Block(std::io::Cursor::new(data.to_vec())); // Block requires Cursor<Vec<u8>> currently.
        inner.transport.send_packet(p);
        1
    }

    fn ready(&self) {
        let inner = self.inner.lock().unwrap();
        let mut p = Apacket::default();
        p.msg.command = A_OKAY;
        if let Some(peer) = inner.peer.as_ref().and_then(|p| p.upgrade()) {
            p.msg.arg0 = peer.id();
        }
        p.msg.arg1 = self.id;
        inner.transport.send_packet(p);
    }

    fn shutdown(&self) {
        let inner = self.inner.lock().unwrap();
        let mut p = Apacket::default();
        p.msg.command = A_CLSE;
        if let Some(peer) = inner.peer.as_ref().and_then(|p| p.upgrade()) {
            p.msg.arg0 = peer.id();
        }
        p.msg.arg1 = self.id;
        inner.transport.send_packet(p);
    }

    fn close(&self) {
        let peer = {
            let mut inner = self.inner.lock().unwrap();
            inner.peer.take().and_then(|p| p.upgrade())
        };
        if let Some(peer) = peer {
            peer.close();
        }
        let inner = self.inner.lock().unwrap();
        if let Some(registry) = inner.registry.upgrade() {
            registry.remove(self.id);
        }
    }

    fn peer_id(&self) -> Option<u32> {
        let inner = self.inner.lock().unwrap();
        inner
            .peer
            .as_ref()
            .and_then(|p| p.upgrade())
            .map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        Some(inner.transport.id())
    }
}

/// Creates a new local socket and registers it with the `fdevent` looper.
/// Ported from `create_local_socket` in `original/sockets.cpp`.
pub fn create_local_socket(
    fd: RawFd,
    registry: Arc<SocketRegistry>,
    fdevent: &mut Fdevent,
) -> Arc<LocalSocket> {
    let id = registry.alloc_id();
    let mio_registry = fdevent.registry();
    let socket = LocalSocket::new(id, fd, registry.clone(), mio_registry, Token(0));
    let socket_arc = Arc::new(socket.clone());

    // SAFETY: fd is a valid file descriptor.
    let borrowed_fd = unsafe { std::os::unix::io::BorrowedFd::borrow_raw(fd) };
    let token = fdevent
        .register(&borrowed_fd, Box::new(socket), Interest::READABLE)
        .unwrap();
    socket_arc.inner.lock().unwrap().token = token;

    registry.install(socket_arc.clone());
    socket_arc
}

/// Creates a new remote socket.
/// Ported from `create_remote_socket` in `original/sockets.cpp`.
pub fn create_remote_socket(
    id: u32,
    transport: Arc<dyn Transport>,
    registry: Arc<SocketRegistry>,
) -> Arc<RemoteSocket> {
    let socket = Arc::new(RemoteSocket::new(id, transport, registry.clone()));
    registry.install(socket.clone());
    socket
}

/// Utility functions for ADB socket management.
pub mod internal {
    /// Parses a host service string of the format `[prefix:]serial:command`.
    /// Ported from `internal::parse_host_service` in `original/sockets.cpp`.
    pub fn parse_host_service<'a>(full_service: &'a str) -> Option<(&'a str, &'a str)> {
        if full_service.is_empty() {
            return None;
        }

        let command = full_service;

        let prefixes = ["usb:", "product:", "model:", "device:", "localfilesystem:"];
        for prefix in prefixes {
            if command.starts_with(prefix) {
                if let Some(offset) = command[prefix.len()..].find(':') {
                    let total_prefix_len = prefix.len() + offset + 1;
                    let serial = &full_service[..total_prefix_len - 1];
                    let command = &full_service[total_prefix_len..];
                    return Some((serial, command));
                }
            }
        }

        let mut command_to_parse = command;
        if command_to_parse.starts_with("tcp:") || command_to_parse.starts_with("udp:") {
            command_to_parse = &command_to_parse[4..];
        }

        if command_to_parse.is_empty() {
            return None;
        }

        let mut found_address = false;
        let mut offset = 0;
        if command_to_parse.starts_with('[') {
            if let Some(ipv6_end) = command_to_parse.find(']') {
                if command_to_parse.len() > ipv6_end + 1
                    && &command_to_parse[ipv6_end + 1..ipv6_end + 2] == ":"
                {
                    offset = ipv6_end + 2;
                    found_address = true;
                }
            }
        }

        if !found_address {
            if let Some(colon_offset) = command_to_parse.find(':') {
                offset = colon_offset + 1;
            } else {
                return None;
            }
        }

        // serial is everything up to the colon
        let serial_end = (full_service.len() - command_to_parse.len()) + offset - 1;
        let mut serial = &full_service[..serial_end];
        let mut command = &full_service[serial_end + 1..];

        // Check for port
        if let Some(next_colon) = command.find(':') {
            let port = &command[..next_colon];
            if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
                let total_serial_len = serial_end + 1 + next_colon;
                serial = &full_service[..total_serial_len];
                command = &full_service[total_serial_len + 1..];
            }
        }

        if serial.is_empty() || command.is_empty() {
            None
        } else {
            Some((serial, command))
        }
    }
}
