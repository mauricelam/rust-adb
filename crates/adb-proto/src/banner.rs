//! Connection banner parsing, feature sets, and protocol version/max-payload negotiation.
//!
//! Ports symbols from `original/adb.h`, `original/transport.h`, `original/transport.cpp`, and `original/adb.cpp`.

use std::fmt;
use std::str::FromStr;

/// Minimum supported ADB transport protocol version (`A_VERSION_MIN`).
/// Ports `A_VERSION_MIN` from `original/adb.h`.
pub const A_VERSION_MIN: u32 = 0x0100_0000;

/// Version from which checksums are omitted (`A_VERSION_SKIP_CHECKSUM`).
/// Ports `A_VERSION_SKIP_CHECKSUM` from `original/adb.h`.
pub const A_VERSION_SKIP_CHECKSUM: u32 = 0x0100_0001;

/// Maximum supported ADB transport protocol version (`A_VERSION`).
/// Ports `A_VERSION` from `original/adb.h`.
pub const A_VERSION: u32 = 0x0100_0001;

/// Initial maximum payload size before version/maxdata negotiation (`MAX_PAYLOAD_V1`).
/// Ports `MAX_PAYLOAD_V1` from `original/adb.h`.
pub const MAX_PAYLOAD_V1: usize = 4 * 1024;

/// Maximum payload size supported by modern ADB protocol (`MAX_PAYLOAD`).
/// Ports `MAX_PAYLOAD` from `original/adb.h`.
pub const MAX_PAYLOAD: usize = 1024 * 1024;

/// Negotiates protocol version as the minimum of the host and remote versions.
/// Ports logic from `atransport::set_protocol_version` in `original/transport.cpp`.
pub fn negotiate_version(v1: u32, v2: u32) -> u32 {
    std::cmp::min(v1, v2)
}

/// Negotiates maximum payload as the minimum of the host and remote values.
/// Ports logic from `atransport::set_max_payload` in `original/transport.cpp`.
pub fn negotiate_max_payload(p1: usize, p2: usize) -> usize {
    std::cmp::min(p1, p2)
}

/// Known feature names and an unknown variant for round-trip preservation.
/// Ports `kFeature*` constants from `original/transport.h` and `original/transport.cpp`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Feature {
    Shell2,
    Cmd,
    Stat2,
    Ls2,
    Libusb,
    PushSync,
    Apex,
    FixedPushMkdir,
    Abb,
    AbbExec,
    FixedPushSymlinkTimestamp,
    RemountShell,
    TrackApp,
    SendRecv2,
    SendRecv2Brotli,
    SendRecv2LZ4,
    SendRecv2Zstd,
    SendRecv2DryRunSend,
    DelayedAck,
    OpenscreenMdns,
    DeviceTrackerProtoFormat,
    DevRaw,
    AppInfo,
    ServerStatus,
    Unknown(String),
}

impl Feature {
    /// Returns the string representation of the feature.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shell2 => "shell_v2",
            Self::Cmd => "cmd",
            Self::Stat2 => "stat_v2",
            Self::Ls2 => "ls_v2",
            Self::Libusb => "libusb",
            Self::PushSync => "push_sync",
            Self::Apex => "apex",
            Self::FixedPushMkdir => "fixed_push_mkdir",
            Self::Abb => "abb",
            Self::AbbExec => "abb_exec",
            Self::FixedPushSymlinkTimestamp => "fixed_push_symlink_timestamp",
            Self::RemountShell => "remount_shell",
            Self::TrackApp => "track_app",
            Self::SendRecv2 => "sendrecv_v2",
            Self::SendRecv2Brotli => "sendrecv_v2_brotli",
            Self::SendRecv2LZ4 => "sendrecv_v2_lz4",
            Self::SendRecv2Zstd => "sendrecv_v2_zstd",
            Self::SendRecv2DryRunSend => "sendrecv_v2_dry_run_send",
            Self::DelayedAck => "delayed_ack",
            Self::OpenscreenMdns => "openscreen_mdns",
            Self::DeviceTrackerProtoFormat => "devicetracker_proto_format",
            Self::DevRaw => "devraw",
            Self::AppInfo => "app_info",
            Self::ServerStatus => "server_status",
            Self::Unknown(s) => s.as_str(),
        }
    }
}

/// Returns the client/server supported features matching `supported_features()` in C++ `original/transport.cpp`.
pub fn supported_features() -> Features {
    let mut features = Features::new();
    features.push(Feature::Shell2);
    features.push(Feature::Cmd);
    features.push(Feature::Stat2);
    features.push(Feature::Ls2);
    features.push(Feature::FixedPushMkdir);
    features.push(Feature::Apex);
    features.push(Feature::Abb);
    features.push(Feature::FixedPushSymlinkTimestamp);
    features.push(Feature::AbbExec);
    features.push(Feature::RemountShell);
    features.push(Feature::TrackApp);
    features.push(Feature::SendRecv2);
    features.push(Feature::SendRecv2Brotli);
    features.push(Feature::SendRecv2LZ4);
    features.push(Feature::SendRecv2Zstd);
    features.push(Feature::SendRecv2DryRunSend);
    features.push(Feature::OpenscreenMdns);
    features.push(Feature::DeviceTrackerProtoFormat);
    features.push(Feature::DevRaw);
    features.push(Feature::AppInfo);
    features.push(Feature::ServerStatus);

    let burst = std::env::var("ADB_BURST_MODE").is_ok_and(|v| v == "1");
    if burst {
        features.push(Feature::DelayedAck);
    }

    features
}

