use compat::{run_client, TestServer};

#[test]
fn test_adb_version_standalone() {
    let output = run_client(&["version"]);
    assert_eq!(
        output.exit_code,
        Some(0),
        "adb version exited with code {:?}, stderr: {}",
        output.exit_code,
        output.stderr
    );
    assert!(
        output.stdout.contains("Android Debug Bridge version")
            || output.stdout.contains("version")
            || output.stdout.contains("Version"),
        "Unexpected output from adb version: stdout: {}, stderr: {}",
        output.stdout,
        output.stderr
    );
}

#[test]
fn test_adb_version_with_server() {
    let server = TestServer::start();
    let output = server.run_client(&["version"]);
    assert_eq!(
        output.exit_code,
        Some(0),
        "adb version exited with code {:?}, stderr: {}",
        output.exit_code,
        output.stderr
    );
    assert!(
        output.stdout.contains("Android Debug Bridge version")
            || output.stdout.contains("version")
            || output.stdout.contains("Version"),
        "Unexpected output from adb version: stdout: {}, stderr: {}",
        output.stdout,
        output.stderr
    );
}
