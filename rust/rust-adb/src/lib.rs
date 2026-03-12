//! ADB client library and CLI implementation.
//! Ported from `client/main.cpp`, `commandline.cpp`, and `adb_client.cpp`.

/// ADB client connection and command execution.
pub mod adb_client;
/// Bugreport command implementation.
pub mod bugreport;
/// Root and restart command implementation.
pub mod root;
/// Logcat command implementation.
pub mod logcat;
/// Forward and reverse command implementation.
pub mod forward;
/// Install and uninstall command implementation.
pub mod adb_install;
/// Sideload and rescue command implementation.
pub mod sideload;
/// File sync command implementation.
pub mod sync;
/// Pairing command implementation.
pub mod pairing;
