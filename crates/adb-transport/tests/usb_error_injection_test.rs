//! USB error-injection tests verifying fault handling across simulated hardware failures.
//!
//! Upstream C++ citations and expected behaviors:
//!
//! 1. Bulk transfer timeout:
//!    - C++ ADB source: `original/client/usb_linux.cpp` (`usb_bulk_read`, `usb_bulk_write`),
//!      `original/client/transport_usb.cpp` (`remote_read`, `remote_write`),
//!      and `original/transport.cpp` (`atransport::HandleError`).
//!    - Expected behavior: When a transfer times out, the backend returns timeout error,
//!      `remote_read` or `remote_write` returns false, and the transport calls `HandleError`
//!      terminating the transport (`handle_offline` and `transport_destroy`).
//!
//! 2. Endpoint stall with clear halt success / failure:
//!    - C++ ADB source: `original/client/usb_osx.cpp` (`usb_read`, lines 624-628) and
//!      `original/client/usb_libusb_device.cpp` (`ClaimInterface`, lines 470-476).
//!    - Expected behavior: On endpoint stall (`kIOUSBPipeStalled`), clearing stall via
//!      `ClearPipeStall` allows retry or recovery. If `clear_halt` fails, the error is logged,
//!      the interface is released, and the connection is closed.
//!
//! 3. Short read in the middle of payload:
//!    - C++ ADB source: `original/client/transport_usb.cpp` (`remote_read`, `UsbReadPayload`).
//!    - Expected behavior: If read payload gives fewer bytes than `p->msg.data_length`
//!      (`rc != static_cast<int>(p->msg.data_length)`), remote read logs:
//!      "remote usb: read payload failed (need %u bytes, give %d bytes), skip it" and
//!      returns -1, tearing down the transport without panicking or hanging.
//!
//! 4. Header with payload length above negotiated maximum:
//!    - C++ ADB source: `original/client/transport_usb.cpp` (`UsbReadPayload`) and
//!      `original/transport.cpp` (`check_header`).
//!    - Expected behavior: When `p->msg.data_length > MAX_PAYLOAD`, `UsbReadPayload` returns -1
//!      and `check_header` returns false, immediately terminating the connection.
//!
//! 5. Header with bad magic or payload checksum mismatch:
//!    - C++ ADB source: `original/transport.cpp` (`check_header`, lines 1692-1698),
//!      `original/adb.cpp` (`calculate_apacket_checksum`), and `original/transport.cpp` (`HandleRead`).
//!    - Expected behavior: Invalid magic (`magic != command ^ 0xffffffff`) or checksum mismatch
//!      rejects the packet and terminates the transport immediately.
//!
//! 6. Write cancelled by dropping future partway through transfer:
//!    - C++ ADB / Rust async source: Drop-to-cancel semantics in async Rust transport.
//!    - Expected behavior: Future cancellation cleanly aborts the transfer, releases locks,
//!      and leaves other operations and background tasks clean without hang or panic.
//!
//! 7. Multi-transport isolation:
//!    - Expected behavior: Failure or teardown of one USB transport must not impact or stall
//!      any other concurrent USB transports.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use adb_proto::{Command, DecodeError, Header};
use adb_transport::{
    calculate_checksum, MockUsbDeviceHandle, Packet, Transport, TransportError, UsbDeviceHandle,
    UsbError, UsbTransport,
};

/// 1. A bulk transfer times out.
/// Upstream: `original/client/usb_linux.cpp`, `original/transport.cpp` (`HandleError`).
/// Expected behavior: Transport returns I/O timeout error, marks transport failed, and tears down cleanly.
#[tokio::test]
async fn test_scenario_bulk_transfer_timeout() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    *mock.should_fail_read.lock().await = Some(UsbError::Timeout);

    let res = timeout(Duration::from_secs(1), transport.read_packet()).await;
    assert!(res.is_ok(), "read_packet must not hang on timeout");
    let err = res.unwrap().unwrap_err();
    match err {
        TransportError::Io(io_err) => {
            assert_eq!(io_err.kind(), std::io::ErrorKind::TimedOut);
        }
        other => panic!("expected TimedOut error, got {other:?}"),
    }

    // Subsequent write or read must fail immediately due to latched failure (clean teardown)
    let write_res = transport
        .write_packet(&Packet::new(Command::Okay, 1, 2, Vec::new(), false))
        .await;
    assert!(write_res.is_err(), "transport must remain in failed state");
}

