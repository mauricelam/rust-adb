//! mDNS discovery data types and utilities.
//!
//! Ports service definitions, name validation, and instance parsing from
//! `original/adb_mdns.h`, `original/adb_mdns.cpp`, and `original/client/mdns_utils.cpp`.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// Service name for classic unencrypted ADB transport.
pub const ADB_MDNS_SERVICE_TYPE: &str = "adb";

/// Service name for secure TLS pairing.
pub const ADB_MDNS_TLS_PAIRING_TYPE: &str = "adb-tls-pairing";

/// Service name for secure TLS connect.
pub const ADB_MDNS_TLS_CONNECT_TYPE: &str = "adb-tls-connect";

/// Fully qualified DNS-SD service registration type for classic ADB (`_adb._tcp`).
pub const ADB_FULL_MDNS_SERVICE_TYPE: &str = "_adb._tcp";

/// Fully qualified DNS-SD service registration type for TLS pairing (`_adb-tls-pairing._tcp`).
pub const ADB_FULL_MDNS_TLS_PAIRING_TYPE: &str = "_adb-tls-pairing._tcp";

/// Fully qualified DNS-SD service registration type for TLS connect (`_adb-tls-connect._tcp`).
pub const ADB_FULL_MDNS_TLS_CONNECT_TYPE: &str = "_adb-tls-connect._tcp";

/// Array of all registered ADB DNS services in C++ index order.
pub const ADB_DNS_SERVICES: [&str; 3] = [
    ADB_FULL_MDNS_SERVICE_TYPE,
    ADB_FULL_MDNS_TLS_PAIRING_TYPE,
    ADB_FULL_MDNS_TLS_CONNECT_TYPE,
];

/// Validates whether a given service name conforms to RFC 6335 rules.
///
/// Ports `isValidMdnsServiceName` from `original/mdns_test.cpp`.
/// The rules for Service Names (RFC 6335) state that they may be no more
/// than 15 characters long, consisting of only letters, digits, and hyphens,
/// must begin and end with a letter or digit, must not contain consecutive hyphens,
/// and must contain at least one letter.
pub fn is_valid_mdns_service_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 15 {
        return false;
    }

    let bytes = name.as_bytes();
    let mut has_at_least_one_letter = false;
    let mut saw_hyphen = false;

    for (i, &b) in bytes.iter().enumerate() {
        if b == b'-' {
            if i == 0 || i == bytes.len() - 1 {
                return false;
            }
            if saw_hyphen {
                return false;
            }
            saw_hyphen = true;
            continue;
        }

        saw_hyphen = false;
        if b.is_ascii_alphabetic() {
            has_at_least_one_letter = true;
            continue;
        }

        if b.is_ascii_digit() {
            continue;
        }

        return false;
    }

    has_at_least_one_letter
}

/// Parsed mDNS instance name.
///
/// Ports `MdnsInstance` from `original/client/mdns_utils.h`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsInstance {
    /// Instance name portion (e.g., "my name" or "adb-serial").
    pub instance_name: String,
    /// Service name portion (e.g., "_adb-tls-connect" or empty).
    pub service_name: String,
    /// Transport type portion (e.g., "_tcp", "_udp", or empty).
    pub transport_type: String,
}

/// Parses a structured service instance name following RFC 6763 section 4.1.
///
/// Ports `mdns_parse_instance_name` from `original/client/mdns_utils.cpp`.
///
/// Supported formats:
/// - `<Instance>.<Service>.<Domain>.?` (Domain assumed to be ".local")
/// - `<Instance>.<Service>.?` (ends with `._tcp` or `._udp`)
/// - `<Instance>` (bare instance, may contain dots)
pub fn mdns_parse_instance_name(name: &str) -> Option<MdnsInstance> {
    if name.is_empty() {
        return None;
    }

    let mut working = name;
    let mut has_local_suffix = false;

    // Strip .local / .local. suffix if present
    if working.ends_with(".local.") {
        working = &working[..working.len() - 7];
        has_local_suffix = true;
    } else if working.ends_with(".local") {
        working = &working[..working.len() - 6];
        has_local_suffix = true;
    }

    if has_local_suffix && working.is_empty() {
        return None;
    }

    let mut transport = String::new();
    let add_dot = !has_local_suffix && working.ends_with('.');
    let transport_suffixes = ["._tcp", "._udp"];

    for t in &transport_suffixes {
        let suffix = if add_dot {
            format!("{t}.")
        } else {
            t.to_string()
        };
        if working.ends_with(&suffix) {
            let new_len = working.len() - suffix.len();
            if new_len == 0 {
                return None;
            }
            working = &working[..new_len];
            // t starts with '.' so skip it for transport string ("_tcp", "_udp")
            transport = t[1..].to_string();
            break;
        }
    }

    if has_local_suffix && transport.is_empty() {
        return None;
    }

    if !has_local_suffix && transport.is_empty() {
        return Some(MdnsInstance {
            instance_name: name.to_string(),
            service_name: String::new(),
            transport_type: String::new(),
        });
    }

    // Split service name from instance name at the last '.'
    if let Some(pos) = working.rfind('.') {
        if pos == 0 || pos == working.len() - 1 {
            return None;
        }
        Some(MdnsInstance {
            instance_name: working[..pos].to_string(),
            service_name: working[pos + 1..].to_string(),
            transport_type: transport,
        })
    } else {
        None
    }
}

