use std::sync::Arc;
use std::time::Duration;

use adb_server::{ConnectionState, DeviceRegistry, TrackerFormat, TransportType};

#[tokio::test]
async fn test_tracker_initial_empty() {
    let registry = Arc::new(DeviceRegistry::new());

    // Normal (short text): empty list is "0000"
    let mut rx_short = registry.subscribe(TrackerFormat::ShortText);
    let initial = rx_short.recv().await.expect("initial msg");
    assert_eq!(initial, b"0000");

    // Long text: empty list is "0000"
    let mut rx_long = registry.subscribe(TrackerFormat::LongText);
    let initial_long = rx_long.recv().await.expect("initial msg");
    assert_eq!(initial_long, b"0000");

    // Binary proto: empty list is "0000"
    let mut rx_proto = registry.subscribe(TrackerFormat::ProtoBinary);
    let initial_proto = rx_proto.recv().await.expect("initial msg");
    assert_eq!(initial_proto, b"0000");

    // Text proto: empty list is "0000"
    let mut rx_proto_txt = registry.subscribe(TrackerFormat::ProtoText);
    let initial_proto_txt = rx_proto_txt.recv().await.expect("initial msg");
    assert_eq!(initial_proto_txt, b"0000");
}

#[tokio::test]
async fn test_tracker_device_connect_and_state_change() {
    let registry = Arc::new(DeviceRegistry::new());

    let mut rx = registry.subscribe(TrackerFormat::ShortText);
    assert_eq!(rx.recv().await.unwrap(), b"0000");

    // Device connects (initial state Offline)
    let id = registry
        .register("127.0.0.1:55555", TransportType::Local)
        .expect("register succeeds");

    let notification = rx.recv().await.expect("notification after register");
    // "127.0.0.1:55555\toffline\n" is 24 bytes (0x0018)
    assert_eq!(notification, b"0018127.0.0.1:55555\toffline\n");

    // Device state changes to Authorizing
    registry.update_state(id, ConnectionState::Authorizing);
    let notification = rx.recv().await.expect("notification after authorizing");
    // "127.0.0.1:55555\tauthorizing\n" is 28 bytes (0x001c)
    assert_eq!(notification, b"001c127.0.0.1:55555\tauthorizing\n");

    // Device state changes to Device
    registry.update_state(id, ConnectionState::Device);
    let notification = rx.recv().await.expect("notification after device");
    // "127.0.0.1:55555\tdevice\n" is 23 bytes (0x0017)
    assert_eq!(notification, b"0017127.0.0.1:55555\tdevice\n");

    // Device disconnects
    registry.unregister_by_id(id);
    let notification = rx.recv().await.expect("notification after disconnect");
    assert_eq!(notification, b"0000");
}