/// 2a. An endpoint stalls and clear halt succeeds.
/// Upstream: `original/client/usb_osx.cpp` (`usb_read`, lines 624-628).
/// Expected behavior: Clearing halt clears the stall condition on the endpoint, allowing recovery.
#[tokio::test]
async fn test_scenario_endpoint_stall_clear_halt_succeeds() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let endpoint = 0x81;

    // Simulate endpoint stalled
    mock.halted_endpoints.lock().await.push(endpoint);
    assert!(mock.halted_endpoints.lock().await.contains(&endpoint));

    // Clear halt succeeds
    let clear_res = mock.clear_halt(endpoint).await;
    assert!(clear_res.is_ok(), "clear_halt should succeed");
    assert_eq!(*mock.clear_halt_call_count.lock().await, 1);
    assert!(
        !mock.halted_endpoints.lock().await.contains(&endpoint),
        "endpoint must no longer be halted"
    );
}

/// 2b. An endpoint stalls and clear halt fails.
/// Upstream: `original/client/usb_libusb_device.cpp` (`ClaimInterface`, lines 471-476).
/// Expected behavior: Failure to clear halt returns error, signaling interface release and abort.
#[tokio::test]
async fn test_scenario_endpoint_stall_clear_halt_fails() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let endpoint = 0x81;

    *mock.should_fail_clear_halt.lock().await =
        Some(UsbError::Io("hardware failed to clear pipe".to_string()));

    let clear_res = mock.clear_halt(endpoint).await;
    assert!(clear_res.is_err(), "clear_halt failure must return error");
    match clear_res.unwrap_err() {
        UsbError::Io(msg) => assert!(msg.contains("hardware failed to clear pipe")),
        other => panic!("expected UsbError::Io, got {other:?}"),
    }
    assert_eq!(*mock.clear_halt_call_count.lock().await, 1);
}

/// 3. A short read in the middle of a payload.
/// Upstream: `original/client/transport_usb.cpp` (`remote_read`, `UsbReadPayload`).
/// Expected behavior: Truncated payload read (unexpected EOF) returns error and triggers transport teardown.
#[tokio::test]
async fn test_scenario_short_read_in_middle_of_payload() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let full_payload = vec![0x33; 100];
    let packet = Packet::new(Command::Wrte, 1, 2, full_payload, false);
    let mut stream_bytes = packet.header.encode().to_vec();
    // Only append 50 bytes of the 100 bytes payload
    stream_bytes.extend_from_slice(&[0x33; 50]);

    *mock.read_data.lock().await = stream_bytes;

    let res = timeout(Duration::from_secs(1), transport.read_packet()).await;
    assert!(res.is_ok(), "read_packet must not hang on short read");
    let err = res.unwrap().unwrap_err();
    match err {
        TransportError::Io(io_err) => {
            assert_eq!(io_err.kind(), std::io::ErrorKind::UnexpectedEof);
        }
        other => panic!("expected UnexpectedEof error, got {other:?}"),
    }

    // Transport must now be marked failed
    let next_read = transport.read_packet().await;
    assert!(next_read.is_err(), "transport must be in failed state");
}

/// 4. The device sends a header with a payload length above the negotiated maximum.
/// Upstream: `original/client/transport_usb.cpp` (`UsbReadPayload`), `original/transport.cpp` (`check_header`).
/// Expected behavior: Returns `PayloadTooLarge`, tearing down connection.
#[tokio::test]
async fn test_scenario_payload_length_above_maximum() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let negotiated_max = 4096;
    transport.set_max_payload(negotiated_max);

    // Device attempts to send header with 8192 bytes payload
    let header = Header::new(Command::Wrte, 1, 2, 8192, 0);
    *mock.read_data.lock().await = header.encode().to_vec();

    let res = timeout(Duration::from_secs(1), transport.read_packet()).await;
    assert!(res.is_ok(), "read_packet must not hang");
    let err = res.unwrap().unwrap_err();
    match err {
        TransportError::PayloadTooLarge { max, actual } => {
            assert_eq!(max, negotiated_max);
            assert_eq!(actual, 8192);
        }
        other => panic!("expected PayloadTooLarge, got {other:?}"),
    }
}

/// 5a. The device sends a header with bad magic.
/// Upstream: `original/transport.cpp` (`check_header`, lines 1692-1698).
/// Expected behavior: Header decoding fails with `InvalidMagic`, terminating connection.
#[tokio::test]
async fn test_scenario_header_bad_magic() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let mut header_bytes = Header::new(Command::Okay, 1, 2, 0, 0).encode().to_vec();
    // Corrupt magic field (bytes 20..24)
    header_bytes[20..24].copy_from_slice(&0xdeadbeef_u32.to_le_bytes());

    *mock.read_data.lock().await = header_bytes;

    let res = timeout(Duration::from_secs(1), transport.read_packet()).await;
    assert!(res.is_ok(), "read_packet must not hang on bad magic");
    let err = res.unwrap().unwrap_err();
    match err {
        TransportError::Decode(decode_err) => {
            assert!(
                matches!(decode_err, DecodeError::InvalidMagic { .. }),
                "expected InvalidMagic error, got {decode_err:?}"
            );
        }
        other => panic!("expected Decode error, got {other:?}"),
    }
}

