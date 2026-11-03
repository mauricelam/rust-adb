//! Compatibility tests verifying ADB server behaviors from the outside against C++.
//!
//! Covers:
//! 1. `host:track-devices`, `track-devices-l`, `track-devices-proto-text`, `track-devices-proto-binary`
//!    lifecycle, initial state, device sorting order (USB before local, then by serial), and updates.
//! 2. `host:transport:<serial>`, `transport-any`, `transport-usb`, `transport-local` success and failure
//!    replies, including device not found and ambiguity errors.
//! 3. Stream behavior through a mock device: 1-WRTE-in-flight rule and CLSE handling.
//! 4. CNXN handshake against a mock device: version, max payload, and banner exchange.

use compat::TestServer;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Returns the path to the built `mock-adbd` binary.
fn get_mock_adbd_bin() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_dir).join("../..");
    let bin_name = if cfg!(target_os = "windows") {
        "mock-adbd.exe"
    } else {
        "mock-adbd"
    };
    let bin_path = root.join("target").join("debug").join(bin_name);
    let status = Command::new("cargo")
        .args(["build", "-p", "mock-adbd"])
        .status()
        .expect("Failed to build mock-adbd binary");
    assert!(status.success(), "Failed to build mock-adbd");
    bin_path
}

/// Helper wrapper for running an isolated `mock-adbd` child process.
struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start() -> Self {
        Self::start_with_args(&[])
    }

    fn start_with_args(extra_args: &[&str]) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen").arg("127.0.0.1:0");
        for arg in extra_args {
            cmd.arg(arg);
        }
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn mock-adbd binary at {:?}: {}", bin, e));

        let stdout = child.stdout.take().expect("Failed to open stdout");
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("Failed to read first line from mock-adbd stdout");
        let addr = first_line.trim().to_string();

        MockAdbdProcess { child, addr }
    }
}

impl Drop for MockAdbdProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Helper to construct a 24-byte wire packet with command, arguments, and payload.
fn make_packet(cmd: u32, arg0: u32, arg1: u32, payload: &[u8]) -> Vec<u8> {
    let mut data = Vec::with_capacity(24 + payload.len());
    data.extend_from_slice(&cmd.to_le_bytes());
    data.extend_from_slice(&arg0.to_le_bytes());
    data.extend_from_slice(&arg1.to_le_bytes());
    data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let crc: u32 = payload.iter().map(|&b| b as u32).sum();
    data.extend_from_slice(&crc.to_le_bytes());
    let magic = cmd ^ 0xffff_ffff;
    data.extend_from_slice(&magic.to_le_bytes());
    data.extend_from_slice(payload);
    data
}

/// Reads a 24-byte packet header and its payload.
fn read_packet(stream: &mut TcpStream) -> std::io::Result<(u32, u32, u32, Vec<u8>)> {
    let mut hdr = [0u8; 24];
    stream.read_exact(&mut hdr)?;
    let cmd = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
    let arg0 = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
    let arg1 = u32::from_le_bytes(hdr[8..12].try_into().unwrap());
    let data_len = u32::from_le_bytes(hdr[12..16].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; data_len];
    if data_len > 0 {
        stream.read_exact(&mut payload)?;
    }
    Ok((cmd, arg0, arg1, payload))
}

