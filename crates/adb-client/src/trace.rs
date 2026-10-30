//! Client-side tracing module.
//!
//! Cites category table and trace mask initialization from `original/adb_trace.cpp`
//! and `original/adb_trace.h`.

use std::sync::atomic::{AtomicU32, Ordering};

static TRACE_MASK: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TraceCategory {
    Adb = 0,
    Sockets = 1,
    Packets = 2,
    Transport = 3,
    Rwx = 4,
    Usb = 5,
    Sync = 6,
    Sysdeps = 7,
    Jdwp = 8,
    Services = 9,
    Auth = 10,
    Fdevent = 11,
    Shell = 12,
    Incremental = 13,
    Mdns = 14,
    MdnsStack = 15,
}

impl TraceCategory {
    pub const fn bit_flag(self) -> u32 {
        1 << (self as u32)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Adb => "adb",
            Self::Sockets => "sockets",
            Self::Packets => "packets",
            Self::Transport => "transport",
            Self::Rwx => "rwx",
            Self::Usb => "usb",
            Self::Sync => "sync",
            Self::Sysdeps => "sysdeps",
            Self::Jdwp => "jdwp",
            Self::Services => "services",
            Self::Auth => "auth",
            Self::Fdevent => "fdevent",
            Self::Shell => "shell",
            Self::Incremental => "incremental",
            Self::Mdns => "mdns",
            Self::MdnsStack => "mdns-stack",
        }
    }
}

/// Initializes trace setting mask from string value.
///
/// Ports `setup_trace_mask` from `original/adb_trace.cpp`.
/// Returns a list of unknown trace flag error messages if any were found.
pub fn trace_init(setting: &str) -> Vec<String> {
    if setting.trim().is_empty() {
        TRACE_MASK.store(0, Ordering::Relaxed);
        return Vec::new();
    }

    let mut mask = 0u32;
    let mut errors = Vec::new();

    let elements = setting.split([',', ' ']);
    for raw in elements {
        let elem = raw.trim();
        if elem.is_empty() {
            continue;
        }

        match elem {
            "1" | "all" => {
                mask = !0;
                break;
            }
            "adb" => mask |= TraceCategory::Adb.bit_flag(),
            "sockets" => mask |= TraceCategory::Sockets.bit_flag(),
            "packets" => mask |= TraceCategory::Packets.bit_flag(),
            "rwx" => mask |= TraceCategory::Rwx.bit_flag(),
            "usb" => mask |= TraceCategory::Usb.bit_flag(),
            "sync" => mask |= TraceCategory::Sync.bit_flag(),
            "sysdeps" => mask |= TraceCategory::Sysdeps.bit_flag(),
            "transport" => mask |= TraceCategory::Transport.bit_flag(),
            "jdwp" => mask |= TraceCategory::Jdwp.bit_flag(),
            "services" => mask |= TraceCategory::Services.bit_flag(),
            "auth" => mask |= TraceCategory::Auth.bit_flag(),
            "fdevent" => mask |= TraceCategory::Fdevent.bit_flag(),
            "shell" => mask |= TraceCategory::Shell.bit_flag(),
            "incremental" => mask |= TraceCategory::Incremental.bit_flag(),
            "mdns" => mask |= TraceCategory::Mdns.bit_flag(),
            "mdns-stack" => mask |= TraceCategory::MdnsStack.bit_flag(),
            _ => {
                errors.push(format!("Unknown trace flag: {elem}"));
            }
        }
    }

    TRACE_MASK.store(mask, Ordering::Relaxed);
    errors
}

/// Checks whether a trace category is enabled.
pub fn trace_enabled(category: TraceCategory) -> bool {
    (TRACE_MASK.load(Ordering::Relaxed) & category.bit_flag()) != 0
}

/// Formats and prints a trace message if category is enabled.
pub fn trace_log(category: TraceCategory, message: &str) {
    if trace_enabled(category) {
        let pid = std::process::id();
        eprintln!("[{pid}] [{}] {message}", category.name());
    }
}

#[macro_export]
macro_rules! adb_trace {
    ($category:expr, $($arg:tt)*) => {
        if $crate::trace::trace_enabled($category) {
            $crate::trace::trace_log($category, &format!($($arg)*));
        }
    };
}
