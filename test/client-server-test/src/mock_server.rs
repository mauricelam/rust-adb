//! Mock ADB server for integration testing.
//!
//! This module implements a simplified version of the ADB host-side protocol.
//! It is used as a Man-In-The-Middle (MITM) to intercept and verify commands
//! sent by the ADB client (either the original C++ implementation or our Rust port).
//!
//! ### ADB Protocol Overview
//!
//! 1. **Handshake**: The client connects to the server (default port 5037).
//! 2. **Request**: The client sends a 4-byte hex length followed by the service name.
//!    Example: `000chost:version`
//! 3. **Response**: The server responds with `OKAY` (4 bytes) or `FAIL` (4 bytes + error message).
//! 4. **Data**: Depending on the service, the server may send additional data or
//!    switch to a raw bidirectional stream (e.g., for `shell`).
//!
//! ### Mocked Services
//!
//! - `host:version`: Returns a hardcoded version (0029).
//! - `host:features`: Returns a list of supported features (e.g., `remount`, `shell_v2`).
//! - `host:tport:*` / `host:transport:*`: Simulates selecting a transport.
//!   The server responds with `OKAY` and 8 bytes representing a dummy transport ID.
//! - `host:get-serialno`: Returns a dummy serial number.
//! - `host:get-devpath`: Returns a dummy device path.
//! - `host:list-forward`: Returns a dummy list of forwardings.
//! - `shell:*`: Simulates a shell command execution by returning "shell output".
//! - `remount`: Simulates a successful remount.
//! - `host:devices`: Returns a dummy list containing one device.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// Starts the mock ADB server on a random available port.
/// Returns the port number, a receiver for intercepted commands, and a thread handle.
pub fn start_mock_server() -> std::io::Result<(u16, Receiver<String>, thread::JoinHandle<()>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();

    let (tx, rx) = mpsc::channel();

    let jh = thread::spawn(move || {
        for stream in listener.incoming() {
            if let Ok(stream) = stream {
                let tx_clone = tx.clone();
                thread::spawn(move || {
                    let _ = handle_connection(stream, tx_clone);
                });
            } else {
                break;
            }
        }
    });

    Ok((port, rx, jh))
}

/// Handles a single client connection to the mock server.
fn handle_connection(mut stream: TcpStream, tx: Sender<String>) -> std::io::Result<()> {
    // Set a timeout to prevent tests from hanging indefinitely if the client fails to send data.
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = stream.try_clone()?;

    loop {
        // Read the 4-byte length prefix.
        let mut len_buf = [0u8; 4];
        if let Err(_) = reader.read_exact(&mut len_buf) {
            // End of stream or timeout.
            break;
        }

        let len_str = std::str::from_utf8(&len_buf).unwrap_or("0000");
        let len = u32::from_str_radix(len_str, 16).unwrap_or(0);

        // Read the actual service request.
        let mut msg_buf = vec![0u8; len as usize];
        if let Err(_) = reader.read_exact(&mut msg_buf) {
            break;
        }

        let msg = String::from_utf8_lossy(&msg_buf).to_string();
        // Send the intercepted command to the test runner for verification.
        let _ = tx.send(msg.clone());

        // Basic ADB Service Dispatching Logic
        if msg == "host:version" {
            // host:version returns a 4-byte hex length (0004) followed by the 4-byte version.
            // 0x0029 is the standard protocol version.
            let _ = stream.write_all(b"OKAY00040029");
        } else if msg == "host:features" || msg.ends_with(":features") {
            // host:features returns a length-prefixed string of features.
            let features = "remount,shell_v2";
            let resp = format!("OKAY{:04x}{}", features.len(), features);
            let _ = stream.write_all(resp.as_bytes());
        } else if msg.starts_with("host:tport") || msg.starts_with("host:transport") || msg.contains(":tport:") {
            // host:tport or host:transport-id is used to select a target device.
            // The server responds with OKAY and 8 raw bytes of the transport ID.
            let mut resp = b"OKAY".to_vec();
            resp.extend_from_slice(&(1u64).to_le_bytes());
            let _ = stream.write_all(&resp);
        } else if msg == "host:get-serialno" || msg.contains(":get-serialno") {
            let serial = "12345678";
            let resp = format!("OKAY{:04x}{}", serial.len(), serial);
            let _ = stream.write_all(resp.as_bytes());
        } else if msg == "host:get-devpath" || msg.contains(":get-devpath") {
            let path = "/dev/usb/001/002";
            let resp = format!("OKAY{:04x}{}", path.len(), path);
            let _ = stream.write_all(resp.as_bytes());
        } else if msg == "host:list-forward" || msg.contains(":list-forward") {
            let list = "tcp:1234 tcp:5678\n";
            let resp = format!("OKAY{:04x}{}", list.len(), list);
            let _ = stream.write_all(resp.as_bytes());
        } else if msg.starts_with("shell") {
             // Device-side shell command
            let _ = stream.write_all(b"OKAY");
            let _ = stream.write_all(b"shell output");
            // Shell commands are typically the last thing on a connection.
            // DO NOT break immediately, wait for client to read.
            let mut buf = [0u8; 1];
            let _ = reader.read(&mut buf);
            break;
        } else if msg.starts_with("remount") || msg.contains(":remount") {
            // remount service responds with OKAY and then a success message.
            let _ = stream.write_all(b"OKAY");
            let _ = stream.write_all(b"remount succeeded");
            let mut buf = [0u8; 1];
            let _ = reader.read(&mut buf);
            break;
        } else if msg == "host:devices" || msg == "host:devices-l" {
            let devices = "12345678\tdevice\n";
            let resp = format!("OKAY{:04x}{}", devices.len(), devices);
            let _ = stream.write_all(resp.as_bytes());
        } else {
            // Default response for unhandled commands.
            let _ = stream.write_all(b"OKAY");
        }
    }
    // Gracefully shut down the connection.
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}