#[tokio::test]
async fn test_tracker_multi_format_and_ordering() {
    let registry = Arc::new(DeviceRegistry::new());

    // Register 2 devices in reverse order:
    // device 2 is Local transport
    let id2 = registry
        .register("127.0.0.1:55556", TransportType::Local)
        .unwrap();
    registry.update_state(id2, ConnectionState::Device);
    registry.set_device_properties(
        id2,
        Some("mock_device".into()),
        Some("mock_model".into()),
        Some("mock_device".into()),
    );

    // device 1 is Usb transport (USB sorts before Local in upstream ADB: kTransportUsb < kTransportLocal)
    let id1 = registry.register("usb-serial", TransportType::Usb).unwrap();
    registry.update_state(id1, ConnectionState::Device);
    registry.set_device_properties(
        id1,
        Some("pixel".into()),
        Some("Pixel 7".into()),
        Some("panther".into()),
    );

    // 1. Short Text: USB device first, then Local device
    let mut rx_short = registry.subscribe(TrackerFormat::ShortText);
    let short_payload = rx_short.recv().await.unwrap();
    let short_str = String::from_utf8(short_payload[4..].to_vec()).unwrap();
    assert_eq!(short_str, "usb-serial\tdevice\n127.0.0.1:55556\tdevice\n");

    // 2. Long Text: formatted with 22-char aligned serial and properties
    let mut rx_long = registry.subscribe(TrackerFormat::LongText);
    let long_payload = rx_long.recv().await.unwrap();
    let long_str = String::from_utf8(long_payload[4..].to_vec()).unwrap();
    let expected_long = "\
usb-serial             device product:pixel model:Pixel_7 device:panther transport_id:2\n\
127.0.0.1:55556        device product:mock_device model:mock_model device:mock_device transport_id:1\n";
    assert_eq!(long_str, expected_long);

    // 3. Text Protobuf
    let mut rx_proto_txt = registry.subscribe(TrackerFormat::ProtoText);
    let proto_txt_payload = rx_proto_txt.recv().await.unwrap();
    let proto_txt_str = String::from_utf8(proto_txt_payload[4..].to_vec()).unwrap();
    let expected_proto_txt = "\
device {
  serial: \"usb-serial\"
  state: DEVICE
  product: \"pixel\"
  model: \"Pixel_7\"
  device: \"panther\"
  connection_type: USB
  transport_id: 2
}
device {
  serial: \"127.0.0.1:55556\"
  state: DEVICE
  product: \"mock_device\"
  model: \"mock_model\"
  device: \"mock_device\"
  connection_type: SOCKET
  transport_id: 1
}
";
    assert_eq!(proto_txt_str, expected_proto_txt);

    // 4. Binary Protobuf
    let mut rx_proto_bin = registry.subscribe(TrackerFormat::ProtoBinary);
    let proto_bin_payload = rx_proto_bin.recv().await.unwrap();
    let length = u16::from_str_radix(std::str::from_utf8(&proto_bin_payload[..4]).unwrap(), 16)
        .unwrap() as usize;
    assert_eq!(proto_bin_payload.len(), 4 + length);

    // Verify binary proto bytes format
    let bin_slice = &proto_bin_payload[4..];
    // Must begin with tag 1 (device), wire type 2 (0x0a)
    assert_eq!(bin_slice[0], 0x0a);
}

#[tokio::test]
async fn test_tracker_multiple_subscribers_concurrent_events() {
    let registry = Arc::new(DeviceRegistry::new());

    let mut sub1 = registry.subscribe(TrackerFormat::ShortText);
    let mut sub2 = registry.subscribe(TrackerFormat::LongText);

    assert_eq!(sub1.recv().await.unwrap(), b"0000");
    assert_eq!(sub2.recv().await.unwrap(), b"0000");

    let reg = Arc::clone(&registry);
    let handle = tokio::spawn(async move {
        for i in 0..5 {
            let serial = format!("dev-{i}");
            let id = reg.register(&serial, TransportType::Usb).unwrap();
            reg.update_state(id, ConnectionState::Device);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });

    let mut sub1_received = 0;
    while sub1_received < 10 {
        // 5 registers + 5 state changes = 10 updates
        let _ = sub1.recv().await.unwrap();
        sub1_received += 1;
    }

    let mut sub2_received = 0;
    while sub2_received < 10 {
        let _ = sub2.recv().await.unwrap();
        sub2_received += 1;
    }

    handle.await.unwrap();
}

#[tokio::test]
async fn test_tracker_dropped_subscriber_does_not_block_others() {
    let registry = Arc::new(DeviceRegistry::new());

    let sub_dropped = registry.subscribe(TrackerFormat::ShortText);
    let mut sub_active = registry.subscribe(TrackerFormat::ShortText);

    assert_eq!(sub_active.recv().await.unwrap(), b"0000");

    // Drop first subscriber
    drop(sub_dropped);

    // Broadcast new device
    registry.register("dev-1", TransportType::Local).unwrap();

    let notification = sub_active.recv().await.unwrap();
    // "dev-1\toffline\n" is 14 bytes (0x000e)
    assert_eq!(notification, b"000edev-1\toffline\n");
}
