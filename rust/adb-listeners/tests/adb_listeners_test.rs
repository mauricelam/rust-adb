//! Tests for adb listeners

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

use adb_listeners::*;
use adb_protocol::{ConnectionState, TransportType};
use adb_sockets::SocketRegistry;
use adb_transport::{register_transport, ATransport, ReconnectResult};
use fdevent::fdevent::Fdevent;
use std::sync::Arc;

fn listener_is_installed(serial: &str, source: &str, dest: &str) -> bool {
    let listeners = format_listeners();
    for line in listeners.lines() {
        let info: Vec<&str> = line.split_whitespace().collect();
        if info.len() == 3 &&
            (serial.is_empty() || info[0] == serial) &&
            (source.is_empty() || info[1] == source) &&
            (dest.is_empty() || info[2] == dest) {
            return true;
        }
    }
    false
}

fn setup() -> (Fdevent, Arc<SocketRegistry>, Arc<ATransport>) {
    let fdevent = Fdevent::new().unwrap();
    let registry = Arc::new(SocketRegistry::new());
    let transport = Arc::new(ATransport::new(
        TransportType::Local,
        Box::new(|_| ReconnectResult::Abort),
        ConnectionState::Device,
    ));
    *transport.serial.lock().unwrap() = "test_serial".to_string();
    *transport.fdevent.lock().unwrap() = Some(Arc::new(std::sync::Mutex::new(Fdevent::new().unwrap()))); // Dummy for disconnect handler
    register_transport(transport.clone());
    (fdevent, registry, transport)
}

#[test]
fn test_install_listener() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry).is_ok());
    assert!(listener_is_installed("test_serial", "tcp:9000", "tcp:9000"));

    remove_all_listeners();
}

#[test]
fn test_install_listener_rebind() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry.clone()).is_ok());
    assert!(install_listener("tcp:9000", "tcp:9001", Some(transport.clone()), 0, &mut fdevent, registry).is_ok());

    assert!(listener_is_installed("test_serial", "tcp:9000", "tcp:9001"));

    remove_all_listeners();
}

#[test]
fn test_install_listener_no_rebind() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), INSTALL_LISTENER_NO_REBIND, &mut fdevent, registry.clone()).is_ok());

    let res = install_listener("tcp:9000", "tcp:9001", Some(transport.clone()), INSTALL_LISTENER_NO_REBIND, &mut fdevent, registry);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().0, InstallStatus::CannotRebind);

    assert!(listener_is_installed("test_serial", "tcp:9000", "tcp:9000"));

    remove_all_listeners();
}

#[test]
fn test_install_listener_tcp_port_0() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    let res = install_listener("tcp:0", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry);
    assert!(res.is_ok());
    let port = res.unwrap().unwrap();
    assert!(port > 0);

    assert!(listener_is_installed("test_serial", &format!("tcp:{}", port), "tcp:9000"));

    remove_all_listeners();
}

#[test]
fn test_remove_listener() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry).is_ok());
    assert!(remove_listener("tcp:9000", Some(&transport)).is_ok());
    assert!(format_listeners().is_empty());
}

#[test]
fn test_remove_nonexistent_listener() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry).is_ok());
    let res = remove_listener("tcp:1", Some(&transport));
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), InstallStatus::ListenerNotFound);
    assert!(listener_is_installed("test_serial", "tcp:9000", "tcp:9000"));

    remove_all_listeners();
}

#[test]
fn test_remove_all_listeners() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry.clone()).is_ok());
    assert!(install_listener("tcp:9001", "tcp:9001", Some(transport.clone()), 0, &mut fdevent, registry).is_ok());

    remove_all_listeners();
    assert!(format_listeners().is_empty());
}

#[test]
fn test_transport_disconnect() {
    let (mut fdevent, registry, transport) = setup();
    remove_all_listeners();

    assert!(install_listener("tcp:9000", "tcp:9000", Some(transport.clone()), 0, &mut fdevent, registry.clone()).is_ok());
    assert!(install_listener("tcp:9001", "tcp:9001", Some(transport.clone()), 0, &mut fdevent, registry).is_ok());

    transport.run_disconnects();
    assert!(format_listeners().is_empty());
}
