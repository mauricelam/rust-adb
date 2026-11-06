//! Physical hardware validation test for `NusbBackend`.
//!
//! Run with an attached Android device:
//! ```bash
//! cargo test -p adb-transport --features usb --test usb_hw_test -- --ignored --nocapture
//! ```

#[cfg(feature = "usb")]
#[tokio::test]
#[ignore]
async fn test_nusb_list_devices_real_hardware() {
    use adb_transport::{find_adb_interface, NusbBackend, UsbBackend};

    let backend = NusbBackend::new();
    let devices = backend
        .list_devices()
        .await
        .expect("must enumerate USB devices");

    println!("Found {} USB devices on system", devices.len());

    let mut found_adb_count = 0;
    for dev in devices {
        if let Some(adb_match) = find_adb_interface(&dev.interfaces) {
            println!(
                "ADB device found: serial={:?}, path={}, interface={}, bulk_in=0x{:02x}, bulk_out=0x{:02x}",
                dev.serial,
                dev.device_path,
                adb_match.interface_number,
                adb_match.bulk_in_address,
                adb_match.bulk_out_address
            );
            assert!(!dev.device_path.is_empty(), "device path must not be empty");
            assert!(
                adb_match.bulk_in_address != 0,
                "must have valid bulk in endpoint"
            );
            assert!(
                adb_match.bulk_out_address != 0,
                "must have valid bulk out endpoint"
            );
            found_adb_count += 1;
        }
    }

    println!("Total ADB devices recognized: {}", found_adb_count);
}

/// macOS smoke test verifying IOKit enumeration of ADB devices.
#[cfg(feature = "usb")]
#[tokio::test]
#[ignore]
async fn test_macos_usb_device_enumeration() {
    use adb_transport::{find_adb_interface, NusbBackend, UsbBackend};

    let backend = NusbBackend::new();
    let devices = backend
        .list_devices()
        .await
        .expect("must enumerate USB devices on macOS");

    println!("macOS USB Enumeration: found {} devices", devices.len());
    let mut adb_found = 0;
    for dev in devices {
        if let Some(adb_match) = find_adb_interface(&dev.interfaces) {
            println!(
                "macOS ADB device: serial={:?}, path={}, iface={}, in=0x{:02x}, out=0x{:02x}",
                dev.serial,
                dev.device_path,
                adb_match.interface_number,
                adb_match.bulk_in_address,
                adb_match.bulk_out_address
            );
            adb_found += 1;
        }
    }
    println!("macOS total ADB interfaces: {}", adb_found);
}

/// Windows smoke test verifying WinUSB enumeration of ADB devices.
#[cfg(feature = "usb")]
#[tokio::test]
#[ignore]
async fn test_windows_winusb_device_enumeration() {
    use adb_transport::{find_adb_interface, NusbBackend, UsbBackend};

    let backend = NusbBackend::new();
    let devices = backend
        .list_devices()
        .await
        .expect("must enumerate USB devices on Windows");

    println!("Windows USB Enumeration: found {} devices", devices.len());
    let mut adb_found = 0;
    for dev in devices {
        if let Some(adb_match) = find_adb_interface(&dev.interfaces) {
            println!(
                "Windows ADB device: serial={:?}, path={}, iface={}, in=0x{:02x}, out=0x{:02x}",
                dev.serial,
                dev.device_path,
                adb_match.interface_number,
                adb_match.bulk_in_address,
                adb_match.bulk_out_address
            );
            adb_found += 1;
        }
    }
    println!("Windows total ADB interfaces: {}", adb_found);
}
