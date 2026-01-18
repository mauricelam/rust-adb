use adb_harness::mock_server;
use adb_harness::runner;
use std::time::Duration;

#[test]
#[ignore] // TODO: Fix the timeout issue.
fn test_host_devices() {
    // Start the mock server and get its port and the receiver for the message.
    let (port, rx) = mock_server::start_mock_server().expect("Failed to start mock server");

    // Give the server thread a moment to start and bind the port.
    std::thread::sleep(Duration::from_millis(100));

    // Run the `devices` command.
    let output = runner::run_adb_command(port, &["devices"]).expect("Failed to run adb command");

    // Assert that the command executed successfully.
    assert!(output.status.success(), "adb command failed with stderr: {}", String::from_utf8_lossy(&output.stderr));

    // Wait for the message from the mock server.
    let received_message = rx.recv_timeout(Duration::from_secs(1)).expect("Failed to receive message from mock server");

    // Assert that the received message is correct.
    assert_eq!(received_message, "host:devices");
}
