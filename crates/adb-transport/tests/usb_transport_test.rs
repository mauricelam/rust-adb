//! Unit tests for UsbTransport packet framing over bulk transfers.

use adb_proto::{Command, Header, MAX_PAYLOAD};
use adb_transport::{
    calculate_checksum, EndpointDirection, MockUsbDeviceHandle, Packet, Transport, TransportError,
    UsbError, UsbTransport,
};
use std::sync::Arc;

#[tokio::test]
async fn test_write_packet_separate_header_and_payload_with_zlp() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(
        mock.clone(),
        0x01, // bulk out
        512,  // bulk out max packet size
        0x81, // bulk in
        512,  // bulk in max packet size
    );

    // Payload length 512 is an exact multiple of bulk_out_max_packet_size (512)
    let payload = vec![0xAB; 512];
    let packet = Packet::new(Command::Wrte, 1, 2, payload.clone(), false);

    transport
        .write_packet(&packet)
        .await
        .expect("write_packet succeeds");

    let transfers = mock.get_transfers().await;
    // Expect 3 transfers:
    // 1. Header (24 bytes)
    // 2. Payload (512 bytes)
    // 3. ZLP (0 bytes)
    assert_eq!(
        transfers.len(),
        3,
        "expected header, payload, and zlp transfers"
    );

    assert_eq!(transfers[0].endpoint, 0x01);
    assert_eq!(transfers[0].direction, EndpointDirection::Out);
    assert_eq!(transfers[0].data, packet.header.encode());

    assert_eq!(transfers[1].endpoint, 0x01);
    assert_eq!(transfers[1].direction, EndpointDirection::Out);
    assert_eq!(transfers[1].data, payload);

    assert_eq!(transfers[2].endpoint, 0x01);
    assert_eq!(transfers[2].direction, EndpointDirection::Out);
    assert_eq!(transfers[2].data.len(), 0, "ZLP must be 0-byte transfer");
}

#[tokio::test]
async fn test_write_packet_zero_length_payload_no_zlp() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let packet = Packet::new(Command::Okay, 1, 2, Vec::new(), false);

    transport
        .write_packet(&packet)
        .await
        .expect("write_packet succeeds");

    let transfers = mock.get_transfers().await;
    // Expect only 1 transfer: header. No payload transfer, no ZLP.
    assert_eq!(transfers.len(), 1);
    assert_eq!(transfers[0].endpoint, 0x01);
    assert_eq!(transfers[0].direction, EndpointDirection::Out);
    assert_eq!(transfers[0].data, packet.header.encode());
}

#[tokio::test]
async fn test_write_packet_non_multiple_payload_no_zlp() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let payload = vec![0x42; 511];
    let packet = Packet::new(Command::Wrte, 1, 2, payload.clone(), false);

    transport
        .write_packet(&packet)
        .await
        .expect("write_packet succeeds");

    let transfers = mock.get_transfers().await;
    // Expect 2 transfers: header, then payload. No ZLP because 511 % 512 != 0.
    assert_eq!(transfers.len(), 2);
    assert_eq!(transfers[0].data, packet.header.encode());
    assert_eq!(transfers[1].data, payload);
}

#[tokio::test]
async fn test_read_packet_separate_header_and_payload() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let payload = b"hello usb device".to_vec();
    let packet = Packet::new(Command::Open, 10, 0, payload.clone(), false);
    let mut stream_bytes = packet.header.encode().to_vec();
    stream_bytes.extend_from_slice(&payload);

    *mock.read_data.lock().await = stream_bytes;

    let read_pkt = transport.read_packet().await.expect("read_packet succeeds");
    assert_eq!(read_pkt, packet);

    let transfers = mock.get_transfers().await;
    assert_eq!(
        transfers.len(),
        2,
        "header and payload read as separate transfers"
    );
    assert_eq!(transfers[0].endpoint, 0x81);
    assert_eq!(transfers[0].direction, EndpointDirection::In);
    assert_eq!(transfers[0].data, packet.header.encode());
    assert_eq!(transfers[1].endpoint, 0x81);
    assert_eq!(transfers[1].direction, EndpointDirection::In);
    assert_eq!(transfers[1].data, payload);
}