/// Discovered mDNS service entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsServiceInfo {
    /// Instance name of the service (e.g. `adb-12345`).
    pub instance_name: String,
    /// Service name / reg_type (e.g. `_adb-tls-connect._tcp`).
    pub service_name: String,
    /// IP address of the service.
    pub addr: IpAddr,
    /// TCP/UDP port.
    pub port: u16,
}

impl MdnsServiceInfo {
    /// Formats this service record as a single tab-separated line matching C++ ADB.
    ///
    /// `<instance_name>\t<service_name>\t<ip>:<port>\n`
    pub fn format_line(&self) -> String {
        format!(
            "{}\t{}\t{}:{}\n",
            self.instance_name, self.service_name, self.addr, self.port
        )
    }
}

/// Registry of discovered mDNS services.
#[derive(Debug, Clone, Default)]
pub struct MdnsDiscoveryRegistry {
    services: Arc<Mutex<BTreeMap<(String, String), MdnsServiceInfo>>>,
}

impl MdnsDiscoveryRegistry {
    /// Creates a new empty discovery registry.
    pub fn new() -> Self {
        Self {
            services: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Registers or updates a discovered mDNS service.
    pub fn register(&self, info: MdnsServiceInfo) {
        let mut map = self.services.lock().unwrap();
        let key = (info.instance_name.clone(), info.service_name.clone());
        map.insert(key, info);
    }

    /// Unregisters an mDNS service by instance name and service type.
    pub fn unregister(&self, instance_name: &str, service_name: &str) -> Option<MdnsServiceInfo> {
        let mut map = self.services.lock().unwrap();
        map.remove(&(instance_name.to_string(), service_name.to_string()))
    }

    /// Lists all discovered mDNS services.
    pub fn list_services(&self) -> Vec<MdnsServiceInfo> {
        let map = self.services.lock().unwrap();
        map.values().cloned().collect()
    }

    /// Formats all discovered services into a single listing string matching `mdns_list_discovered_services()`.
    pub fn format_discovered_services(&self) -> String {
        let services = self.list_services();
        let mut out = String::new();
        for s in services {
            out.push_str(&s.format_line());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_is_valid_mdns_service_name() {
        // Longer than 15 characters
        assert!(!is_valid_mdns_service_name("abcd1234abcd1234"));

        // Contains invalid characters
        assert!(!is_valid_mdns_service_name("a*a"));
        assert!(!is_valid_mdns_service_name("a_a"));
        assert!(!is_valid_mdns_service_name("_a"));

        // Does not begin or end with letter or digit
        assert!(!is_valid_mdns_service_name(""));
        assert!(!is_valid_mdns_service_name("-"));
        assert!(!is_valid_mdns_service_name("-a"));
        assert!(!is_valid_mdns_service_name("-1"));
        assert!(!is_valid_mdns_service_name("a-"));
        assert!(!is_valid_mdns_service_name("1-"));

        // Contains consecutive hyphens
        assert!(!is_valid_mdns_service_name("a--a"));

        // Does not contain at least one letter
        assert!(!is_valid_mdns_service_name("1"));
        assert!(!is_valid_mdns_service_name("12"));
        assert!(!is_valid_mdns_service_name("1-2"));

        // Valid service names
        assert!(is_valid_mdns_service_name("a"));
        assert!(is_valid_mdns_service_name("a1"));
        assert!(is_valid_mdns_service_name("1A"));
        assert!(is_valid_mdns_service_name("aZ"));
        assert!(is_valid_mdns_service_name("a-Z"));
        assert!(is_valid_mdns_service_name("a-b-Z"));
        assert!(is_valid_mdns_service_name("abc-def-123-456"));
        assert!(is_valid_mdns_service_name(ADB_MDNS_SERVICE_TYPE));
        assert!(is_valid_mdns_service_name(ADB_MDNS_TLS_PAIRING_TYPE));
        assert!(is_valid_mdns_service_name(ADB_MDNS_TLS_CONNECT_TYPE));
    }

    #[test]
    fn test_mdns_parse_instance_name() {
        // Just the instance name
        {
            let res = mdns_parse_instance_name(".").unwrap();
            assert_eq!(res.instance_name, ".");
            assert!(res.service_name.is_empty());
            assert!(res.transport_type.is_empty());
        }
        {
            let res = mdns_parse_instance_name("my.name").unwrap();
            assert_eq!(res.instance_name, "my.name");
            assert!(res.service_name.is_empty());
            assert!(res.transport_type.is_empty());
        }
        {
            let res = mdns_parse_instance_name("my.name.").unwrap();
            assert_eq!(res.instance_name, "my.name.");
            assert!(res.service_name.is_empty());
            assert!(res.transport_type.is_empty());
        }

        // With _tcp and _udp transport types
        for transport in ["._tcp", "._udp"] {
            assert!(mdns_parse_instance_name(transport).is_none());
            assert!(mdns_parse_instance_name(&format!("{transport}.")).is_none());
            assert!(mdns_parse_instance_name(&format!("service{transport}")).is_none());
            assert!(mdns_parse_instance_name(&format!(".service{transport}")).is_none());
            assert!(mdns_parse_instance_name(&format!("service.{transport}")).is_none());

            let res1 = mdns_parse_instance_name(&format!("my.service{transport}")).unwrap();
            assert_eq!(res1.instance_name, "my");
            assert_eq!(res1.service_name, "service");
            assert_eq!(res1.transport_type, &transport[1..]);

            let res2 = mdns_parse_instance_name(&format!("my.service{transport}.")).unwrap();
            assert_eq!(res2.instance_name, "my");
            assert_eq!(res2.service_name, "service");
            assert_eq!(res2.transport_type, &transport[1..]);

            let res3 = mdns_parse_instance_name(&format!("my..service{transport}")).unwrap();
            assert_eq!(res3.instance_name, "my.");
            assert_eq!(res3.service_name, "service");
            assert_eq!(res3.transport_type, &transport[1..]);

            let res4 = mdns_parse_instance_name(&format!("my.name.service{transport}.")).unwrap();
            assert_eq!(res4.instance_name, "my.name");
            assert_eq!(res4.service_name, "service");
            assert_eq!(res4.transport_type, &transport[1..]);

            assert!(mdns_parse_instance_name(&format!("name.service.{transport}.")).is_none());

            // With .local domain
            assert!(mdns_parse_instance_name(".local").is_none());
            assert!(mdns_parse_instance_name(".local.").is_none());
            assert!(mdns_parse_instance_name("name.local").is_none());
            assert!(mdns_parse_instance_name(&format!("{transport}.local")).is_none());
            assert!(mdns_parse_instance_name(&format!("service{transport}.local")).is_none());

            let res5 = mdns_parse_instance_name(&format!("name.service{transport}.local")).unwrap();
            assert_eq!(res5.instance_name, "name");
            assert_eq!(res5.service_name, "service");
            assert_eq!(res5.transport_type, &transport[1..]);

            let res6 =
                mdns_parse_instance_name(&format!("name.service{transport}.local.")).unwrap();
            assert_eq!(res6.instance_name, "name");
            assert_eq!(res6.service_name, "service");
            assert_eq!(res6.transport_type, &transport[1..]);

            assert!(
                mdns_parse_instance_name(&format!("name.service{transport}..local.")).is_none()
            );
            assert!(
                mdns_parse_instance_name(&format!("name.service.{transport}.local.")).is_none()
            );
        }
    }

    #[test]
    fn test_mdns_discovery_registry() {
        let registry = MdnsDiscoveryRegistry::new();
        assert_eq!(registry.format_discovered_services(), "");

        let info = MdnsServiceInfo {
            instance_name: "device1".to_string(),
            service_name: "_adb-tls-connect._tcp".to_string(),
            addr: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)),
            port: 5555,
        };
        registry.register(info.clone());

        assert_eq!(
            registry.format_discovered_services(),
            "device1\t_adb-tls-connect._tcp\t192.168.1.100:5555\n"
        );

        let removed = registry.unregister("device1", "_adb-tls-connect._tcp");
        assert_eq!(removed, Some(info));
        assert_eq!(registry.format_discovered_services(), "");
    }
}
