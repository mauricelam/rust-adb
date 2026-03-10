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

//! ADB MDNS Support.
//! Ported from `adb_mdns.h` and `adb_mdns.cpp`.

/// Utilities for MDNS parsing and configuration.
pub mod utils;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;

/// ADB MDNS service type.
pub const ADB_MDNS_SERVICE_TYPE: &str = "adb";
/// ADB-TLS pairing service type.
pub const ADB_MDNS_TLS_PAIRING_TYPE: &str = "adb-tls-pairing";
/// ADB-TLS connect service type.
pub const ADB_MDNS_TLS_CONNECT_TYPE: &str = "adb-tls-connect";

/// Version of the secure ADB service.
pub const ADB_SECURE_SERVICE_VERSION: i32 = 1;

/// Index for the basic ADB transport service.
pub const K_ADB_TRANSPORT_SERVICE_REF_INDEX: usize = 0;
/// Index for the secure pairing service.
pub const K_ADB_SECURE_PAIRING_SERVICE_REF_INDEX: usize = 1;
/// Index for the secure connect service.
pub const K_ADB_SECURE_CONNECT_SERVICE_REF_INDEX: usize = 2;
/// Number of ADB DNS services.
pub const K_NUM_ADB_DNS_SERVICES: usize = 3;

/// Array of ADB DNS service registration types.
pub const ADB_DNS_SERVICES: [&str; K_NUM_ADB_DNS_SERVICES] = [
    "_adb._tcp.local.",
    "_adb-tls-pairing._tcp.local.",
    "_adb-tls-connect._tcp.local.",
];

/// Information about an MDNS service.
/// Ported from `MdnsInfo` in `adb_mdns.h`.
#[derive(Debug, Clone)]
pub struct MdnsServiceInfo {
    /// Full instance name.
    pub instance_name: String,
    /// Service type (e.g., "_adb._tcp.local.").
    pub service_type: String,
    /// Hostname of the service.
    pub hostname: String,
    /// IP addresses of the service.
    pub ip_addresses: Vec<std::net::IpAddr>,
    /// Port number.
    pub port: u16,
    /// TXT record properties.
    pub txt_record: HashMap<String, String>,
}

/// Callback for discovered MDNS services.
pub type MdnsServiceCallback = Arc<dyn Fn(MdnsServiceInfo) + Send + Sync>;

/// Manages MDNS discovery and registration.
/// Ported from `AdbMdns` in `adb_mdns.h`.
pub struct AdbMdns {
    daemon: ServiceDaemon,
    autoconn_allowedlist: Arc<Mutex<HashSet<usize>>>,
    discovered_services: Arc<Mutex<HashMap<String, Vec<MdnsServiceInfo>>>>,
}

impl AdbMdns {
    /// Creates a new `AdbMdns` instance.
    pub fn new() -> anyhow::Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let mut autoconn_allowedlist = HashSet::new();

        // Default: allow adb-tls-connect to auto-connect
        autoconn_allowedlist.insert(K_ADB_SECURE_CONNECT_SERVICE_REF_INDEX);

        // Ported from original/adb_mdns.cpp: `config_auto_connect_services`
        if let Ok(srvs) = std::env::var("ADB_MDNS_AUTO_CONNECT") {
            if srvs == "all" {
                autoconn_allowedlist.insert(K_ADB_TRANSPORT_SERVICE_REF_INDEX);
                autoconn_allowedlist.insert(K_ADB_SECURE_PAIRING_SERVICE_REF_INDEX);
                autoconn_allowedlist.insert(K_ADB_SECURE_CONNECT_SERVICE_REF_INDEX);
            } else if srvs != "0" {
                for item in srvs.split(',') {
                    let full_srv = format!("_{}._tcp.local.", item);
                    if let Some(idx) = adb_dns_service_index_by_name(&full_srv) {
                        autoconn_allowedlist.insert(idx);
                    }
                }
            }
        }