impl FromStr for Feature {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let f = match s {
            "shell_v2" => Self::Shell2,
            "cmd" => Self::Cmd,
            "stat_v2" => Self::Stat2,
            "ls_v2" => Self::Ls2,
            "libusb" => Self::Libusb,
            "push_sync" => Self::PushSync,
            "apex" => Self::Apex,
            "fixed_push_mkdir" => Self::FixedPushMkdir,
            "abb" => Self::Abb,
            "abb_exec" => Self::AbbExec,
            "fixed_push_symlink_timestamp" => Self::FixedPushSymlinkTimestamp,
            "remount_shell" => Self::RemountShell,
            "track_app" => Self::TrackApp,
            "sendrecv_v2" => Self::SendRecv2,
            "sendrecv_v2_brotli" => Self::SendRecv2Brotli,
            "sendrecv_v2_lz4" => Self::SendRecv2LZ4,
            "sendrecv_v2_zstd" => Self::SendRecv2Zstd,
            "sendrecv_v2_dry_run_send" => Self::SendRecv2DryRunSend,
            "delayed_ack" => Self::DelayedAck,
            "openscreen_mdns" => Self::OpenscreenMdns,
            "devicetracker_proto_format" => Self::DeviceTrackerProtoFormat,
            "devraw" => Self::DevRaw,
            "app_info" => Self::AppInfo,
            "server_status" => Self::ServerStatus,
            other => Self::Unknown(other.to_string()),
        };
        Ok(f)
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Set of features parsed from or formatted into a comma-separated feature list string.
/// Ports `FeatureSet`, `FeatureSetToString`, and `StringToFeatureSet` from `original/transport.h` and `original/transport.cpp`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Features {
    items: Vec<Feature>,
}

impl Features {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn parse(s: &str) -> Self {
        if s.is_empty() {
            return Self::new();
        }
        let items = s
            .split(',')
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap())
            .collect();
        Self { items }
    }

    pub fn contains(&self, feature: &Feature) -> bool {
        self.items.contains(feature)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn items(&self) -> &[Feature] {
        &self.items
    }

    pub fn push(&mut self, feature: Feature) {
        if !self.contains(&feature) {
            self.items.push(feature);
        }
    }
}

impl fmt::Display for Features {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let str_items: Vec<&str> = self.items.iter().map(|item| item.as_str()).collect();
        write!(f, "{}", str_items.join(","))
    }
}

/// Parsed CNXN connection banner (`<systemtype>:<serialno>:<properties>`).
///
/// Ports `parse_banner` and `get_connection_string` from `original/adb.cpp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub system_type: String,
    pub serial: String,
    pub properties: Vec<(String, String)>,
    pub features: Features,
    pub ends_with_semicolon: bool,
}

impl Banner {
    /// Parses a connection banner string.
    /// Format: `<systemtype>:<serialno>:<key=val;key2=val2;>` or `<systemtype>::<properties>`.
    /// Preserves extra properties and unknown features in insertion order.
    pub fn parse(s: &str) -> Self {
        let pieces: Vec<&str> = s.split(':').collect();
        let system_type = pieces.first().copied().unwrap_or("").to_string();
        let serial = pieces.get(1).copied().unwrap_or("").to_string();

        let mut properties = Vec::new();
        let mut features = Features::new();
        let mut ends_with_semicolon = false;

        if pieces.len() > 2 {
            // Rejoin remainder in case property values contain colons
            let props_str = pieces[2..].join(":");
            ends_with_semicolon = props_str.ends_with(';');

            for prop in props_str.split(';') {
                if prop.is_empty() {
                    continue;
                }
                let key_val: Vec<&str> = prop.splitn(2, '=').collect();
                if key_val.len() != 2 {
                    continue;
                }
                let key = key_val[0];
                let val = key_val[1];

                if key == "features" {
                    features = Features::parse(val);
                } else {
                    properties.push((key.to_string(), val.to_string()));
                }
            }
        }

        Self {
            system_type,
            serial,
            properties,
            features,
            ends_with_semicolon,
        }
    }

    pub fn product(&self) -> Option<&str> {
        self.get_property("ro.product.name")
    }

    pub fn model(&self) -> Option<&str> {
        self.get_property("ro.product.model")
    }

    pub fn device(&self) -> Option<&str> {
        self.get_property("ro.product.device")
    }

    pub fn get_property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn set_property(&mut self, key: impl Into<String>, val: impl Into<String>) {
        let key = key.into();
        let val = val.into();
        if let Some(pair) = self.properties.iter_mut().find(|(k, _)| k == &key) {
            pair.1 = val;
        } else {
            self.properties.push((key, val));
        }
    }

    /// Serializes the banner as raw bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl fmt::Display for Banner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut prop_parts: Vec<String> = self
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        if !self.features.is_empty() {
            prop_parts.push(format!("features={}", self.features));
        }

        if prop_parts.is_empty() {
            write!(f, "{}:{}:", self.system_type, self.serial)
        } else {
            let joined = prop_parts.join(";");
            if self.ends_with_semicolon {
                write!(f, "{}:{}:{};", self.system_type, self.serial, joined)
            } else {
                write!(f, "{}:{}:{}", self.system_type, self.serial, joined)
            }
        }
    }
}
