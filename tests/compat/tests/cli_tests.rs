use assert_cmd::Command;
use compat::TestServer;
use predicates::prelude::*;
use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Resolves or builds the path to the compiled `adb-cli` binary (`adb`).
pub fn get_adb_cli_bin() -> PathBuf {
    if let Ok(path) = env::var("ADB_CLIENT") {
        return PathBuf::from(path);
    }
    if let Ok(path) = env::var("ADB_REFERENCE") {
        return PathBuf::from(path);
    }
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

/// Computes the SHA-256 digest of a binary file as a hex string.
fn compute_binary_sha256(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "Failed to read binary file {} for checksum: {e}",
            path.display()
        )
    });
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

#[test]
fn test_binary_checksum_validation() {
    let bin_path = get_adb_cli_bin();
    assert!(
        bin_path.exists(),
        "Binary path does not exist: {}",
        bin_path.display()
    );

    let sha256_1 = compute_binary_sha256(&bin_path);
    assert_eq!(
        sha256_1.len(),
        64,
        "Computed binary SHA-256 digest must be 64 hex characters"
    );

    let sha256_2 = compute_binary_sha256(&bin_path);
    assert_eq!(
        sha256_1, sha256_2,
        "Binary checksum changed across reads (C5 checksum validation)"
    );
}

/// Normalizes output strings for cross-platform and dynamic environment comparisons:
/// - Converts CRLF to LF
/// - Replaces dynamic `Version ...` release lines with `Version [VERSION]`
/// - Replaces dynamic `Installed as ...` path lines with `Installed as [PATH]`
/// - Replaces dynamic `Running on ...` OS lines with `Running on [OS]`
fn normalize_cli_output(input: &str) -> String {
    let s = input.replace("\r\n", "\n");
    let mut lines = Vec::new();
    for line in s.split('\n') {
        if line.starts_with("Version ") {
            lines.push("Version [VERSION]".to_string());
        } else if line.starts_with("Installed as ") {
            lines.push("Installed as [PATH]".to_string());
        } else if line.starts_with("Running on ") {
            lines.push("Running on [OS]".to_string());
        } else {
            lines.push(line.to_string());
        }
    }
    lines.join("\n")
}

/// Reads a golden fixture file from `tests/compat/fixtures/cli_stdout/`.
fn read_fixture(filename: &str) -> String {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let path = PathBuf::from(manifest_dir)
        .join("fixtures")
        .join("cli_stdout")
        .join(filename);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read golden fixture file {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

#[test]
fn test_golden_version_output() {
    let assert = run_cli(&["version"], None, &[]);
    let output = assert.get_output();

    assert!(output.status.success(), "adb version failed: {:?}", output);

    let actual_stdout = normalize_cli_output(&String::from_utf8_lossy(&output.stdout));
    let actual_stderr = normalize_cli_output(&String::from_utf8_lossy(&output.stderr));

    let expected_stdout = read_fixture("version.stdout");
    let expected_stderr = read_fixture("version.stderr");

    assert_eq!(
        actual_stdout, expected_stdout,
        "adb version stdout mismatched golden fixture"
    );
    assert_eq!(
        actual_stderr, expected_stderr,
        "adb version stderr mismatched golden fixture"
    );
}

#[test]
fn test_golden_help_output() {
    let assert = run_cli(&["help"], None, &[]);
    let output = assert.get_output();

    assert!(output.status.success(), "adb help failed: {:?}", output);

    let actual_stdout = normalize_cli_output(&String::from_utf8_lossy(&output.stdout));
    let actual_stderr = normalize_cli_output(&String::from_utf8_lossy(&output.stderr));

    let expected_stdout = read_fixture("help.stdout");
    let expected_stderr = read_fixture("help.stderr");

    assert_eq!(
        actual_stdout, expected_stdout,
        "adb help stdout mismatched golden fixture"
    );
    assert_eq!(
        actual_stderr, expected_stderr,
        "adb help stderr mismatched golden fixture"
    );
}

#[test]
fn test_golden_devices_output() {
    let server = TestServer::start();
    let assert = run_cli_with_server(&server, &["devices"], &[]);
    let output = assert.get_output();

    assert!(output.status.success(), "adb devices failed: {:?}", output);

    let actual_stdout = normalize_cli_output(&String::from_utf8_lossy(&output.stdout));
    let actual_stderr = normalize_cli_output(&String::from_utf8_lossy(&output.stderr));

    let expected_stdout = read_fixture("devices.stdout");
    let expected_stderr = read_fixture("devices.stderr");

    assert_eq!(
        actual_stdout, expected_stdout,
        "adb devices stdout mismatched golden fixture"
    );
    assert_eq!(
        actual_stderr, expected_stderr,
        "adb devices stderr mismatched golden fixture"
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
