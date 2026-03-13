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

#[test]
fn test_version() {
    let (port, _rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let output = runner::run_adb_command(port, &["version"]).expect("Failed to run adb version");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Android Debug Bridge version"), "Output does not contain version info");
}

#[test]
fn test_get_serialno() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let output = runner::run_adb_command(port, &["get-serialno"]).expect("Failed to run adb get-serialno");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("12345678"), "Output does not contain expected serial number: {}", stdout);
    assert!(wait_for_cmd(&rx, "get-serialno"), "Did not receive get-serialno command");
}

#[test]
fn test_get_devpath() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let output = runner::run_adb_command(port, &["get-devpath"]).expect("Failed to run adb get-devpath");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("/dev/usb/001/002"), "Output does not contain expected devpath: {}", stdout);
    assert!(wait_for_cmd(&rx, "get-devpath"), "Did not receive get-devpath command");
}

#[test]
fn test_forward_list() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    let output = runner::run_adb_command(port, &["forward", "--list"]).expect("Failed to run adb forward --list");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("tcp:1234 tcp:5678"), "Output does not contain expected forward list: {}", stdout);
    assert!(wait_for_cmd(&rx, "list-forward"), "Did not receive list-forward command");
}

#[test]
fn test_shell_basic() {
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");
    std::thread::sleep(Duration::from_secs(1));

    // For shell commands, we use -e and pipe to ensure adb doesn't hang waiting for stdin
    let _ = runner::run_adb_command(port, &["-e", "shell", "ls"]);

    // We only assert that the command was sent.
    // 'shell' might be sent as 'shell:ls' or 'shell,v2...:ls'.
    assert!(wait_for_cmd(&rx, "shell"), "Did not receive shell command");
}
