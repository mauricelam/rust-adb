use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::os::unix::io::RawFd;
use bytes::Bytes;
use adb_types::{Apacket, IoVector};
use mio::{Interest, Token, event::Event, unix::SourceFd};
use fdevent::fdevent::{Fdevent, FdeventHandler};

pub const MAX_PAYLOAD: usize = 1024 * 1024;

pub const A_SYNC: u32 = 0x434e5953;
pub const A_CNXN: u32 = 0x4e584e43;
pub const A_OPEN: u32 = 0x4e45504f;
pub const A_OKAY: u32 = 0x59414b4f;
pub const A_CLSE: u32 = 0x45534c43;
pub const A_WRTE: u32 = 0x45545257;
pub const A_AUTH: u32 = 0x48545541;
pub const A_STLS: u32 = 0x534C5453;

pub trait Socket: Send + Sync {
    fn id(&self) -> u32;
    fn enqueue(&self, data: Bytes) -> i32;
    fn ready(&self);
    fn shutdown(&self);
    fn close(&self);
    fn peer_id(&self) -> Option<u32>;
    fn transport_id(&self) -> Option<u64>;
}

pub trait Transport: Send + Sync {
    fn id(&self) -> u64;
    fn send_packet(&self, packet: Apacket);
    fn send_ready(&self, local: u32, remote: u32, ack_bytes: u32);
    fn get_max_payload(&self) -> usize;
    fn supports_delayed_ack(&self) -> bool;
}

struct SocketRegistryInner {
    sockets: HashMap<u32, Arc<dyn Socket>>,
    closing_sockets: HashMap<u32, Arc<dyn Socket>>,
    next_id: u32,
}

pub struct SocketRegistry {
    inner: Mutex<SocketRegistryInner>,
}

impl SocketRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SocketRegistryInner {
                sockets: HashMap::new(),
                closing_sockets: HashMap::new(),
                next_id: 1,
            }),
        }
    }

    pub fn alloc_id(&self) -> u32 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id = inner.next_id.checked_add(1).expect("Socket ID overflow");
        id
    }

    pub fn install(&self, socket: Arc<dyn Socket>) {
        let mut inner = self.inner.lock().unwrap();
        inner.sockets.insert(socket.id(), socket);
    }

    pub fn find(&self, id: u32) -> Option<Arc<dyn Socket>> {
        let inner = self.inner.lock().unwrap();
        inner.sockets.get(&id).cloned()
    }

    pub fn find_local_socket(&self, local_id: u32, peer_id: u32) -> Option<Arc<dyn Socket>> {
        let inner = self.inner.lock().unwrap();
        if let Some(s) = inner.sockets.get(&local_id) {
            if peer_id == 0 || s.peer_id() == Some(peer_id) {
                return Some(s.clone());
            }
        }
        None
    }

    pub fn remove(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        inner.sockets.remove(&id);
        inner.closing_sockets.remove(&id);
    }

    pub fn move_to_closing(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(s) = inner.sockets.remove(&id) {
            inner.closing_sockets.insert(id, s);
        }
    }

    pub fn close_all_sockets(&self, transport_id: u64) {
        let ids: Vec<u32> = {
            let inner = self.inner.lock().unwrap();
            inner.sockets.values()
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

#[derive(Clone)]
pub struct LocalSocket {
    inner: Arc<Mutex<LocalSocketInner>>,
}

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
    interests: Interest,
    available_send_bytes: Option<i64>,
}

impl LocalSocket {
    pub fn new(id: u32, fd: RawFd, registry: Arc<SocketRegistry>, mio_registry: mio::Registry, token: Token) -> Self {
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
                interests: Interest::READABLE,
                available_send_bytes: None,
            })),
        }
    }

    pub fn set_peer(&self, peer: Arc<dyn Socket>) {
        let mut inner = self.inner.lock().unwrap();
        inner.peer = Some(Arc::downgrade(&peer));
    }

    pub fn set_transport(&self, transport: Arc<dyn Transport>) {
        let mut inner = self.inner.lock().unwrap();
        inner.transport = Some(transport);
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
            inner.remove_interest(Interest::READABLE);
            inner.add_interest(Interest::WRITABLE);
            if let Some(registry) = inner.registry.upgrade() {
                registry.move_to_closing(inner.id);
            }
        }
    }

    fn peer_id(&self) -> Option<u32> {
        let inner = self.inner.lock().unwrap();
        inner.peer.as_ref().and_then(|p| p.upgrade()).map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        inner.transport.as_ref().map(|t| t.id())
    }
}

enum FlushResult {
    Destroyed,
    TryAgain,
    Completed,
}

impl LocalSocketInner {
    fn add_interest(&mut self, interest: Interest) {
        let new_interests = self.interests | interest;
        if new_interests != self.interests {
            self.interests = new_interests;
            let mut source = SourceFd(&self.fd);
            self.mio_registry.reregister(&mut source, self.token, self.interests).ok();
        }
    }

