//! Integration tests for the ADB harness.
//!
//! These tests work by running a mock MITM server in between the ADB client and the ADB server (host-side).
//! The mock server intercepts the ADB commands and asserts that they are correct.

use adb_client_server_test::mock_server;
use adb_client_server_test::runner;
use std::time::Duration;
use std::sync::mpsc::Receiver;

fn wait_for_cmd(rx: &Receiver<String>, target: &str) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(cmd) = rx.recv_timeout(Duration::from_millis(100)) {
            println!("Got command: {}", cmd);
            if cmd.contains(target) {
                return true;
            }
        }
    }
    false
}

#[test]
fn test_host_devices() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let _ = runner::run_adb_command(port, &["devices"]);

    assert!(wait_for_cmd(&rx, "devices"), "Did not receive devices command");
}

#[test]
fn test_host_devices_l() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let _ = runner::run_adb_command(port, &["devices", "-l"]);

    assert!(wait_for_cmd(&rx, "devices-l"), "Did not receive devices-l command");
}

#[test]
#[cfg(not(target_os = "windows"))]
fn test_host_track_devices() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let mut child = runner::spawn_adb_command(port, &["track-devices"]).unwrap();

    assert!(wait_for_cmd(&rx, "track-devices"), "Did not receive track-devices command");

    child.kill().unwrap();
}

#[test]
fn test_remount() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let _ = runner::run_adb_command(port, &["remount"]);

    assert!(wait_for_cmd(&rx, "remount"), "Did not receive remount command");
}