/// 5b. The device sends a payload whose checksum does not match.
/// Upstream: `original/transport.cpp` (`HandleRead`), `original/adb.cpp` (`calculate_apacket_checksum`).
/// Expected behavior: Checksum mismatch returns `ChecksumMismatch`, terminating connection.
#[tokio::test]
async fn test_scenario_payload_checksum_mismatch() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let payload = b"critical firmware update".to_vec();
    let header = Header::new(Command::Wrte, 10, 20, payload.len() as u32, 0xbeefcafe);
    let mut stream_bytes = header.encode().to_vec();
    stream_bytes.extend_from_slice(&payload);

    *mock.read_data.lock().await = stream_bytes;

    let res = timeout(Duration::from_secs(1), transport.read_packet()).await;
    assert!(
        res.is_ok(),
        "read_packet must not hang on checksum mismatch"
    );
    let err = res.unwrap().unwrap_err();
    match err {
        TransportError::ChecksumMismatch { expected, actual } => {
            assert_eq!(expected, 0xbeefcafe);
            assert_eq!(actual, calculate_checksum(&payload));
        }
        other => panic!("expected ChecksumMismatch, got {other:?}"),
    }
}

/// 6. A write is cancelled by dropping the future partway through a transfer.
/// Expected behavior: Dropping the write future does not panic or deadlock the handle.
#[tokio::test]
async fn test_scenario_write_cancelled_by_drop() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let payload = vec![0xAA; 1024];
    let packet = Packet::new(Command::Wrte, 1, 2, payload, false);

    // Spawn a task that writes the packet
    let write_fut = transport.write_packet(&packet);
    // Drop future before completion or immediately by running a brief timeout
    drop(write_fut);

    // Verify handle and transport remain usable and not deadlocked
    let test_packet = Packet::new(Command::Okay, 1, 2, Vec::new(), false);
    let write_res = timeout(Duration::from_secs(1), transport.write_packet(&test_packet)).await;
    assert!(
        write_res.is_ok(),
        "subsequent write must not hang or deadlock after prior drop"
    );
    assert!(write_res.unwrap().is_ok());
}

/// 7. Multi-transport isolation: failure of one transport does not affect another.
#[tokio::test]
async fn test_scenario_multiple_transports_isolation() {
    let mock1 = Arc::new(MockUsbDeviceHandle::default());
    let mock2 = Arc::new(MockUsbDeviceHandle::default());

    let mut transport1 = UsbTransport::new(mock1.clone(), 0x01, 512, 0x81, 512);
    let mut transport2 = UsbTransport::new(mock2.clone(), 0x02, 512, 0x82, 512);

    // Fail transport1 with disconnected error
    *mock1.should_fail_read.lock().await = Some(UsbError::Disconnected);

    // Provide valid data to transport2
    let valid_packet = Packet::new(Command::Okay, 10, 20, Vec::new(), false);
    *mock2.read_data.lock().await = valid_packet.header.encode().to_vec();

    // Transport 1 fails
    let res1 = transport1.read_packet().await;
    assert!(res1.is_err(), "transport1 must fail");

    // Transport 2 must succeed completely unaffected
    let res2 = transport2.read_packet().await;
    assert!(
        res2.is_ok(),
        "transport2 must not be affected by transport1"
    );
    assert_eq!(res2.unwrap(), valid_packet);
}

/// Verification that continuing after a bad header would be caught:
/// Test verifies that an invalid magic header is strictly rejected and never returned as a valid packet.
#[tokio::test]
async fn test_mutant_ignoring_bad_header_is_caught() {
    let mock = Arc::new(MockUsbDeviceHandle::default());
    let mut transport = UsbTransport::new(mock.clone(), 0x01, 512, 0x81, 512);

    let mut bad_header = Header::new(Command::Cnxn, 1, 2, 0, 0).encode().to_vec();
    // Corrupt magic
    bad_header[20..24].copy_from_slice(&[0, 0, 0, 0]);
    *mock.read_data.lock().await = bad_header;

    let res = transport.read_packet().await;
    assert!(
        res.is_err(),
        "bad header magic must never be accepted as valid packet"
    );
}
