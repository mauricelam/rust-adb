//! Strongly typed host and target services for ADB smart sockets.
//!
//! Ported from `original/sockets.cpp`, `original/commandline.cpp`, and `original/docs/dev/services.md`.

use std::fmt;
use std::str::FromStr;

use crate::transport_selector::TransportSelector;
use crate::types::TransportId;

/// Track devices service format mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackDevicesMode {
    Normal,
    Long,
    ProtoText,
    ProtoBinary,
}

impl TrackDevicesMode {
    pub fn service_name(&self) -> &'static str {
        match self {
            Self::Normal => "track-devices",
            Self::Long => "track-devices-l",
            Self::ProtoText => "track-devices-proto-text",
            Self::ProtoBinary => "track-devices-proto-binary",
        }
    }
}

/// Strongly typed host services dispatched on the local ADB daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostService {
    Version,
    Kill,
    Devices {
        long: bool,
    },
    TrackDevices(TrackDevicesMode),
    Features,
    HostFeatures,
    GetState,
    GetSerialno,
    GetDevpath,
    TransportId,
    Connect(String),
    Disconnect(Option<String>),
    Transport(TransportSelector),
    ForwardList,
    ForwardKillAll,
    ForwardKill(String),
    ForwardAdd {
        no_rebind: bool,
        local: String,
        remote: String,
    },
    Other(String),
}

impl FromStr for HostService {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let service = s.strip_prefix("host:").unwrap_or(s);

        if service == "version" {
            Ok(Self::Version)
        } else if service == "kill" {
            Ok(Self::Kill)
        } else if service == "devices" {
            Ok(Self::Devices { long: false })
        } else if service == "devices-l" {
            Ok(Self::Devices { long: true })
        } else if service == "track-devices" {
            Ok(Self::TrackDevices(TrackDevicesMode::Normal))
        } else if service == "track-devices-l" {
            Ok(Self::TrackDevices(TrackDevicesMode::Long))
        } else if service == "track-devices-proto-text" {
            Ok(Self::TrackDevices(TrackDevicesMode::ProtoText))
        } else if service == "track-devices-proto-binary" {
            Ok(Self::TrackDevices(TrackDevicesMode::ProtoBinary))
        } else if service == "features" {
            Ok(Self::Features)
        } else if service == "host-features" {
            Ok(Self::HostFeatures)
        } else if service == "get-state" {
            Ok(Self::GetState)
        } else if service == "get-serialno" {
            Ok(Self::GetSerialno)
        } else if service == "get-devpath" {
            Ok(Self::GetDevpath)
        } else if service == "transport-id" {
            Ok(Self::TransportId)
        } else if let Some(target) = service.strip_prefix("connect:") {
            Ok(Self::Connect(target.to_string()))
        } else if let Some(target) = service.strip_prefix("disconnect:") {
            let t = if target.is_empty() {
                None
            } else {
                Some(target.to_string())
            };
            Ok(Self::Disconnect(t))
        } else if service == "list-forward" {
            Ok(Self::ForwardList)
        } else if service == "killforward-all" {
            Ok(Self::ForwardKillAll)
        } else if let Some(local) = service.strip_prefix("killforward:") {
            Ok(Self::ForwardKill(local.to_string()))
        } else if let Some(fwd_args) = service.strip_prefix("forward:") {
            let (no_rebind, args) = if let Some(norebind_args) = fwd_args.strip_prefix("norebind:")
            {
                (true, norebind_args)
            } else {
                (false, fwd_args)
            };
            if let Some((local, remote)) = args.split_once(';') {
                Ok(Self::ForwardAdd {
                    no_rebind,
                    local: local.to_string(),
                    remote: remote.to_string(),
                })
            } else {
                Ok(Self::Other(s.to_string()))
            }
        } else if service.starts_with("transport") || service.starts_with("tport:") {
            let selector = if let Some(tport_spec) = service.strip_prefix("tport:") {
                if let Some(serial) = tport_spec.strip_prefix("serial:") {
                    TransportSelector::Serial(serial.to_string())
                } else if tport_spec == "usb" {
                    TransportSelector::Usb
                } else if tport_spec == "local" {
                    TransportSelector::Local
                } else {
                    TransportSelector::Any
                }
            } else if let Some(serial) = service.strip_prefix("transport:") {
                TransportSelector::Serial(serial.to_string())
            } else if let Some(id_str) = service.strip_prefix("transport-id:") {
                if let Ok(id) = id_str.parse::<u64>() {
                    match TransportId::from_u64(id) {
                        Some(tid) => TransportSelector::Id(tid),
                        None => TransportSelector::Any,
                    }
                } else {
                    TransportSelector::Any
                }
            } else if service == "transport-usb" {
                TransportSelector::Usb
            } else if service == "transport-local" {
                TransportSelector::Local
            } else {
                TransportSelector::Any
            };
            Ok(Self::Transport(selector))
        } else {
            Ok(Self::Other(s.to_string()))
        }
    }
}

