//! A tracing library for adb, inspired by the C++ implementation.
//!
//! This library provides a way to enable tagged tracing via the `ADB_TRACE`
//! environment variable.
//!
//! # Usage
//!
//! First, initialize the logger by calling `adb_trace_init()` at the start
//! of your program.
//!
//! Then, use the `log` crate's macros with the appropriate target to log
//! messages. The target should be the lowercase version of the `AdbTrace`
//! enum variant.
//!
//! ```
//! use log::trace;
//! use trace::adb_trace_init;
//!
//! fn main() {
//!     adb_trace_init();
//!     trace!(target: "adb", "This is an adb trace message");
//!     trace!(target: "sockets", "This is a sockets trace message");
//! }
//! ```
//!
//! To enable tracing, set the `ADB_TRACE` environment variable to a
//! comma-separated list of tags. For example:
//!
//! ```sh
//! ADB_TRACE=adb,sockets cargo run
//! ```
//!
//! The special values "1" and "all" can be used to enable all traces.

use log::LevelFilter;
use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdbTrace {
    Adb,
    Sockets,
    Packets,
    Transport,
    Rwx,
    Usb,
    Sync,
    Sysdeps,
    Jdwp,
    Services,
    Auth,
    Fdevent,
    Shell,
    Incremental,
    Mdns,
    MdnsStack,
}

impl AdbTrace {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdbTrace::Adb => "adb",
            AdbTrace::Sockets => "sockets",
            AdbTrace::Packets => "packets",
            AdbTrace::Transport => "transport",
            AdbTrace::Rwx => "rwx",
            AdbTrace::Usb => "usb",
            AdbTrace::Sync => "sync",
            AdbTrace::Sysdeps => "sysdeps",
            AdbTrace::Jdwp => "jdwp",
            AdbTrace::Services => "services",
            AdbTrace::Auth => "auth",
            AdbTrace::Fdevent => "fdevent",
            AdbTrace::Shell => "shell",
            AdbTrace::Incremental => "incremental",
            AdbTrace::Mdns => "mdns",
            AdbTrace::MdnsStack => "mdns_stack",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "adb" => Some(AdbTrace::Adb),
            "sockets" => Some(AdbTrace::Sockets),
            "packets" => Some(AdbTrace::Packets),
            "transport" => Some(AdbTrace::Transport),
            "rwx" => Some(AdbTrace::Rwx),
            "usb" => Some(AdbTrace::Usb),
            "sync" => Some(AdbTrace::Sync),
            "sysdeps" => Some(AdbTrace::Sysdeps),
            "jdwp" => Some(AdbTrace::Jdwp),
            "services" => Some(AdbTrace::Services),
            "auth" => Some(AdbTrace::Auth),
            "fdevent" => Some(AdbTrace::Fdevent),
            "shell" => Some(AdbTrace::Shell),
            "incremental" => Some(AdbTrace::Incremental),
            "mdns" => Some(AdbTrace::Mdns),
            "mdns_stack" => Some(AdbTrace::MdnsStack),
            _ => None,
        }
    }

    pub fn all_tags() -> Vec<Self> {
        vec![
            AdbTrace::Adb,
            AdbTrace::Sockets,
            AdbTrace::Packets,
            AdbTrace::Transport,
            AdbTrace::Rwx,
            AdbTrace::Usb,
            AdbTrace::Sync,
            AdbTrace::Sysdeps,
            AdbTrace::Jdwp,
            AdbTrace::Services,
            AdbTrace::Auth,
            AdbTrace::Fdevent,
            AdbTrace::Shell,
            AdbTrace::Incremental,
            AdbTrace::Mdns,
            AdbTrace::MdnsStack,
        ]
    }
}

/// Initializes the tracing system.
///
/// This function reads the `ADB_TRACE` environment variable and configures
/// the `env_logger` backend to show trace messages for the specified tags.
pub fn adb_trace_init() {
    let trace_setting = env::var("ADB_TRACE").unwrap_or_default();
    if trace_setting.is_empty() {
        return;
    }

    let mut builder = env_logger::Builder::new();
    builder.filter(None, LevelFilter::Info); // Default level

    let tags = trace_setting.split(|c| c == ',' || c == ' ').collect::<Vec<_>>();

    if tags.contains(&"1") || tags.contains(&"all") {
        for tag in AdbTrace::all_tags() {
            builder.filter(Some(tag.as_str()), LevelFilter::Trace);
        }
    } else {
        for tag_str in tags {
            if let Some(tag) = AdbTrace::from_str(tag_str) {
                builder.filter(Some(tag.as_str()), LevelFilter::Trace);
            }
        }
    }

    builder.try_init().ok();
}
