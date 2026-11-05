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

#[tokio::test]
async fn test_hotplug_events_attach_and_detach_updates_registry_and_tracker() {
    let mock_backend = Arc::new(MockUsbBackend::new());
    let registry = Arc::new(DeviceRegistry::new());

    // Subscribe to device tracker
    let mut tracker_rx = registry.subscribe(TrackerFormat::ShortText);
    // Read initial empty tracker state
    let initial = tracker_rx.recv().await.expect("initial tracker");
    assert_eq!(initial, b"0000");

    let scanner = Arc::new(UsbDeviceScanner::with_debounce(
        Arc::clone(&mock_backend) as Arc<dyn adb_transport::UsbBackend>,
        Arc::clone(&registry),
        std::time::Duration::from_millis(5),
    ));

    // Spawn scanner in background
    let scanner_clone = Arc::clone(&scanner);
    let run_handle = tokio::spawn(async move {
        scanner_clone.run(std::time::Duration::from_millis(20)).await;
    });

    let dev = UsbDeviceInfo {
        bus_number: 1,
        device_address: 4,
        device_path: "usb:1-4".to_string(),
        vendor_id: 0x18d1,
        product_id: 0x4ee7,
        serial: Some("HOTPLUG_DEVICE_1".to_string()),
        interfaces: vec![make_adb_interface()],
    };

    // 1. Device appears after server starts
    mock_backend.set_devices(vec![dev.clone()]).await;
    mock_backend
        .emit_event(adb_transport::UsbHotplugEvent::Attached(dev.clone()))
        .await;

    // Tracker should receive update containing the new device
    let update = tracker_rx.recv().await.expect("tracker update on attach");
    let update_str = String::from_utf8_lossy(&update);
    assert!(
        update_str.contains("HOTPLUG_DEVICE_1\toffline"),
        "tracker received device arrival"
    );

    let reg_dev = registry
        .get_by_serial("HOTPLUG_DEVICE_1")
        .expect("device in registry");
    assert_eq!(reg_dev.devpath.as_deref(), Some("usb:1-4"));

    // 2. Device disappears
    mock_backend.set_devices(vec![]).await;
    mock_backend
        .emit_event(adb_transport::UsbHotplugEvent::Detached("usb:1-4".to_string()))
        .await;

    // Tracker should receive update indicating empty device list
    let detach_update = tracker_rx.recv().await.expect("tracker update on detach");
    let detach_str = String::from_utf8_lossy(&detach_update);
    assert_eq!(detach_str, "0000", "tracker empty on detach");
    assert!(registry.get_by_serial("HOTPLUG_DEVICE_1").is_none());

    run_handle.abort();
}

#[tokio::test]
async fn test_hotplug_debounce_rapid_attach_detach_leaves_clean_state() {
    let mock_backend = Arc::new(MockUsbBackend::new());
    let registry = Arc::new(DeviceRegistry::new());

    let scanner = UsbDeviceScanner::with_debounce(
        Arc::clone(&mock_backend) as Arc<dyn adb_transport::UsbBackend>,
        Arc::clone(&registry),
        std::time::Duration::from_millis(30),
    );

    let dev = UsbDeviceInfo {
        bus_number: 1,
        device_address: 6,
        device_path: "usb:1-6".to_string(),
        vendor_id: 0x18d1,
        product_id: 0x4ee7,
        serial: Some("FLAPPING_DEV".to_string()),
        interfaces: vec![make_adb_interface()],
    };

    // Device appears briefly, but disappears before debounce completes
    mock_backend.set_devices(vec![]).await; // already gone in backend
    scanner
        .process_hotplug_event(adb_transport::UsbHotplugEvent::Attached(dev))
        .await;

    // Due to debounce check, device should NOT be registered
    assert!(registry.get_by_serial("FLAPPING_DEV").is_none());
}

#[tokio::test]
async fn test_mutant_ignoring_detach_fails() {
    let registry = Arc::new(DeviceRegistry::new());
    registry
        .register_with_devpath("DETACH_TEST", TransportType::Usb, Some("usb:1-9".to_string()))
        .unwrap();

    let mock_backend = Arc::new(MockUsbBackend::new());
    let scanner = UsbDeviceScanner::with_debounce(
        mock_backend,
        Arc::clone(&registry),
        std::time::Duration::from_millis(0),
    );

    // If detach is processed, device is unregistered
    scanner
        .process_hotplug_event(adb_transport::UsbHotplugEvent::Detached("usb:1-9".to_string()))
        .await;
    assert!(
        registry.get_by_serial("DETACH_TEST").is_none(),
        "detach event must unregister the device"
    );
}
