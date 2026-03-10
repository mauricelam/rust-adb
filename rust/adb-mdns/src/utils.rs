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

/// Represents an MDNS instance.
/// Ported from `MdnsInstance` in `client/mdns_utils.h`.
#[derive(Debug, PartialEq, Eq)]
pub struct MdnsInstance {
    /// The name of the instance.
    pub instance_name: String,
    /// The name of the service.
    pub service_name: String,
    /// The transport type (e.g., "tcp", "udp").
    pub transport_type: String,
}

impl MdnsInstance {
    /// Creates a new `MdnsInstance`.
    pub fn new(instance_name: &str, service_name: &str, transport_type: &str) -> Self {
        Self {
            instance_name: instance_name.to_string(),
            service_name: service_name.to_string(),
            transport_type: transport_type.to_string(),
        }
    }
}

/// Parses an MDNS instance name into its components.
/// Ported from `mdns_parse_instance_name` in `client/mdns_utils.cpp`.
pub fn mdns_parse_instance_name(mut name: &str) -> Option<MdnsInstance> {
    if name.is_empty() {
        return None;
    }

    let mut has_local_suffix = false;
    // Strip the local suffix, if any
    {
        let local_suffix = if name.ends_with(".local.") {
            ".local."
        } else if name.ends_with(".local") {
            ".local"
        } else {
            ""
        };

        if !local_suffix.is_empty() {
            name = &name[..name.len() - local_suffix.len()];
            if name.is_empty() {
                return None;
            }
            has_local_suffix = true;
        }
    }

    let mut transport = "";
    // Strip the transport suffix, if any
    {
        let add_dot = if !has_local_suffix && name.ends_with('.') { "." } else { "" };
        let transport_suffixes = ["._tcp", "._udp"];

        for t in transport_suffixes {
            let suffix = format!("{}{}", t, add_dot);
            if name.ends_with(&suffix) {
                name = &name[..name.len() - suffix.len()];
                if name.is_empty() {
                    return None;
                }
                transport = &t[1..];
                break;
            }
        }

        if has_local_suffix && transport.is_empty() {
            return None;
        }
    }

    if !has_local_suffix && transport.is_empty() {
        return Some(MdnsInstance::new(name, "", ""));
    }

    // Split the service name from the instance name
    if let Some(pos) = name.rfind('.') {
        if pos == 0 || pos == name.len() - 1 {
            return None;
        }
        Some(MdnsInstance::new(&name[..pos], &name[pos + 1..], transport))
    } else {
        None
    }
}

/// Returns true if MDNS is enabled.
/// Ported from `is_enabled` in `client/mdns_utils.cpp`.
pub fn is_mdns_enabled() -> bool {
    std::env::var("ADB_MDNS").map(|v| v != "0").unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mdns_parse_instance_name() {
        // Ported from original/client/mdns_utils_test.cpp

        // Just the instance name
        {
            let str = ".";
            let res = mdns_parse_instance_name(str).unwrap();
            assert_eq!(res.instance_name, str);
            assert!(res.service_name.is_empty());
            assert!(res.transport_type.is_empty());
        }
        {
            let str = "my.name";
            let res = mdns_parse_instance_name(str).unwrap();
            assert_eq!(res.instance_name, str);
            assert!(res.service_name.is_empty());
            assert!(res.transport_type.is_empty());
        }
        {
            let str = "my.name.";
            let res = mdns_parse_instance_name(str).unwrap();
            assert_eq!(res.instance_name, str);
            assert!(res.service_name.is_empty());
            assert!(res.transport_type.is_empty());
        }

        // With "_tcp", "_udp" transport type
        for transport in ["._tcp", "._udp"] {
            {
                let str = transport;
                assert!(mdns_parse_instance_name(str).is_none());
            }
            {
                let str = format!("{}.", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!("service{}", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!(".service{}", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!("service.{}", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!("my.service{}", transport);
                let res = mdns_parse_instance_name(&str).unwrap();
                assert_eq!(res.instance_name, "my");
                assert_eq!(res.service_name, "service");
                assert_eq!(res.transport_type, &transport[1..]);
            }
            {
                let str = format!("my.service{}.", transport);
                let res = mdns_parse_instance_name(&str).unwrap();
                assert_eq!(res.instance_name, "my");
                assert_eq!(res.service_name, "service");
                assert_eq!(res.transport_type, &transport[1..]);
            }
            {
                let str = format!("my..service{}", transport);
                let res = mdns_parse_instance_name(&str).unwrap();
                assert_eq!(res.instance_name, "my.");
                assert_eq!(res.service_name, "service");
                assert_eq!(res.transport_type, &transport[1..]);
            }
            {
                let str = format!("my.name.service{}.", transport);
                let res = mdns_parse_instance_name(&str).unwrap();
                assert_eq!(res.instance_name, "my.name");
                assert_eq!(res.service_name, "service");
                assert_eq!(res.transport_type, &transport[1..]);
            }
            {
                let str = format!("name.service.{}.", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }

            // With ".local" domain
            {
                let str = ".local";
                assert!(mdns_parse_instance_name(str).is_none());
            }
            {
                let str = ".local.";
                assert!(mdns_parse_instance_name(str).is_none());
            }
            {
                let str = "name.local";
                assert!(mdns_parse_instance_name(str).is_none());
            }
            {
                let str = format!("{}.local", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!("service{}.local", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!("name.service{}.local", transport);
                let res = mdns_parse_instance_name(&str).unwrap();
                assert_eq!(res.instance_name, "name");
                assert_eq!(res.service_name, "service");
                assert_eq!(res.transport_type, &transport[1..]);
            }
            {
                let str = format!("name.service{}.local.", transport);
                let res = mdns_parse_instance_name(&str).unwrap();
                assert_eq!(res.instance_name, "name");
                assert_eq!(res.service_name, "service");
                assert_eq!(res.transport_type, &transport[1..]);
            }
            {
                let str = format!("name.service{}..local.", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
            {
                let str = format!("name.service.{}.local.", transport);
                assert!(mdns_parse_instance_name(&str).is_none());
            }
        }
    }
}
