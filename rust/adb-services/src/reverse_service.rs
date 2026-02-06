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

use adb_io::{send_fail, send_okay, send_protocol_string};
use adb_socket_spec::socket_spec_listen;
use adb_sockets::{connect_to_remote, create_local_socket};
use adb_transport::{ATransport, DisconnectHandler};
use fdevent::fdevent::{Fdevent, FdeventHandler};
use mio::{Interest, Token};
use std::collections::HashMap;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::{Arc, Mutex, OnceLock, Weak};

struct ReverseForward {
    local: String,
    token: Token,
}

static REVERSE_FORWARDS: OnceLock<Mutex<HashMap<u64, HashMap<String, ReverseForward>>>> =
    OnceLock::new();

fn reverse_forwards() -> &'static Mutex<HashMap<u64, HashMap<String, ReverseForward>>> {
    REVERSE_FORWARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn reverse_service(fd: OwnedFd, command: &str, transport: Arc<ATransport>) {
    let mut file = std::fs::File::from(fd);
    let tid = transport.id;

    if let Some(args) = command.strip_prefix("forward:") {
        let parts: Vec<&str> = args.split(';').collect();
        if parts.len() != 2 {
            let _ = send_fail(&mut file, "invalid reverse forward command");
            return;
        }
        let remote = parts[0].to_string();
        let local = parts[1].to_string();

        match socket_spec_listen(&remote, None) {
            Ok(listen_fd) => {
                let fdevent_lock = transport.fdevent.lock().unwrap();
                if let Some(ref fdevent_arc) = *fdevent_lock {
                    let mut fdevent = fdevent_arc.lock().unwrap();
                    let handler = Box::new(ReverseListener {
                        fd: listen_fd.as_raw_fd(),
                        local: local.clone(),
                        transport: Arc::downgrade(&transport),
                    });
                    match fdevent.register(Arc::new(listen_fd), handler, Interest::READABLE) {
                        Ok(token) => {
                            let mut forwards_lock = reverse_forwards().lock().unwrap();
                            let transport_forwards =
                                forwards_lock.entry(tid).or_insert_with(HashMap::new);
                            if transport_forwards.is_empty() {
                                transport.add_disconnect(Box::new(ReverseDisconnectHandler { tid }));
                            }
                            transport_forwards.insert(remote, ReverseForward { local, token });
                            let _ = send_okay(&mut file);
                        }
                        Err(e) => {
                            let _ =
                                send_fail(&mut file, &format!("failed to register listener: {}", e));
                        }
                    }
                } else {
                    let _ = send_fail(&mut file, "no fdevent available");
                }
            }
            Err(e) => {
                let _ = send_fail(&mut file, &format!("failed to listen on {}: {}", remote, e));
            }
        }
    } else if let Some(remote) = command.strip_prefix("killforward:") {
        let mut forwards_lock = reverse_forwards().lock().unwrap();
        if let Some(transport_forwards) = forwards_lock.get_mut(&tid) {
            if let Some(forward) = transport_forwards.remove(remote) {
                let fdevent_lock = transport.fdevent.lock().unwrap();
                if let Some(ref fdevent_arc) = *fdevent_lock {
                    let mut fdevent = fdevent_arc.lock().unwrap();
                    let _ = fdevent.unregister(forward.token);
                }
                let _ = send_okay(&mut file);
            } else {
                let _ = send_fail(&mut file, &format!("reverse forward not found: {}", remote));
            }
        } else {
            let _ = send_fail(&mut file, &format!("no reverse forwards for transport {}", tid));
        }
    } else if command == "killforward-all" {
        let mut forwards_lock = reverse_forwards().lock().unwrap();
        if let Some(transport_forwards) = forwards_lock.remove(&tid) {
            let fdevent_lock = transport.fdevent.lock().unwrap();
            if let Some(ref fdevent_arc) = *fdevent_lock {
                let mut fdevent = fdevent_arc.lock().unwrap();
                for (_, forward) in transport_forwards {
                    let _ = fdevent.unregister(forward.token);
                }
            }
        }
        let _ = send_okay(&mut file);
    } else if command == "list-forward" {
        let forwards_lock = reverse_forwards().lock().unwrap();
        let mut response = String::new();
        if let Some(transport_forwards) = forwards_lock.get(&tid) {
            for (remote, forward) in transport_forwards {
                response.push_str(&format!(
                    "{} {} {}\n",
                    transport.serial.lock().unwrap(),
                    remote,
                    forward.local
                ));
            }
        }
        let _ = send_protocol_string(&mut file, &response);
    } else {
        let _ = send_fail(&mut file, &format!("unknown reverse command {}", command));
    }
}

struct ReverseListener {
    fd: RawFd,
    local: String,
    transport: Weak<ATransport>,
}

impl FdeventHandler for ReverseListener {
    fn on_event(&mut self, event: &mio::event::Event, fdevent: &mut Fdevent) {
        if event.is_readable() {
            unsafe {
                let mut addr: libc::sockaddr_storage = std::mem::zeroed();
                let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
                let client_fd =
                    libc::accept(self.fd, &mut addr as *mut _ as *mut libc::sockaddr, &mut len);
                if client_fd >= 0 {
                    if let Some(transport) = self.transport.upgrade() {
                        let registry = transport
                            .registry
                            .lock()
                            .unwrap()
                            .as_ref()
                            .expect("transport missing registry")
                            .clone();
                        let local_socket = create_local_socket(OwnedFd::from_raw_fd(client_fd), registry, fdevent);
                        connect_to_remote(&local_socket, &self.local);
                    } else {
                        libc::close(client_fd);
                    }
                }
            }
        }
    }
    fn on_timeout(&mut self, _fdevent: &mut Fdevent) {}
}

struct ReverseDisconnectHandler {
    tid: u64,
}

impl DisconnectHandler for ReverseDisconnectHandler {
    fn on_disconnect(&self, transport: &ATransport) {
        let mut forwards_lock = reverse_forwards().lock().unwrap();
        if let Some(transport_forwards) = forwards_lock.remove(&self.tid) {
            let fdevent_lock = transport.fdevent.lock().unwrap();
            if let Some(ref fdevent_arc) = *fdevent_lock {
                let mut fdevent = fdevent_arc.lock().unwrap();
                for (_, forward) in transport_forwards {
                    let _ = fdevent.unregister(forward.token);
                }
            }
        }
    }
}
