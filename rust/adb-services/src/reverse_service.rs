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
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(unix)]
use libc;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, AsRawSocket, OwnedSocket, RawHandle, RawSocket};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use sysdeps::AdbFd;

struct ReverseForward {
    local: String,
    token: Token,
}

static REVERSE_FORWARDS: OnceLock<Mutex<HashMap<u64, HashMap<String, ReverseForward>>>> =
    OnceLock::new();

fn reverse_forwards() -> &'static Mutex<HashMap<u64, HashMap<String, ReverseForward>>> {
    REVERSE_FORWARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn reverse_service(fd: AdbFd, command: &str, transport: Arc<ATransport>) {
    let mut file = fd;
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
                    let listen_fd_arc = Arc::new(AdbFd::from(listen_fd));
                    let handler = Box::new(ReverseListener {
                        fd: listen_fd_arc.clone(),
                        local: local.clone(),
                        transport: Arc::downgrade(&transport),
                    });
                    match fdevent.register(listen_fd_arc, handler, Interest::READABLE) {
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
    fd: Arc<AdbFd>,
    local: String,
    transport: Weak<ATransport>,
}

impl FdeventHandler for ReverseListener {
    fn on_event(&mut self, event: &mio::event::Event, fdevent: &mut Fdevent) {
        if event.is_readable() {
            let client_fd = match () {
                #[cfg(unix)]
                () => {
                    let res = unsafe {
                        libc::accept(self.fd.as_raw_fd(), std::ptr::null_mut(), std::ptr::null_mut())
                    };
                    if res >= 0 {
                        Some(AdbFd::from(unsafe {
                            std::os::unix::io::OwnedFd::from_raw_fd(res)
                        }))
                    } else {
                        None
                    }
                }
                #[cfg(windows)]
                () => {
                    let res = unsafe {
                        windows_sys::Win32::Networking::WinSock::accept(
                            self.fd.as_raw_socket() as _,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    };
                    if res != windows_sys::Win32::Networking::WinSock::INVALID_SOCKET as _ {
                        Some(AdbFd::from(unsafe {
                            std::os::windows::io::OwnedSocket::from_raw_socket(res as _)
                        }))
                    } else {
                        None
                    }
                }
            };

            if let Some(client_fd) = client_fd {
                if let Some(transport) = self.transport.upgrade() {
                    let registry = transport
                        .registry
                        .lock()
                        .unwrap()
                        .as_ref()
                        .expect("transport missing registry")
                        .clone();
                    let local_socket = create_local_socket(client_fd, registry, fdevent);
                    connect_to_remote(&local_socket, &self.local);
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
