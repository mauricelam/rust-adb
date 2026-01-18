use adb_harness::mock_server;
use adb_harness::runner;
use std::process::Command;
use std::time::Duration;

#[test]
fn test_host_devices() {
    runner::run_adb_command(5037, &["devices"]).unwrap();
    // Start the mock server and get its port and the receiver for the message.
    let (port, rx) = mock_server::start_mock_server().expect("Failed to start mock server");

    // Give the server thread a moment to start and bind the port.
    std::thread::sleep(Duration::from_secs(1));

    // Run the `devices` command.
    runner::run_adb_command(port, &["devices"]).unwrap();

    // Assert that the received message is correct.
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "host:version"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "host:devices"
    );
}

#[test]
fn test_host_devices_l() {
    runner::run_adb_command(5037, &["devices"]).unwrap();
    // Start the mock server and get its port and the receiver for the message.
    let (port, rx) = mock_server::start_mock_server().expect("Failed to start mock server");

    // Give the server thread a moment to start and bind the port.
    std::thread::sleep(Duration::from_secs(1));

    // Run the `devices -l` command.
    runner::run_adb_command(port, &["devices", "-l"]).unwrap();

    // Assert that the received messages are correct.
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "host:version"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "host:devices-l"
    );
}

#[test]
#[cfg(not(target_os = "windows"))]
fn test_host_track_devices() {
    runner::run_adb_command(5037, &["devices"]).unwrap();
    // Start the mock server and get its port and the receiver for the message.
    let (port, rx) = mock_server::start_mock_server().expect("Failed to start mock server");

    // Give the server thread a moment to start and bind the port.
    std::thread::sleep(Duration::from_secs(1));

    // Path to the adb binary, relative to the workspace root
    #[cfg(target_os = "linux")]
    let adb_path = "../binaries/linux/adb";
    #[cfg(target_os = "macos")]
    let adb_path = "../binaries/mac/adb";

    // Run the `track-devices` command. Since this command doesn't exit,
    // we spawn it and then kill it after we've received the message.
    let mut child = Command::new(adb_path)
        .args(["-P", &port.to_string(), "track-devices"])
        .spawn()
        .unwrap();

    // Assert that the received messages are correct.
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "host:version"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "host:track-devices"
    );

    child.kill().unwrap();
}
