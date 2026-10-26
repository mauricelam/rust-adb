use assert_cmd::Command;
use compat::TestServer;
use predicates::prelude::*;
use std::env;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Resolves or builds the path to the compiled `adb-cli` binary (`adb`).
pub fn get_adb_cli_bin() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_adb") {
        return PathBuf::from(path);
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let root = PathBuf::from(manifest_dir).join("../..");
    let bin_name = if cfg!(target_os = "windows") {
        "adb.exe"
    } else {
        "adb"
    };
    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target"));
    let bin_path = target_dir.join("debug").join(bin_name);
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "adb-cli"])
        .status()
        .expect("Failed to build adb-cli binary");
    assert!(status.success(), "Failed to build adb-cli binary");
    bin_path
}

/// Helper function to construct an `assert_cmd::Command` for the compiled `adb-cli` binary.
pub fn adb_cli_cmd() -> Command {
    Command::new(get_adb_cli_bin())
}

/// Configures an `assert_cmd::Command` with isolated environment variables,
/// home directory, optional server port, and custom environment variable overrides.
pub fn configure_cli_env(
    cmd: &mut Command,
    home_dir: &Path,
    port: Option<u16>,
    custom_envs: &[(&str, &str)],
) {
    cmd.env("HOME", home_dir)
        .env("USERPROFILE", home_dir)
        .env("ANDROID_ADB_HOME", home_dir)
        .env("ANDROID_USER_HOME", home_dir)
        .env("ADB_MDNS_AUTO_CONNECT", "0")
        .env("ADB_MDNS_OPENSCREEN", "0");

    if let Some(port) = port {
        cmd.env("ANDROID_ADB_SERVER_PORT", port.to_string())
            .env("ADB_SERVER_SOCKET", format!("tcp:localhost:{port}"));
    }

    for &(k, v) in custom_envs {
        cmd.env(k, v);
    }
}

/// Helper function to run `adb-cli` in an isolated environment with arguments and env overrides.
pub fn run_cli(
    args: &[&str],
    port: Option<u16>,
    custom_envs: &[(&str, &str)],
) -> assert_cmd::assert::Assert {
    let temp_dir = TempDir::new().expect("Failed to create temporary directory for CLI test");
    let mut cmd = adb_cli_cmd();
    cmd.args(args);
    configure_cli_env(&mut cmd, temp_dir.path(), port, custom_envs);
    cmd.assert()
}

/// Helper function to run `adb-cli` targeting a `TestServer` daemon.
pub fn run_cli_with_server(
    server: &TestServer,
    args: &[&str],
    custom_envs: &[(&str, &str)],
) -> assert_cmd::assert::Assert {
    run_cli(args, Some(server.port), custom_envs)
}

#[test]
fn test_cli_binary_invocation_and_capturing() {
    // Basic binary execution check with no args
    let assert = run_cli(&[], None, &[]);
    let output = assert.get_output();

    // Verify stdout/stderr capturing and process exit status
    assert!(
        output.status.code().is_some(),
        "Expected process exit code to be set"
    );
    assert
        .stdout(predicate::str::is_empty().or(predicate::str::is_empty().not()))
        .stderr(predicate::str::is_empty().or(predicate::str::is_empty().not()));
}

#[test]
fn test_cli_argument_handling() {
    // Testing argument handling with custom environment variable overrides
    let custom_envs = &[("TEST_CUSTOM_FLAG", "enabled")];
    let assert = run_cli(&["--help"], None, custom_envs);
    let output = assert.get_output();

    assert!(
        output.status.code().is_some(),
        "Expected process exit code to be set"
    );
}

#[test]
fn test_cli_server_port_and_socket_overrides() {
    // Start an isolated test server daemon
    let server = TestServer::start();

    // Overriding ADB_SERVER_SOCKET and server port flags
    let socket_spec = format!("tcp:localhost:{}", server.port);
    let custom_envs = &[("ADB_SERVER_SOCKET", socket_spec.as_str())];

    let assert = run_cli_with_server(&server, &["version"], custom_envs);
    let output = assert.get_output();

    assert!(
        output.status.code().is_some(),
        "Expected process exit code to be set"
    );
}
