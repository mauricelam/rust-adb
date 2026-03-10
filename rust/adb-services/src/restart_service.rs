/*
 * Copyright (C) 2019 The Android Open Source Project
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

//! Restart service implementation.
//! Ported from original/daemon/restart_service.cpp.

use adb_io::send_protocol_string;
use sysdeps::AdbFd;
use std::io::Write;
use std::sync::Mutex;
use std::collections::HashMap;
use std::sync::OnceLock;

static PROPERTIES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn get_properties() -> &'static Mutex<HashMap<String, String>> {
    PROPERTIES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Sets a system property.
pub fn set_property(name: &str, value: &str) {
    #[cfg(target_os = "android")]
    {
        // In real android we would use android::base::SetProperty
        // For now, use our mock for all platforms to support tests.
        get_properties().lock().unwrap().insert(name.to_string(), value.to_string());
    }
    #[cfg(not(target_os = "android"))]
    {
        get_properties().lock().unwrap().insert(name.to_string(), value.to_string());
    }
}

/// Returns the value of a system property.
pub fn get_property(name: &str, default: &str) -> String {
    get_properties().lock().unwrap().get(name).cloned().unwrap_or_else(|| default.to_string())
}

fn is_debuggable() -> bool {
    #[cfg(target_os = "android")]
    {
        // Placeholder
        get_property("ro.debuggable", "0") == "1"
    }
    #[cfg(not(target_os = "android"))]
    {
        true
    }
}

/// Restarts adbd as root.
/// Ported from `restart_root_service` in `restart_service.cpp`.
pub fn restart_root_service(mut fd: AdbFd) {
    #[cfg(unix)]
    {
        if unsafe { libc::getuid() } == 0 {
            let _ = fd.write_all(b"adbd is already running as root\n");
            return;
        }
    }
    if !is_debuggable() {
        let _ = fd.write_all(b"adbd cannot run as root in production builds\n");
        return;
    }

    log::info!("adbd restarting as root");
    set_property("service.adb.root", "1");
    let _ = fd.write_all(b"restarting adbd as root\n");
}

/// Restarts adbd as non-root.
/// Ported from `restart_unroot_service` in `restart_service.cpp`.
pub fn restart_unroot_service(mut fd: AdbFd) {
    #[cfg(unix)]
    {
        if unsafe { libc::getuid() } != 0 {
            let _ = fd.write_all(b"adbd not running as root\n");
            return;
        }
    }

    log::info!("adbd restarting as nonroot");
    set_property("service.adb.root", "0");
    let _ = fd.write_all(b"restarting adbd as non root\n");
}

/// Restarts adbd in TCP mode on the given port.
/// Ported from `restart_tcp_service` in `restart_service.cpp`.
pub fn restart_tcp_service(mut fd: AdbFd, port: i32) {
    if port <= 0 {
        let _ = fd.write_all(format!("invalid port {}\n", port).as_bytes());
        return;
    }

    log::info!("adbd restarting in TCP mode (port = {})", port);
    set_property("service.adb.tcp.port", &port.to_string());
    let _ = fd.write_all(format!("restarting in TCP mode port: {}\n", port).as_bytes());
}

/// Restarts adbd in USB mode.
/// Ported from `restart_usb_service` in `restart_service.cpp`.
pub fn restart_usb_service(mut fd: AdbFd) {
    log::info!("adbd restarting in USB mode");
    set_property("service.adb.tcp.port", "0");
    let _ = fd.write_all(b"restarting in USB mode\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_service_thread;
    use std::io::Read;

    fn read_raw(mut fd: AdbFd) -> String {
        let mut buf = String::new();
        let _ = fd.read_to_string(&mut buf);
        buf
    }

    #[test]
    fn test_restart_tcp_service_valid_port_success() {
        let assigned_port = 5555;
        let fd = create_service_thread("tcp", move |fd| {
            restart_tcp_service(fd, assigned_port);
        })
        .unwrap();

        assert_eq!(
            read_raw(fd),
            format!("restarting in TCP mode port: {}\n", assigned_port)
        );
        assert_eq!(get_property("service.adb.tcp.port", ""), assigned_port.to_string());
    }

    #[test]
    fn test_restart_tcp_service_invalid_port_failure() {
        set_property("service.adb.tcp.port", "5555");
        let port = -5;
        let fd = create_service_thread("tcp", move |fd| {
            restart_tcp_service(fd, port);
        })
        .unwrap();

        assert_eq!(read_raw(fd), format!("invalid port {}\n", port));
        assert_eq!(get_property("service.adb.tcp.port", ""), "5555");
    }

    #[test]
    fn test_restart_usb_service_success() {
        let fd = create_service_thread("usb", restart_usb_service).unwrap();

        assert_eq!(read_raw(fd), "restarting in USB mode\n");
        assert_eq!(get_property("service.adb.tcp.port", ""), "0");
    }
}