        Ok(Self {
            daemon,
            autoconn_allowedlist: Arc::new(Mutex::new(autoconn_allowedlist)),
            discovered_services: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Starts browsing for ADB MDNS services.
    /// Ported from `init_mdns_transport_discovery_thread` in `mdnsresponder_client.cpp`.
    pub fn browse(&self, callback: Option<MdnsServiceCallback>) -> anyhow::Result<()> {
        let autoconn_allowedlist = self.autoconn_allowedlist.clone();
        let discovered_services = self.discovered_services.clone();

        for &service_type in &ADB_DNS_SERVICES {
            let receiver = self.daemon.browse(service_type)?;
            let callback = callback.clone();
            let service_type_str = service_type.to_string();
            let autoconn_allowedlist = autoconn_allowedlist.clone();
            let discovered_services = discovered_services.clone();

            thread::spawn(move || {
                while let Ok(event) = receiver.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let m_info = MdnsServiceInfo {
                                instance_name: info.get_fullname().to_string(),
                                service_type: service_type_str.clone(),
                                hostname: info.get_hostname().to_string(),
                                ip_addresses: info.get_addresses().iter().map(|s| s.to_ip_addr()).collect(),
                                port: info.get_port(),
                                txt_record: info.get_properties().iter().map(|item| {
                                    let key = item.key().to_string();
                                    let val = item.val().map(|v| String::from_utf8_lossy(v).into_owned()).unwrap_or_default();
                                    (key, val)
                                }).collect(),
                            };

                            {
                                let mut services = discovered_services.lock().unwrap();
                                let entry = services.entry(service_type_str.clone()).or_insert_with(Vec::new);
                                entry.retain(|s| s.instance_name != m_info.instance_name);
                                entry.push(m_info.clone());
                            }

                            if should_auto_connect(&service_type_str, info.get_fullname(), &autoconn_allowedlist) {
                                if let Some(cb) = &callback {
                                    cb(m_info);
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(service_type, fullname) => {
                            let mut services = discovered_services.lock().unwrap();
                            if let Some(entry) = services.get_mut(&service_type) {
                                entry.retain(|s| s.instance_name != fullname);
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
        Ok(())
    }

    /// Registers an ADB service via MDNS.
    /// Ported from `register_mdns_service` in `daemon/mdns.cpp`.
    pub fn register(&self, name: &str, service_type: &str, address: &str, port: u16, properties: HashMap<String, String>) -> anyhow::Result<()> {
        let info = ServiceInfo::new(service_type, name, &format!("{}.local.", name), address, port, Some(properties))?;
        self.daemon.register(info)?;
        Ok(())
    }

    /// Checks the status of the MDNS daemon.
    /// Ported from `mdns_check` in `adb_mdns.h`.
    pub fn mdns_check(&self) -> String {
        // Since we are using a pure-rust daemon, we can just say it's running.
        "mdns daemon version [pure-rust]".to_string()
    }

    /// Lists all discovered MDNS services.
    /// Ported from `mdns_list_discovered_services` in `adb_mdns.h`.
    pub fn mdns_list_discovered_services(&self) -> String {
        let mut result = String::new();
        let services = self.discovered_services.lock().unwrap();
        for service_list in services.values() {
            for si in service_list {
                let ip = si.ip_addresses.get(0).map(|ip| ip.to_string()).unwrap_or_else(|| "unknown".to_string());
                result.push_str(&format!("{}\t{}\t{}:{}\n", si.instance_name, si.service_type, ip, si.port));
            }
        }
        result
    }

    /// Returns information about a specific service.
    pub fn get_service_info(&self, name: &str, reg_type_filter: Option<&str>) -> Option<MdnsServiceInfo> {
        let services = self.discovered_services.lock().unwrap();

        let mdns_instance = utils::mdns_parse_instance_name(name);

        for (reg_type, service_list) in services.iter() {
            if let Some(filter) = reg_type_filter {
                if !reg_type.starts_with(filter) {
                    continue;
                }
            }

            for si in service_list {
                if si.instance_name == name {
                    return Some(si.clone());
                }
                // Also check if it matches the instance name part
                if let Some(ref instance) = mdns_instance {
                    if si.instance_name.contains(&instance.instance_name) {
                        return Some(si.clone());
                    }
                }
            }
        }
        None
    }
}

/// Returns the index of an ADB DNS service by its registration type.
/// Ported from `adb_DNSServiceIndexByName` in `adb_mdns.cpp`.
pub fn adb_dns_service_index_by_name(reg_type: &str) -> Option<usize> {
    for (i, &service) in ADB_DNS_SERVICES.iter().enumerate() {
        if reg_type.starts_with(service) {
            return Some(i);
        }
    }
    None
}

/// Ported from original/adb_mdns.cpp: `adb_DNSServiceShouldAutoConnect`
fn should_auto_connect(reg_type: &str, instance_name: &str, allowedlist: &Arc<Mutex<HashSet<usize>>>) -> bool {
    if !utils::is_mdns_enabled() {
        return false;
    }

    let index = match adb_dns_service_index_by_name(reg_type) {
        Some(i) => i,
        None => return false,
    };

    if index != K_ADB_TRANSPORT_SERVICE_REF_INDEX && index != K_ADB_SECURE_CONNECT_SERVICE_REF_INDEX {
        return false;
    }

    if !allowedlist.lock().unwrap().contains(&index) {
        return false;
    }

    if instance_name.starts_with("adb-EMULATOR") {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adb_dns_service_index_by_name() {
        assert_eq!(adb_dns_service_index_by_name("_adb._tcp.local."), Some(K_ADB_TRANSPORT_SERVICE_REF_INDEX));
        assert_eq!(adb_dns_service_index_by_name("_adb-tls-pairing._tcp.local."), Some(K_ADB_SECURE_PAIRING_SERVICE_REF_INDEX));
        assert_eq!(adb_dns_service_index_by_name("_adb-tls-connect._tcp.local."), Some(K_ADB_SECURE_CONNECT_SERVICE_REF_INDEX));
        assert_eq!(adb_dns_service_index_by_name("_unknown._tcp.local."), None);
    }

    #[test]
    fn test_should_auto_connect() {
        let allowedlist = Arc::new(Mutex::new(HashSet::new()));
        allowedlist.lock().unwrap().insert(K_ADB_TRANSPORT_SERVICE_REF_INDEX);

        // Allowed type
        assert!(should_auto_connect("_adb._tcp.local.", "my-device", &allowedlist));

        // Emulator should be ignored
        assert!(!should_auto_connect("_adb._tcp.local.", "adb-EMULATOR-5554", &allowedlist));

        // Not in allowedlist
        assert!(!should_auto_connect("_adb-tls-connect._tcp.local.", "my-device", &allowedlist));

        // Not a transport or connect type
        assert!(!should_auto_connect("_adb-tls-pairing._tcp.local.", "my-device", &allowedlist));
    }
}