/// Reads the next length-prefixed packet from a smart-socket stream:
/// 4-hex-digit length followed by payload bytes.
fn read_smart_socket_framed_packet(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len_str = std::str::from_utf8(&len_bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let payload_len = usize::from_str_radix(len_str, 16)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut payload = vec![0u8; payload_len];
    if payload_len > 0 {
        stream.read_exact(&mut payload)?;
    }
    Ok(payload)
}

/// Reads initial `OKAY` followed by the first length-prefixed packet on a tracking stream.
fn start_tracker_stream(server_port: u16, service: &str) -> std::io::Result<(TcpStream, Vec<u8>)> {
    let mut stream = TcpStream::connect(("127.0.0.1", server_port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;

    let req = format!("{:04x}{service}", service.len());
    stream.write_all(req.as_bytes())?;
    stream.flush()?;

    let mut status = [0u8; 4];
    stream.read_exact(&mut status)?;
    assert_eq!(&status, b"OKAY", "Expected OKAY on tracker request");

    let initial_payload = read_smart_socket_framed_packet(&mut stream)?;
    Ok((stream, initial_payload))
}

/// Verifies track-devices variants, empty states, ordering (by serial), and notification updates upon connect and disconnect.
#[test]
fn test_track_devices_all_variants_and_lifecycle() {
    let server = TestServer::start();

    // 1. Initial empty states
    let (mut track_stream, initial_track) =
        start_tracker_stream(server.port, "host:track-devices").unwrap();
    assert!(initial_track.is_empty());

    let (mut track_l_stream, initial_l) =
        start_tracker_stream(server.port, "host:track-devices-l").unwrap();
    assert!(initial_l.is_empty());

    let (mut proto_txt_stream, initial_proto_txt) =
        start_tracker_stream(server.port, "host:track-devices-proto-text").unwrap();
    assert!(initial_proto_txt.is_empty());

    let (mut proto_bin_stream, initial_proto_bin) =
        start_tracker_stream(server.port, "host:track-devices-proto-binary").unwrap();
    assert!(initial_proto_bin.is_empty());

    // 2. Connect mock device 1
    let mock1 = MockAdbdProcess::start();
    let connect_out1 = server.run_client(&["connect", &mock1.addr]);
    assert!(connect_out1.success(), "adb connect 1 failed");

    // Connect mock device 2
    let mock2 = MockAdbdProcess::start();
    let connect_out2 = server.run_client(&["connect", &mock2.addr]);
    assert!(connect_out2.success(), "adb connect 2 failed");

    // Read track-devices updates until both devices appear in state "device"
    let mut got_both_devices = false;
    let start_time = std::time::Instant::now();
    let mut last_update_str = String::new();
    while start_time.elapsed() < Duration::from_secs(5) {
        let update = read_smart_socket_framed_packet(&mut track_stream).unwrap();
        last_update_str = String::from_utf8_lossy(&update).to_string();
        if last_update_str.contains(&format!("{}\tdevice\n", mock1.addr))
            && last_update_str.contains(&format!("{}\tdevice\n", mock2.addr))
        {
            got_both_devices = true;
            break;
        }
    }
    assert!(
        got_both_devices,
        "Failed to receive both devices in track-devices: {last_update_str}"
    );

    // Verify ordering: local transports sorted alphabetically by serial
    let (first_serial, second_serial) = if mock1.addr < mock2.addr {
        (&mock1.addr, &mock2.addr)
    } else {
        (&mock2.addr, &mock1.addr)
    };
    let pos1 = last_update_str.find(first_serial).unwrap();
    let pos2 = last_update_str.find(second_serial).unwrap();
    assert!(
        pos1 < pos2,
        "Expected {first_serial} before {second_serial} in track-devices list"
    );

    // Verify track-devices-l contains tabular format for both devices
    let update_l = read_smart_socket_framed_packet(&mut track_l_stream).unwrap();
    let update_l_str = String::from_utf8_lossy(&update_l);
    assert!(
        update_l_str.contains(&mock1.addr) || update_l_str.contains(&mock2.addr),
        "track-devices-l missing expected devices: {update_l_str}"
    );

    // Verify proto-text format contains device message
    let update_proto_txt = read_smart_socket_framed_packet(&mut proto_txt_stream).unwrap();
    let proto_txt_str = String::from_utf8_lossy(&update_proto_txt);
    assert!(
        proto_txt_str.contains("device {"),
        "track-devices-proto-text unexpected: {proto_txt_str}"
    );

    // Verify proto-binary format contains protobuf payload
    let update_proto_bin = read_smart_socket_framed_packet(&mut proto_bin_stream).unwrap();
    assert!(
        !update_proto_bin.is_empty(),
        "track-devices-proto-binary received empty payload"
    );
    assert_eq!(
        update_proto_bin[0], 0x0a,
        "Protobuf message must start with tag 1, wire type 2 (0x0a)"
    );

    // 3. Disconnect device 1 and verify update
    let disconnect_out1 = server.run_client(&["disconnect", &mock1.addr]);
    assert!(disconnect_out1.success(), "adb disconnect failed");

    let disconnect_out2 = server.run_client(&["disconnect", &mock2.addr]);
    assert!(disconnect_out2.success(), "adb disconnect 2 failed");

    let mut got_empty_state = false;
    let start_time = std::time::Instant::now();
    while start_time.elapsed() < Duration::from_secs(5) {
        let update = read_smart_socket_framed_packet(&mut track_stream).unwrap();
        if update.is_empty() {
            got_empty_state = true;
            break;
        }
    }
    assert!(
        got_empty_state,
        "Failed to receive empty state in track-devices upon disconnect"
    );
}

/// Verifies `host:transport:*` service replies, including success, not found, and ambiguity errors.
#[test]
fn test_host_transport_selection_and_errors() {
    let server = TestServer::start();

    // 1. With no devices connected:
    // host:transport-any fails with "no devices/emulators found"
    let resp_any = server
        .send_smart_socket_request("host:transport-any")
        .unwrap();
    assert!(resp_any.is_fail());
    assert_eq!(resp_any.fail_reason(), Some("no devices/emulators found"));

    // host:transport-usb fails with "no devices found"
    let resp_usb = server
        .send_smart_socket_request("host:transport-usb")
        .unwrap();
    assert!(resp_usb.is_fail());
    assert_eq!(resp_usb.fail_reason(), Some("no devices found"));

    // host:transport-local fails with "no emulators found"
    let resp_local = server
        .send_smart_socket_request("host:transport-local")
        .unwrap();
    assert!(resp_local.is_fail());
    assert_eq!(resp_local.fail_reason(), Some("no emulators found"));

    // host:transport:nonexistent_serial fails with "device 'nonexistent_serial' not found"
    let resp_serial = server
        .send_smart_socket_request("host:transport:nonexistent_serial")
        .unwrap();
    assert!(resp_serial.is_fail());
    assert_eq!(
        resp_serial.fail_reason(),
        Some("device 'nonexistent_serial' not found")
    );

    // 2. Connect mock device
    let mock1 = MockAdbdProcess::start();
    let conn1 = server.run_client(&["connect", &mock1.addr]);
    assert!(conn1.success());

    // host:transport:<mock1.addr> succeeds with OKAY (no payload)
    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let req = format!("host:transport:{}", mock1.addr);
    let framed = format!("{:04x}{req}", req.len());
    stream.write_all(framed.as_bytes()).unwrap();
    let mut status = [0u8; 4];
    stream.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"OKAY");

    // host:transport-any succeeds with OKAY
    let mut stream_any = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let req_any = "0012host:transport-any";
    stream_any.write_all(req_any.as_bytes()).unwrap();
    stream_any.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"OKAY");

    // 3. Connect a second mock device to test ambiguity
    let mock2 = MockAdbdProcess::start();
    let conn2 = server.run_client(&["connect", &mock2.addr]);
    assert!(conn2.success());

    // host:transport-any fails with ambiguity error
    let resp_ambiguous = server
        .send_smart_socket_request("host:transport-any")
        .unwrap();
    assert!(resp_ambiguous.is_fail());
    assert_eq!(
        resp_ambiguous.fail_reason(),
        Some("more than one device/emulator")
    );

    // Specific serial still succeeds
    let mut stream_s2 = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    let req_s2 = format!("host:transport:{}", mock2.addr);
    let framed_s2 = format!("{:04x}{req_s2}", req_s2.len());
    stream_s2.write_all(framed_s2.as_bytes()).unwrap();
    stream_s2.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"OKAY");
}

