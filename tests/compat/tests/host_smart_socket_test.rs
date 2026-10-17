use compat::{SmartSocketStatus, TestServer};

/// Covers SERVICES.TXT section:
/// host:version
///     Ask the ADB server for its internal version number.
#[test]
fn test_host_version() {
    let server = TestServer::start();

    let resp = server
        .send_smart_socket_request("host:version")
        .expect("Failed to send smart socket request host:version");

    assert_eq!(resp.status, SmartSocketStatus::Okay);
    assert!(resp.is_okay());
    assert_eq!(resp.status_bytes, *b"OKAY");
    assert_eq!(resp.len_prefix.as_deref(), Some("0004"));
    assert_eq!(resp.payload.len(), 4);

    // Payload should be a 4-digit hex string representing version number (e.g., "0029" or "0028")
    let version_str = resp
        .payload_str()
        .expect("Version string should be valid UTF-8");
    assert!(
        version_str.chars().all(|c| c.is_ascii_hexdigit()),
        "Version payload expected to be hex digits, got: {version_str}"
    );

    // Verify raw bytes consistency: "OKAY" + 4-hex length + 4-hex version
    let raw = server
        .send_smart_socket_request_raw("host:version")
        .expect("Failed to send raw request");
    assert_eq!(raw.len(), 12);
    assert_eq!(&raw[0..4], b"OKAY");
    assert_eq!(&raw[4..8], b"0004");
    assert_eq!(&raw[8..12], resp.payload.as_slice());
}

/// Covers SERVICES.TXT section:
/// host:devices
///     Ask to return the list of available Android devices and their state.
#[test]
fn test_host_devices_no_device() {
    let server = TestServer::start();

    let resp = server
        .send_smart_socket_request("host:devices")
        .expect("Failed to send smart socket request host:devices");

    assert_eq!(resp.status, SmartSocketStatus::Okay);
    assert!(resp.is_okay());
    assert_eq!(resp.status_bytes, *b"OKAY");
    assert_eq!(resp.len_prefix.as_deref(), Some("0000"));
    assert!(resp.payload.is_empty());

    // Assert exact bytes returned when no device is connected: b"OKAY0000"
    let raw = server
        .send_smart_socket_request_raw("host:devices")
        .expect("Failed to send raw request");
    assert_eq!(raw, b"OKAY0000");
}

/// Covers SERVICES.TXT section:
/// host:devices-l
///     Ask to return the list of available Android devices and their state.
///     devices-l includes the device paths in the state.
#[test]
fn test_host_devices_l_no_device() {
    let server = TestServer::start();

    let resp = server
        .send_smart_socket_request("host:devices-l")
        .expect("Failed to send smart socket request host:devices-l");

    assert_eq!(resp.status, SmartSocketStatus::Okay);
    assert!(resp.is_okay());
    assert_eq!(resp.status_bytes, *b"OKAY");
    assert_eq!(resp.len_prefix.as_deref(), Some("0000"));
    assert!(resp.payload.is_empty());

    // Assert exact bytes returned when no device is connected: b"OKAY0000"
    let raw = server
        .send_smart_socket_request_raw("host:devices-l")
        .expect("Failed to send raw request");
    assert_eq!(raw, b"OKAY0000");
}

/// Covers SERVICES.TXT smart-socket protocol error handling:
/// Requests for a non-existent host service must return FAIL with 4-hex length and error reason string.
#[test]
fn test_nonexistent_service() {
    let server = TestServer::start();
    let service = "host:nonexistent_service_12345";

    let resp = server
        .send_smart_socket_request(service)
        .expect("Failed to send smart socket request");

    assert_eq!(resp.status, SmartSocketStatus::Fail);
    assert!(resp.is_fail());
    assert_eq!(resp.status_bytes, *b"FAIL");

    let sub_command = service.strip_prefix("host:").unwrap_or(service);
    let expected_reason = format!("unknown host service '{sub_command}'");
    let expected_len_prefix = format!("{:04x}", expected_reason.len());

    assert_eq!(
        resp.len_prefix.as_deref(),
        Some(expected_len_prefix.as_str())
    );
    assert_eq!(resp.fail_reason(), Some(expected_reason.as_str()));

    // Assert exact raw bytes returned
    let expected_raw = format!("FAIL{expected_len_prefix}{expected_reason}");
    let raw = server
        .send_smart_socket_request_raw(service)
        .expect("Failed to send raw request");
    assert_eq!(raw, expected_raw.as_bytes());
}

/// Covers SERVICES.TXT section:
/// host:transport-any
///     Another host:transport variant. Ask to switch the connection to
///     either the device or emulator connect to/running on the host.
///     Will fail if there is more than one such device/emulator available.
#[test]
fn test_host_transport_any_no_device() {
    let server = TestServer::start();

    let resp = server
        .send_smart_socket_request("host:transport-any")
        .expect("Failed to send smart socket request host:transport-any");

    assert_eq!(resp.status, SmartSocketStatus::Fail);
    assert!(resp.is_fail());
    assert_eq!(resp.status_bytes, *b"FAIL");
    assert_eq!(resp.len_prefix.as_deref(), Some("001a"));
    assert_eq!(resp.fail_reason(), Some("no devices/emulators found"));

    // Assert exact bytes returned when no device is connected: b"FAIL001ano devices/emulators found"
    let raw = server
        .send_smart_socket_request_raw("host:transport-any")
        .expect("Failed to send raw request");
    assert_eq!(raw, b"FAIL001ano devices/emulators found");
}

/// Covers SERVICES.TXT section:
/// <host-prefix>:get-state
///     Returns the state of a given device as a string.
#[test]
fn test_host_get_state_no_device() {
    let server = TestServer::start();

    let resp = server
        .send_smart_socket_request("host:get-state")
        .expect("Failed to send smart socket request host:get-state");

    assert_eq!(resp.status, SmartSocketStatus::Fail);
    assert!(resp.is_fail());
    assert_eq!(resp.status_bytes, *b"FAIL");
    assert_eq!(resp.len_prefix.as_deref(), Some("001a"));
    assert_eq!(resp.fail_reason(), Some("no devices/emulators found"));

    // Assert exact bytes returned when no device is connected: b"FAIL001ano devices/emulators found"
    let raw = server
        .send_smart_socket_request_raw("host:get-state")
        .expect("Failed to send raw request");
    assert_eq!(raw, b"FAIL001ano devices/emulators found");
}
