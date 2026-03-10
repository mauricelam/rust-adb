//! ADB USB transport implementation.
//! Ported from `client/usb_libusb.cpp` and `daemon/usb.cpp`.

/// Host-side USB transport using libusb.
pub mod host;

/// Daemon-side USB transport using FunctionFS.
#[cfg(target_os = "linux")]
pub mod daemon;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_structure() {
        // Just verify it compiles and we can access constants
        assert_eq!(host::ADB_CLASS, 0xff);
    }
}
