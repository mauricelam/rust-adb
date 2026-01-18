use adb_harness::mock_server;
use adb_harness::runner;
use std::time::Duration;

#[test]
fn test_host_devices() {
    runner::run_adb_command(5037, &[]).unwrap();
    // Start the mock server and get its port and the receiver for the message.
    let (port, rx, _jh) = mock_server::start_mock_server().expect("Failed to start mock server");

    // Give the server thread a moment to start and bind the port.
    std::thread::sleep(Duration::from_millis(100));

    // Run the `devices` command.
    runner::run_adb_command(port, &["devices"]).unwrap();

    // Assert that the received message is correct.
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "host:version"
    );
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        "host:devices"
    );
}
