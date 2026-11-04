//! Integration and unit tests for `UsbDeviceScanner`.

use adb_server::{DeviceRegistry, TrackerFormat, TransportType, UsbDeviceScanner};
use adb_transport::{
    EndpointDirection, EndpointType, MockUsbBackend, UsbDeviceInfo, UsbEndpointInfo,
    UsbInterfaceInfo, ADB_CLASS, ADB_PROTOCOL, ADB_SUBCLASS,
};
use std::sync::Arc;

fn make_adb_interface() -> UsbInterfaceInfo {
    UsbInterfaceInfo {
        interface_number: 0,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![
            UsbEndpointInfo::new(0x01, EndpointDirection::Out, EndpointType::Bulk, 512),
            UsbEndpointInfo::new(0x81, EndpointDirection::In, EndpointType::Bulk, 512),
        ],
    }
}

fn make_non_adb_interface() -> UsbInterfaceInfo {
    UsbInterfaceInfo {
        interface_number: 0,
        class: 0x08, // Mass Storage
        subclass: 0x06,
        protocol: 0x50,
        endpoints: vec![
            UsbEndpointInfo::new(0x01, EndpointDirection::Out, EndpointType::Bulk, 512),
            UsbEndpointInfo::new(0x81, EndpointDirection::In, EndpointType::Bulk, 512),
        ],
    }
}

#[tokio::test]
async fn test_usb_scanner_one_matching_device() {
    let mock_backend = Arc::new(MockUsbBackend::new());
    let registry = Arc::new(DeviceRegistry::new());

    let dev = UsbDeviceInfo {
        bus_number: 1,
        device_address: 2,
        device_path: "usb:1-2".to_string(),
        vendor_id: 0x18d1,
        product_id: 0x4ee7,
        serial: Some("SERIAL123".to_string()),
        interfaces: vec![make_adb_interface()],
    };

    mock_backend.set_devices(vec![dev]).await;

    let scanner = UsbDeviceScanner::new(mock_backend, Arc::clone(&registry));
    let registered = scanner.scan_once().await.expect("scan success");
    assert_eq!(registered, 1);

    let dev_entry = registry
        .get_by_serial("SERIAL123")
        .expect("device registered");
    assert_eq!(dev_entry.transport_type, TransportType::Usb);
    assert_eq!(dev_entry.devpath.as_deref(), Some("usb:1-2"));

    // Check devices -l formatted output contains serial and devpath
    let devices = registry.list_devices();
    let framed = adb_server::format_devices_raw(&devices, TrackerFormat::LongText);
    let str_out = String::from_utf8_lossy(&framed);
    assert!(str_out.contains("SERIAL123"));
    assert!(str_out.contains("usb:1-2"));
}

#[tokio::test]
async fn test_usb_scanner_two_devices_same_serial_handled_per_cpp() {
    let mock_backend = Arc::new(MockUsbBackend::new());
    let registry = Arc::new(DeviceRegistry::new());

    let dev1 = UsbDeviceInfo {
        bus_number: 1,
        device_address: 2,
        device_path: "usb:1-2".to_string(),
        vendor_id: 0x18d1,
        product_id: 0x4ee7,
        serial: Some("DUPLICATE_SN".to_string()),
        interfaces: vec![make_adb_interface()],
    };

    let dev2 = UsbDeviceInfo {
        bus_number: 1,
        device_address: 3,
        device_path: "usb:1-3".to_string(),
        vendor_id: 0x18d1,
        product_id: 0x4ee7,
        serial: Some("DUPLICATE_SN".to_string()),
        interfaces: vec![make_adb_interface()],
    };

    mock_backend.set_devices(vec![dev1, dev2]).await;

    let scanner = UsbDeviceScanner::new(mock_backend, Arc::clone(&registry));
    let registered = scanner.scan_once().await.expect("scan success");
    // Second device with same serial is rejected as duplicate per C++ validate_transport_list
    assert_eq!(registered, 1);

    let dev_entry = registry
        .get_by_serial("DUPLICATE_SN")
        .expect("entry exists");
    assert_eq!(dev_entry.devpath.as_deref(), Some("usb:1-2"));
}

#[tokio::test]
async fn test_usb_scanner_non_adb_device_ignored() {
    let mock_backend = Arc::new(MockUsbBackend::new());
    let registry = Arc::new(DeviceRegistry::new());

    let non_adb = UsbDeviceInfo {
        bus_number: 1,
        device_address: 5,
        device_path: "usb:1-5".to_string(),
        vendor_id: 0x058f,
        product_id: 0x6387,
        serial: Some("FLASH_DRIVE".to_string()),
        interfaces: vec![make_non_adb_interface()],
    };

    mock_backend.set_devices(vec![non_adb]).await;

    let scanner = UsbDeviceScanner::new(mock_backend, Arc::clone(&registry));
    let registered = scanner.scan_once().await.expect("scan success");
    assert_eq!(registered, 0);

    assert!(registry.get_by_serial("FLASH_DRIVE").is_none());
}

#[tokio::test]
async fn test_usb_devices_sort_before_local_in_tracker() {
    let registry = Arc::new(DeviceRegistry::new());

    // Register a local device first
    registry
        .register("127.0.0.1:5555", TransportType::Local)
        .expect("register local");

    // Register a USB device second
    registry
        .register_with_devpath(
            "USB_DEVICE_A",
            TransportType::Usb,
            Some("usb:1-1".to_string()),
        )
        .expect("register usb");

    let devices = registry.list_devices();
    let framed = adb_server::format_devices_raw(&devices, TrackerFormat::ShortText);
    let out = String::from_utf8_lossy(&framed);

    // ShortText should format USB device before local device
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("USB_DEVICE_A\t"));
    assert!(lines[1].starts_with("127.0.0.1:5555\t"));
}
