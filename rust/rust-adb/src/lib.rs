//! ADB client library and CLI implementation.
//! Ported from `client/main.cpp`, `commandline.cpp`, and `adb_client.cpp`.

/// ADB client connection and command execution.
pub mod adb_client;
/// Installation and uninstallation logic.
pub mod adb_install;
/// Logcat command implementation.
pub mod logcat;
/// Bugreport command implementation.
pub mod bugreport;
/// Sideload and rescue command implementation.
pub mod sideload;
