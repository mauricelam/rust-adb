//! Protocol-wide limits, version numbers, and payload size constants.
//!
//! Ports constants from `original/adb.h`.

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
/// Ports `MAX_PAYLOAD_V1` (4 KiB) from `original/adb.h`.
pub const MAX_PAYLOAD_V1: usize = 4 * 1024;

/// Maximum payload size supported by modern ADB protocol (`MAX_PAYLOAD`).
/// Ports `MAX_PAYLOAD` (1 MiB) from `original/adb.h`.
pub const MAX_PAYLOAD: usize = 1024 * 1024;