    fn remove_interest(&mut self, interest: Interest) {
        // Since Interest doesn't support subtraction, we have to rebuild.
        let mut new_interest = None;
        if interest == Interest::READABLE {
            if self.interests == (Interest::READABLE | Interest::WRITABLE) {
                new_interest = Some(Interest::WRITABLE);
            } else if self.interests == Interest::READABLE {
                // Cannot have empty interest in mio, but we can't easily avoid it here
                // without more complex logic.
            }
        } else if interest == Interest::WRITABLE {
            if self.interests == (Interest::READABLE | Interest::WRITABLE) {
                new_interest = Some(Interest::READABLE);
            } else if self.interests == Interest::WRITABLE {
                new_interest = Some(Interest::READABLE); // Fallback to readable?
            }
        }

        if let Some(i) = new_interest {
            if i != self.interests {
                self.interests = i;
                let mut source = SourceFd(&self.fd);
                self.mio_registry.reregister(&mut source, self.token, self.interests).ok();
            }
        }
    }

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

    fn destroy(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.remove(self.id);
        }
        let _ = nix::unistd::close(self.fd);
    }
}

impl FdeventHandler for LocalSocket {
    fn on_event(&mut self, event: &Event) {
        if event.is_writable() {
            let mut inner = self.inner.lock().unwrap();
            inner.flush_incoming();
        }
        if event.is_readable() {
            let (data, is_eof) = {
                let inner = self.inner.lock().unwrap();
                let max_payload = MAX_PAYLOAD;
                let mut buf = vec![0u8; max_payload];
                match nix::unistd::read(inner.fd, &mut buf) {
                    Ok(0) => (None, true),
                    Ok(n) => (Some(Bytes::copy_from_slice(&buf[..n])), false),
                    Err(e) if e == nix::errno::Errno::EAGAIN => (None, false),
                    Err(_) => (None, true),
                }
            };

            if let Some(bytes) = data {
                let peer = {
                    let inner = self.inner.lock().unwrap();
                    inner.peer.as_ref().and_then(|p| p.upgrade())
                };
                if let Some(peer) = peer {
                    let r = peer.enqueue(bytes);
                    if r > 0 {
                        let mut inner = self.inner.lock().unwrap();
                        inner.remove_interest(Interest::READABLE);
                        inner.add_interest(Interest::WRITABLE);
                    } else if r < 0 {
                        // Peer closed us
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

pub struct RemoteSocket {
    id: u32,
    inner: Mutex<RemoteSocketInner>,
}

struct RemoteSocketInner {
    peer: Option<Weak<dyn Socket>>,
    transport: Arc<dyn Transport>,
    registry: Weak<SocketRegistry>,
}

impl RemoteSocket {
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

    pub fn set_peer(&self, peer: Arc<dyn Socket>) {
        let mut inner = self.inner.lock().unwrap();
        inner.peer = Some(Arc::downgrade(&peer));
    }
}

impl Socket for RemoteSocket {
    fn id(&self) -> u32 { self.id }

    fn enqueue(&self, data: Bytes) -> i32 {
        let inner = self.inner.lock().unwrap();
        let mut p = Apacket::default();
        p.msg.command = A_WRTE;
        if let Some(peer) = inner.peer.as_ref().and_then(|p| p.upgrade()) {
            p.msg.arg0 = peer.id();
        }
        p.msg.arg1 = self.id;
        p.msg.data_length = data.len() as u32;
        p.payload = std::io::Cursor::new(data.to_vec());
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
        inner.peer.as_ref().and_then(|p| p.upgrade()).map(|p| p.id())
    }

    fn transport_id(&self) -> Option<u64> {
        let inner = self.inner.lock().unwrap();
        Some(inner.transport.id())
    }
}

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
    let token = fdevent.register(&borrowed_fd, Box::new(socket), Interest::READABLE).unwrap();
    socket_arc.inner.lock().unwrap().token = token;

    registry.install(socket_arc.clone());
    socket_arc
}

pub fn create_remote_socket(
    id: u32,
    transport: Arc<dyn Transport>,
    registry: Arc<SocketRegistry>,
) -> Arc<RemoteSocket> {
    let socket = Arc::new(RemoteSocket::new(id, transport, registry.clone()));
    registry.install(socket.clone());
    socket
}

pub mod internal {
    pub fn parse_host_service<'a>(
        full_service: &'a str,
    ) -> Option<(&'a str, &'a str)> {
        if full_service.is_empty() {
            return None;
        }

        let mut _serial = "";
        let mut command = full_service;

        let prefixes = ["usb:", "product:", "model:", "device:", "localfilesystem:"];
        for prefix in prefixes {
            if command.starts_with(prefix) {
                if let Some(offset) = command[prefix.len()..].find(':') {
                    let total_prefix_len = prefix.len() + offset + 1;
                    let serial = &full_service[..total_prefix_len - 1];
                    command = &full_service[total_prefix_len..];
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
                if command_to_parse.len() > ipv6_end + 1 && &command_to_parse[ipv6_end+1..ipv6_end+2] == ":" {
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
        command = &full_service[serial_end + 1..];

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