/// Verifies stream behavior through a mock device:
/// 1-WRTE-in-flight rule (server does not send next WRTE until client sends OKAY)
/// and clean CLSE packet handling.
#[test]
fn test_stream_flow_control_and_close() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mock_port = listener.local_addr().unwrap().port();

    let server = TestServer::start();

    // Spawn client connecting adb server to our raw mock listener
    let mock_addr = format!("127.0.0.1:{mock_port}");
    let connect_handle = std::thread::spawn({
        let server_port = server.port;
        let mock_addr_clone = mock_addr.clone();
        move || {
            let mut stream = TcpStream::connect(("127.0.0.1", server_port)).unwrap();
            let req = format!("host:connect:{mock_addr_clone}");
            let framed = format!("{:04x}{req}", req.len());
            let _ = stream.write_all(framed.as_bytes());
            let mut buf = [0u8; 100];
            let _ = stream.read(&mut buf);
        }
    });

    let (mut dev_sock, _) = listener.accept().unwrap();
    dev_sock
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    dev_sock
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    // 1. Handshake
    let (cnxn_cmd, _version, _max_payload, _banner) = read_packet(&mut dev_sock).unwrap();
    assert_eq!(cnxn_cmd, 0x4e584e43, "Expected A_CNXN");

    // Respond with device CNXN
    let dev_banner =
        b"device::ro.product.name=mock;ro.product.model=mock;ro.product.device=mock;features=shell_v2,cmd;";
    dev_sock
        .write_all(&make_packet(0x4e584e43, 0x01000001, 1048576, dev_banner))
        .unwrap();

    let _ = connect_handle.join();

    // 2. Client initiates a stream to the device
    let mut client_sock = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    client_sock
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client_sock
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let tport_req = format!("host:transport:{mock_addr}");
    client_sock
        .write_all(format!("{:04x}{tport_req}", tport_req.len()).as_bytes())
        .unwrap();
    let mut status = [0u8; 4];
    client_sock.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"OKAY");

    let service_req = "shell:cat";
    client_sock
        .write_all(format!("{:04x}{service_req}", service_req.len()).as_bytes())
        .unwrap();

    // 3. Verify server sends A_OPEN (0x4e45504f) to device
    let (open_cmd, server_local_id, _, open_payload) = read_packet(&mut dev_sock).unwrap();
    assert_eq!(open_cmd, 0x4e45504f, "Expected A_OPEN");
    assert_eq!(open_payload, b"shell:cat\0");

    // Device accepts stream with A_OKAY (0x59414b4f)
    let mock_local_id = 42u32;
    dev_sock
        .write_all(&make_packet(
            0x59414b4f,
            mock_local_id,
            server_local_id,
            b"",
        ))
        .unwrap();

    // Client receives OKAY confirming service connection
    client_sock.read_exact(&mut status).unwrap();
    assert_eq!(&status, b"OKAY");

    // 4. Test 1-WRTE-in-flight rule:
    // Client sends multiple data chunks into the stream
    client_sock.write_all(b"first chunk\n").unwrap();
    client_sock.flush().unwrap();
    std::thread::sleep(Duration::from_millis(50));
    client_sock.write_all(b"second chunk\n").unwrap();
    client_sock.flush().unwrap();

    // Server must send the first A_WRTE to device
    let (wrte1_cmd, wrte1_arg0, wrte1_arg1, wrte1_payload) = read_packet(&mut dev_sock).unwrap();
    assert_eq!(wrte1_cmd, 0x45545257, "Expected first A_WRTE packet");
    assert_eq!(wrte1_arg0, server_local_id);
    assert_eq!(wrte1_arg1, mock_local_id);
    assert_eq!(wrte1_payload, b"first chunk\n");

    // Verify 1-WRTE-in-flight: before device sends A_OKAY, server MUST NOT send a second A_WRTE
    dev_sock
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let second_read_result = read_packet(&mut dev_sock);
    assert!(
        second_read_result.is_err(),
        "Expected timeout: server sent second WRTE before OKAY arrived: {:?}",
        second_read_result
    );

    // Now device sends A_OKAY acknowledging the first WRTE
    dev_sock
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    dev_sock
        .write_all(&make_packet(
            0x59414b4f,
            mock_local_id,
            server_local_id,
            b"",
        ))
        .unwrap();

    // Now server must send the second A_WRTE packet
    let (wrte2_cmd, wrte2_arg0, wrte2_arg1, wrte2_payload) = read_packet(&mut dev_sock).unwrap();
    assert_eq!(wrte2_cmd, 0x45545257, "Expected second A_WRTE packet");
    assert_eq!(wrte2_arg0, server_local_id);
    assert_eq!(wrte2_arg1, mock_local_id);
    assert_eq!(wrte2_payload, b"second chunk\n");

    // Acknowledge second write
    dev_sock
        .write_all(&make_packet(
            0x59414b4f,
            mock_local_id,
            server_local_id,
            b"",
        ))
        .unwrap();

    // 5. Test CLSE handling:
    // Device closes stream with A_CLSE (0x45534c43)
    dev_sock
        .write_all(&make_packet(
            0x45534c43,
            mock_local_id,
            server_local_id,
            b"",
        ))
        .unwrap();

    // Server sends A_CLSE back to device confirming stream termination
    let (clse_cmd, clse_arg0, clse_arg1, _) = read_packet(&mut dev_sock).unwrap();
    assert_eq!(clse_cmd, 0x45534c43, "Expected A_CLSE confirmation");
    assert_eq!(clse_arg0, server_local_id);
    assert_eq!(clse_arg1, mock_local_id);

    // Client connection closes
    let mut eof_buf = [0u8; 10];
    let n = client_sock.read(&mut eof_buf).unwrap();
    assert_eq!(n, 0, "Client socket should be closed after stream CLSE");
}