#[tokio::test]
async fn test_read_packet_payload_fragmented_across_multiple_transfers() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let payload = vec![0x77; 1000];
    let packet = Packet::new(Command::Wrte, 10, 20, payload.clone(), false);
    let mut stream_bytes = packet.header.encode().to_vec();
    stream_bytes.extend_from_slice(&payload);

    *mock.read_data.lock().await = stream_bytes;
    // Script read chunk sizes:
    // First read: 24 bytes header
    // Second read: 400 bytes payload
    // Third read: 400 bytes payload
    // Fourth read: 200 bytes payload
    *mock.read_chunk_sizes.lock().await = vec![24, 400, 400, 200];

    let read_pkt = transport.read_packet().await.expect("read_packet succeeds");
    assert_eq!(read_pkt, packet);

    let transfers = mock.get_transfers().await;
    assert_eq!(transfers.len(), 4);
    assert_eq!(transfers[0].data.len(), 24);
    assert_eq!(transfers[1].data.len(), 400);
    assert_eq!(transfers[2].data.len(), 400);
    assert_eq!(transfers[3].data.len(), 200);
}

#[tokio::test]
async fn test_read_packet_zero_length_payload() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let packet = Packet::new(Command::Clse, 5, 6, Vec::new(), false);
    *mock.read_data.lock().await = packet.header.encode().to_vec();

    let read_pkt = transport.read_packet().await.expect("read_packet succeeds");
    assert_eq!(read_pkt, packet);

    let transfers = mock.get_transfers().await;
    assert_eq!(
        transfers.len(),
        1,
        "only header read for zero-length payload"
    );
    assert_eq!(transfers[0].data, packet.header.encode());
}

#[tokio::test]
async fn test_read_packet_payload_too_large() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    // Header claiming payload larger than MAX_PAYLOAD
    let header = Header::new(Command::Wrte, 1, 2, (MAX_PAYLOAD + 1) as u32, 0);
    *mock.read_data.lock().await = header.encode().to_vec();

    let err = transport.read_packet().await.unwrap_err();
    match err {
        TransportError::PayloadTooLarge { max, actual } => {
            assert_eq!(max, MAX_PAYLOAD);
            assert_eq!(actual, MAX_PAYLOAD + 1);
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn test_read_packet_checksum_mismatch() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let payload = b"original data".to_vec();
    let header = Header::new(Command::Wrte, 1, 2, payload.len() as u32, 0x12345678);
    let mut stream_bytes = header.encode().to_vec();
    stream_bytes.extend_from_slice(&payload);
    *mock.read_data.lock().await = stream_bytes;

    let err = transport.read_packet().await.unwrap_err();
    match err {
        TransportError::ChecksumMismatch { expected, actual } => {
            assert_eq!(expected, 0x12345678);
            assert_eq!(actual, calculate_checksum(&payload));
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn test_backend_disconnect_fails_and_future_calls_fail_immediately() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    *mock.should_fail_read.lock().await = Some(UsbError::Disconnected);

    let err1 = transport.read_packet().await.unwrap_err();
    assert!(matches!(err1, TransportError::Io(_)));

    // Ensure subsequent calls fail immediately rather than hang or retry
    let err2 = transport.read_packet().await.unwrap_err();
    assert!(matches!(err2, TransportError::Io(_)));

    let packet = Packet::new(Command::Okay, 1, 2, Vec::new(), false);
    let err3 = transport.write_packet(&packet).await.unwrap_err();
    assert!(matches!(err3, TransportError::Io(_)));
}

#[tokio::test]
async fn test_usb_transport_with_boxed_handle() {
    let mock = Box::new(MockUsbDeviceHandle::default()) as Box<dyn adb_transport::UsbDeviceHandle>;
    let mut transport = UsbTransport::new(mock, 0x01, 512, 0x81, 512);

    let packet = Packet::new(Command::Okay, 1, 2, Vec::new(), false);
    transport
        .write_packet(&packet)
        .await
        .expect("write_packet succeeds on Box<dyn UsbDeviceHandle>");
}
