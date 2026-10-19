use compat::TestServer;
use mock_adbd::MockAdbdServer;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mock_adbd_handshake_and_shell() {
    // Start mock-adbd in-process on a free local port
    let mock = MockAdbdServer::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind mock-adbd");
    let mock_addr = mock.local_addr();

    tokio::spawn(async move {
        if let Err(err) = mock.run().await {
            eprintln!("mock-adbd error: {err}");
        }
    });

    // Run synchronous client calls in spawn_blocking so Tokio runtime isn't starved
    tokio::task::spawn_blocking(move || {
        // Start reference ADB server with isolated home directory
        let server = TestServer::start();

        // Run `adb connect 127.0.0.1:<port>`
        let connect_output = server.run_client(&["connect", &mock_addr.to_string()]);
        assert!(
            connect_output.success(),
            "adb connect failed: stdout={}, stderr={}",
            connect_output.stdout,
            connect_output.stderr
        );

        // Assert `adb devices` lists the mock with state `device`
        let devices_output = server.run_client(&["devices"]);
        assert!(
            devices_output.success(),
            "adb devices failed: stdout={}, stderr={}",
            devices_output.stdout,
            devices_output.stderr
        );
        let serial = mock_addr.to_string();
        let found_device = devices_output
            .stdout
            .lines()
            .any(|line| line.contains(&serial) && line.contains("device"));
        assert!(
            found_device,
            "adb devices output did not contain '{}' with state 'device':\n{}",
            serial, devices_output.stdout
        );

        // Assert `adb -s 127.0.0.1:<port> shell echo hello` prints `hello` and exits 0
        let shell_output = server.run_client(&["-s", &serial, "shell", "echo", "hello"]);
        assert_eq!(
            shell_output.exit_code,
            Some(0),
            "adb shell exit code was not 0: stdout={}, stderr={}",
            shell_output.stdout,
            shell_output.stderr
        );
        assert_eq!(
            shell_output.stdout.trim(),
            "hello",
            "adb shell output mismatch: expected 'hello', got {:?}",
            shell_output.stdout
        );
    })
    .await
    .unwrap();
}
