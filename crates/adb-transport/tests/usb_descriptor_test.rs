//! Unit tests for USB descriptor matching, UsbBackend abstractions, and MockUsbBackend.
//!
//! Note on real device dumps: Real descriptor fixtures should be placed in `tests/fixtures/descriptors/`
//! and require maintainer review before inclusion.

use adb_transport::{
    find_adb_interface, is_adb_interface, EndpointDirection, EndpointType, MockUsbBackend,
    UsbBackend, UsbDeviceInfo, UsbEndpointInfo, UsbError, UsbInterfaceInfo, ADB_CLASS,
    ADB_DBC_CLASS, ADB_DBC_SUBCLASS, ADB_PROTOCOL, ADB_SUBCLASS,
};

#[test]
fn test_is_adb_interface() {
    // Valid standard ADB interface
    assert!(is_adb_interface(ADB_CLASS, ADB_SUBCLASS, ADB_PROTOCOL));
    assert!(is_adb_interface(0xFF, 0x42, 0x01));

    // Valid DbC debug interface
    assert!(is_adb_interface(
        ADB_DBC_CLASS,
        ADB_DBC_SUBCLASS,
        ADB_PROTOCOL
    ));
    assert!(is_adb_interface(0xDC, 0x02, 0x01));

    // Invalid class
    assert!(!is_adb_interface(0x00, 0x42, 0x01));
    assert!(!is_adb_interface(0xFE, 0x42, 0x01));

    // Invalid subclass
    assert!(!is_adb_interface(0xFF, 0x43, 0x01));
    assert!(!is_adb_interface(0xFF, 0x00, 0x01));

    // Invalid protocol
    assert!(!is_adb_interface(0xFF, 0x42, 0x02));
    assert!(!is_adb_interface(0xFF, 0x42, 0x00));

    // Invalid DbC combinations
    assert!(!is_adb_interface(0xDC, 0x03, 0x01));
    assert!(!is_adb_interface(0xDD, 0x02, 0x01));
    assert!(!is_adb_interface(0xDC, 0x02, 0x02));
}

#[test]
fn test_find_adb_interface_single_matching() {
    let iface = UsbInterfaceInfo {
        interface_number: 0,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![
            UsbEndpointInfo::new(0x01, EndpointDirection::Out, EndpointType::Bulk, 512),
            UsbEndpointInfo::new(0x81, EndpointDirection::In, EndpointType::Bulk, 512),
        ],
    };

    let matched = find_adb_interface(&[iface]).expect("must match ADB interface");
    assert_eq!(matched.interface_number, 0);
    assert_eq!(matched.bulk_out_address, 0x01);
    assert_eq!(matched.bulk_out_max_packet_size, 512);
    assert_eq!(matched.bulk_in_address, 0x81);
    assert_eq!(matched.bulk_in_max_packet_size, 512);
}

#[test]
fn test_find_adb_interface_composite_device() {
    // Interface 0: MTP / non-ADB (Class 0x06 Imaging)
    let mtp_iface = UsbInterfaceInfo {
        interface_number: 0,
        class: 0x06,
        subclass: 0x01,
        protocol: 0x01,
        endpoints: vec![
            UsbEndpointInfo::new(0x01, EndpointDirection::Out, EndpointType::Bulk, 512),
            UsbEndpointInfo::new(0x81, EndpointDirection::In, EndpointType::Bulk, 512),
        ],
    };

    // Interface 1: ADB (Class 0xFF, Subclass 0x42, Protocol 0x01)
    let adb_iface = UsbInterfaceInfo {
        interface_number: 1,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![
            UsbEndpointInfo::new(0x82, EndpointDirection::In, EndpointType::Bulk, 1024),
            UsbEndpointInfo::new(0x02, EndpointDirection::Out, EndpointType::Bulk, 1024),
        ],
    };

    let matched =
        find_adb_interface(&[mtp_iface, adb_iface]).expect("must find ADB in composite device");
    assert_eq!(matched.interface_number, 1);
    assert_eq!(matched.bulk_out_address, 0x02);
    assert_eq!(matched.bulk_out_max_packet_size, 1024);
    assert_eq!(matched.bulk_in_address, 0x82);
    assert_eq!(matched.bulk_in_max_packet_size, 1024);
}

#[test]
fn test_find_adb_interface_missing_bulk_endpoints() {
    // Matching interface class/subclass/protocol but no endpoints
    let no_eps = UsbInterfaceInfo {
        interface_number: 0,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![],
    };
    assert!(find_adb_interface(&[no_eps]).is_none());

    // Only OUT endpoint
    let only_out = UsbInterfaceInfo {
        interface_number: 0,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![UsbEndpointInfo::new(
            0x01,
            EndpointDirection::Out,
            EndpointType::Bulk,
            512,
        )],
    };
    assert!(find_adb_interface(&[only_out]).is_none());

    // Only IN endpoint
    let only_in = UsbInterfaceInfo {
        interface_number: 0,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![UsbEndpointInfo::new(
            0x81,
            EndpointDirection::In,
            EndpointType::Bulk,
            512,
        )],
    };
    assert!(find_adb_interface(&[only_in]).is_none());

    // Interrupt endpoints instead of bulk
    let interrupt_eps = UsbInterfaceInfo {
        interface_number: 0,
        class: ADB_CLASS,
        subclass: ADB_SUBCLASS,
        protocol: ADB_PROTOCOL,
        endpoints: vec![
            UsbEndpointInfo::new(0x01, EndpointDirection::Out, EndpointType::Interrupt, 64),
            UsbEndpointInfo::new(0x81, EndpointDirection::In, EndpointType::Interrupt, 64),
        ],
    };
    assert!(find_adb_interface(&[interrupt_eps]).is_none());
}

#[tokio::test]
async fn test_mock_backend_scripted_failures_and_operations() {
    let mock = MockUsbBackend::new();

    let dev = UsbDeviceInfo {
        bus_number: 1,
        device_address: 2,
        device_path: "usb:1-2".to_string(),
        vendor_id: 0x18D1,
        product_id: 0x4EE7,
        serial: Some("TEST_SERIAL_123".to_string()),
        interfaces: vec![],
    };

    mock.set_devices(vec![dev.clone()]).await;

    let list = mock.list_devices().await.expect("list devices");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].serial.as_deref(), Some("TEST_SERIAL_123"));

    // Script list error
    mock.set_list_error(Some(UsbError::PermissionDenied)).await;
    let err = mock.list_devices().await.unwrap_err();
    assert_eq!(err, UsbError::PermissionDenied);

    mock.set_list_error(None).await;

    // Script open error
    mock.set_open_error(Some(UsbError::Disconnected)).await;
    let open_err = mock.open(&dev).await.unwrap_err();
    assert_eq!(open_err, UsbError::Disconnected);

    mock.set_open_error(None).await;
    let handle = mock.open(&dev).await.expect("open success");

    // Test handle operations
    assert_eq!(handle.bulk_write(0x01, b"hello adb").await.unwrap(), 9);
    assert!(handle.clear_halt(0x01).await.is_ok());
    assert!(handle.close().await.is_ok());
}