impl fmt::Display for HostService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Version => write!(f, "host:version"),
            Self::Kill => write!(f, "host:kill"),
            Self::Devices { long: false } => write!(f, "host:devices"),
            Self::Devices { long: true } => write!(f, "host:devices-l"),
            Self::TrackDevices(mode) => write!(f, "host:{}", mode.service_name()),
            Self::Features => write!(f, "host:features"),
            Self::HostFeatures => write!(f, "host-features"),
            Self::GetState => write!(f, "get-state"),
            Self::GetSerialno => write!(f, "get-serialno"),
            Self::GetDevpath => write!(f, "get-devpath"),
            Self::TransportId => write!(f, "transport-id"),
            Self::Connect(t) => write!(f, "host:connect:{t}"),
            Self::Disconnect(t) => match t {
                Some(serial) => write!(f, "host:disconnect:{serial}"),
                None => write!(f, "host:disconnect:"),
            },
            Self::Transport(sel) => match sel {
                TransportSelector::Any => write!(f, "host:transport-any"),
                TransportSelector::Usb => write!(f, "host:transport-usb"),
                TransportSelector::Local => write!(f, "host:transport-local"),
                TransportSelector::Serial(s) => write!(f, "host:transport:{s}"),
                TransportSelector::Id(id) => write!(f, "host:transport-id:{id}"),
            },
            Self::ForwardList => write!(f, "host:list-forward"),
            Self::ForwardKillAll => write!(f, "host:killforward-all"),
            Self::ForwardKill(local) => write!(f, "host:killforward:{local}"),
            Self::ForwardAdd {
                no_rebind,
                local,
                remote,
            } => {
                if *no_rebind {
                    write!(f, "host:forward:norebind:{local};{remote}")
                } else {
                    write!(f, "host:forward:{local};{remote}")
                }
            }
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

/// Target device services executed on the connected target device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Service {
    Shell(String),
    Exec(String),
    Sync,
    Abb(String),
    AbbExec(String),
    Remount(String),
    Dev(String),
    DevRaw(String),
    Reverse(String),
    Other(String),
}

impl FromStr for Service {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(cmd) = s.strip_prefix("shell:") {
            Ok(Self::Shell(cmd.to_string()))
        } else if let Some(cmd) = s.strip_prefix("shell,") {
            Ok(Self::Shell(format!(",{}", cmd)))
        } else if s == "shell" {
            Ok(Self::Shell(String::new()))
        } else if let Some(cmd) = s.strip_prefix("exec:") {
            Ok(Self::Exec(cmd.to_string()))
        } else if s == "sync:" || s == "sync" {
            Ok(Self::Sync)
        } else if let Some(cmd) = s.strip_prefix("abb:") {
            Ok(Self::Abb(cmd.to_string()))
        } else if let Some(cmd) = s.strip_prefix("abb_exec:") {
            Ok(Self::AbbExec(cmd.to_string()))
        } else if let Some(cmd) = s.strip_prefix("remount:") {
            Ok(Self::Remount(cmd.to_string()))
        } else if let Some(dev) = s.strip_prefix("dev:") {
            Ok(Self::Dev(dev.to_string()))
        } else if let Some(dev) = s.strip_prefix("dev-raw:") {
            Ok(Self::DevRaw(dev.to_string()))
        } else if let Some(rev) = s.strip_prefix("reverse:") {
            Ok(Self::Reverse(rev.to_string()))
        } else {
            Ok(Self::Other(s.to_string()))
        }
    }
}

impl fmt::Display for Service {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shell(cmd) => {
                if cmd.starts_with(',') {
                    write!(f, "shell{cmd}")
                } else if cmd.is_empty() {
                    write!(f, "shell")
                } else {
                    write!(f, "shell:{cmd}")
                }
            }
            Self::Exec(cmd) => write!(f, "exec:{cmd}"),
            Self::Sync => write!(f, "sync:"),
            Self::Abb(cmd) => write!(f, "abb:{cmd}"),
            Self::AbbExec(cmd) => write!(f, "abb_exec:{cmd}"),
            Self::Remount(cmd) => write!(f, "remount:{cmd}"),
            Self::Dev(dev) => write!(f, "dev:{dev}"),
            Self::DevRaw(dev) => write!(f, "dev-raw:{dev}"),
            Self::Reverse(rev) => write!(f, "reverse:{rev}"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}