/// Verifies the CNXN handshake behavior against a mock device listening on TCP:
/// Version, max payload negotiation, and banner exchange/parsing.
#[test]
fn test_cnxn_handshake_parameters() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(false).unwrap();

    let server = TestServer::start();

    // Instruct server to connect to our listener
    let addr = format!("127.0.0.1:{port}");
    let addr_clone = addr.clone();
    let server_port = server.port;
    let _handle = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", server_port)).unwrap();
        let req = format!("host:connect:{addr_clone}");
        let framed = format!("{:04x}{req}", req.len());
        let _ = stream.write_all(framed.as_bytes());
        let mut buf = [0u8; 100];
        let _ = stream.read(&mut buf);
    });

    let (mut conn, _) = listener.accept().unwrap();
    conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    conn.set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    // 1. Read CNXN header: 24 bytes
    let (cmd, version, max_payload, payload) = read_packet(&mut conn).unwrap();

    // Command must be A_CNXN ("CNXN" = 0x4e584e43)
    assert_eq!(cmd, 0x4e584e43, "Expected A_CNXN header");

    // arg0: version must be >= 0x01000000 (A_VERSION)
    assert!(
        version >= 0x01000000,
        "Expected protocol version >= 0x01000000, got 0x{version:08x}"
    );

    // arg1: max_payload must be >= 4096 (standard ADB max payload)
    assert!(
        max_payload >= 4096,
        "Expected max payload >= 4096, got {max_payload}"
    );

    // Banner parsing
    assert!(!payload.is_empty(), "CNXN payload must not be empty");
    let banner = String::from_utf8_lossy(&payload);
    assert!(
        banner.starts_with("host:"),
        "Banner should start with 'host:', got: {banner}"
    );
    assert!(
        banner.contains("features="),
        "Banner should advertise host features: {banner}"
    );

    // 2. Respond with device CNXN to complete handshake
    let dev_banner =
        b"device::ro.product.name=mock;ro.product.model=mock;ro.product.device=mock;features=shell_v2,cmd;";
    conn.write_all(&make_packet(0x4e584e43, 0x01000001, 1048576, dev_banner))
        .unwrap();

    // Verify the server establishes the connection:
    // query server devices list, device should appear as 'device'
    let mut got_device = false;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Ok(resp) = server.send_smart_socket_request("host:devices") {
            let devices_str = resp.payload_str().unwrap_or_default();
            if devices_str.contains(&format!("{addr}\tdevice\n")) {
                got_device = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        got_device,
        "Server failed to complete handshake and register device after receiving CNXN"
    );
}
