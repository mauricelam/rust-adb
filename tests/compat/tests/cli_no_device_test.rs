use compat::{normalize_output, Config, GoldenCase, TestOutput, TestServer};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn load_golden(case_name: &str) -> GoldenCase {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let golden_path = PathBuf::from(manifest_dir)
        .join("golden")
        .join(format!("{case_name}.json"));
    let content = fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("Failed to read golden file {}: {e}", golden_path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse golden file {}: {e}", golden_path.display()))
}

fn assert_matches_golden(case_name: &str, actual: &TestOutput) {
    let golden = load_golden(case_name);
    let norm_stdout = normalize_output(&actual.stdout);
    let norm_stderr = normalize_output(&actual.stderr);

    assert_eq!(
        norm_stdout, golden.stdout,
        "stdout mismatch for golden test case '{case_name}'"
    );
    assert_eq!(
        norm_stderr, golden.stderr,
        "stderr mismatch for golden test case '{case_name}'"
    );
    assert_eq!(
        actual.exit_code, golden.exit_code,
        "exit_code mismatch for golden test case '{case_name}'"
    );
}

fn check_no_device(args: &[&str], case_name: &str) {
    let config = Config::from_env();
    let output = compat::run_client_with_config(&config, args);
    assert_matches_golden(case_name, &output);
}

fn check_server_no_device(args: &[&str], case_name: &str) {
    let server = TestServer::start();
    let output = server.run_client(args);
    assert_matches_golden(case_name, &output);
}

#[test]
fn test_no_args() {
    check_no_device(&[], "no_args");
}

#[test]
fn test_help() {
    check_no_device(&["help"], "help");
}

#[test]
fn test_help_long() {
    check_no_device(&["--help"], "help_long");
}

#[test]
fn test_help_short() {
    check_no_device(&["-h"], "help_short");
}

#[test]
fn test_version() {
    check_no_device(&["version"], "version");
}

#[test]
fn test_version_long() {
    check_no_device(&["--version"], "version_long");
}

#[test]
fn test_unknown_command() {
    check_no_device(&["nosuchcommand"], "unknown_command");
}

#[test]
fn test_global_option_error_s() {
    check_no_device(&["-s"], "global_option_error_s");
}

#[test]
fn test_global_option_error_t() {
    check_no_device(&["-t", "abc"], "global_option_error_t");
}

#[test]
fn test_global_option_error_h() {
    check_no_device(&["-H"], "global_option_error_H");
}

#[test]
fn test_global_option_error_l_h() {
    check_no_device(
        &["-L", "tcp:localhost:5037", "-H", "localhost"],
        "global_option_error_L_H",
    );
}

#[test]
fn test_global_option_error_p() {
    check_no_device(&["-P", "99999"], "global_option_error_P");
}

#[test]
fn test_global_option_error_d_e() {
    check_no_device(&["-d", "-e"], "global_option_error_d_e");
}

#[test]
fn test_wrong_arg_count_devices() {
    check_no_device(&["devices", "extra"], "wrong_arg_count_devices");
}

#[test]
fn test_wrong_arg_count_connect_0() {
    check_no_device(&["connect"], "wrong_arg_count_connect_0");
}

#[test]
fn test_wrong_arg_count_connect_2() {
    check_no_device(&["connect", "a", "b"], "wrong_arg_count_connect_2");
}

#[test]
fn test_wrong_arg_count_disconnect_2() {
    check_no_device(&["disconnect", "a", "b"], "wrong_arg_count_disconnect_2");
}

#[test]
fn test_wrong_arg_count_track_devices_bogus() {
    check_no_device(
        &["track-devices", "--bogus"],
        "wrong_arg_count_track_devices_bogus",
    );
}

#[test]
fn test_wrong_arg_count_forward_0() {
    check_no_device(&["forward"], "wrong_arg_count_forward_0");
}

#[test]
fn test_wrong_arg_count_forward_list_extra() {
    check_no_device(
        &["forward", "--list", "extra"],
        "wrong_arg_count_forward_list_extra",
    );
}

#[test]
fn test_wrong_arg_count_forward_remove_0() {
    check_no_device(&["forward", "--remove"], "wrong_arg_count_forward_remove_0");
}

#[test]
fn test_wrong_arg_count_forward_1() {
    check_no_device(&["forward", "a"], "wrong_arg_count_forward_1");
}

#[test]
fn test_server_no_device_devices() {
    check_server_no_device(&["devices"], "server_no_device_devices");
}

#[test]
fn test_server_no_device_devices_l() {
    check_server_no_device(&["devices", "-l"], "server_no_device_devices_l");
}

#[test]
fn test_server_no_device_get_state() {
    check_server_no_device(&["get-state"], "server_no_device_get_state");
}

#[test]
fn test_server_no_device_get_serialno() {
    check_server_no_device(&["get-serialno"], "server_no_device_get_serialno");
}

#[test]
fn test_server_no_device_get_devpath() {
    check_server_no_device(&["get-devpath"], "server_no_device_get_devpath");
}

#[test]
fn test_server_no_device_transport_id() {
    check_server_no_device(&["transport-id"], "server_no_device_transport_id");
}

#[test]
fn test_server_no_device_features() {
    check_server_no_device(&["features"], "server_no_device_features");
}

#[test]
fn test_server_no_device_shell_echo_hi() {
    check_server_no_device(&["shell", "echo", "hi"], "server_no_device_shell_echo_hi");
}

#[test]
fn test_server_no_device_nosuchserial_get_state() {
    check_server_no_device(
        &["-s", "nosuchserial", "get-state"],
        "server_no_device_nosuchserial_get_state",
    );
}

#[test]
fn test_server_no_device_t99_get_state() {
    check_server_no_device(&["-t", "99", "get-state"], "server_no_device_t99_get_state");
}

#[test]
fn test_server_lifecycle_kill_running() {
    let config = Config::from_env();
    let server = TestServer::start_with_config(config);
    let output = server.run_client(&["kill-server"]);
    assert_matches_golden("server_lifecycle_kill_running", &output);
}

#[test]
fn test_server_lifecycle_kill_none() {
    let config = Config::from_env();
    let temp_dir = TempDir::new().expect("temp dir");
    let mut cmd = std::process::Command::new(&config.client_path);
    cmd.args(["-P", "58273", "kill-server"]);
    cmd.env("HOME", temp_dir.path());
    cmd.env("USERPROFILE", temp_dir.path());
    cmd.env("ANDROID_ADB_HOME", temp_dir.path());
    cmd.env("ANDROID_USER_HOME", temp_dir.path());
    cmd.env("ADB_MDNS_AUTO_CONNECT", "0");
    cmd.env("ADB_MDNS_OPENSCREEN", "0");
    cmd.env("ANDROID_ADB_SERVER_PORT", "58273");
    cmd.env("ADB_SERVER_SOCKET", "tcp:localhost:58273");

    let out = cmd.output().expect("execute kill-server");
    let test_out = TestOutput {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        exit_code: out.status.code(),
        status: out.status,
    };
    assert_matches_golden("server_lifecycle_kill_none", &test_out);
}

#[test]
fn test_server_lifecycle_start_twice() {
    let config = Config::from_env();
    let temp_dir = TempDir::new().expect("temp dir");
    let port = 58274;
    let mut cmd1 = std::process::Command::new(&config.client_path);
    cmd1.args(["-P", &port.to_string(), "start-server"]);
    cmd1.env("HOME", temp_dir.path());
    cmd1.env("USERPROFILE", temp_dir.path());
    cmd1.env("ANDROID_ADB_HOME", temp_dir.path());
    cmd1.env("ANDROID_USER_HOME", temp_dir.path());
    cmd1.env("ADB_MDNS_AUTO_CONNECT", "0");
    cmd1.env("ADB_MDNS_OPENSCREEN", "0");
    cmd1.env("ANDROID_ADB_SERVER_PORT", port.to_string());
    cmd1.env("ADB_SERVER_SOCKET", format!("tcp:localhost:{port}"));
    let _ = cmd1.output();

    let mut cmd2 = std::process::Command::new(&config.client_path);
    cmd2.args(["-P", &port.to_string(), "start-server"]);
    cmd2.env("HOME", temp_dir.path());
    cmd2.env("USERPROFILE", temp_dir.path());
    cmd2.env("ANDROID_ADB_HOME", temp_dir.path());
    cmd2.env("ANDROID_USER_HOME", temp_dir.path());
    cmd2.env("ADB_MDNS_AUTO_CONNECT", "0");
    cmd2.env("ADB_MDNS_OPENSCREEN", "0");
    cmd2.env("ANDROID_ADB_SERVER_PORT", port.to_string());
    cmd2.env("ADB_SERVER_SOCKET", format!("tcp:localhost:{port}"));
    let out2 = cmd2.output().expect("execute second start-server");

    // Cleanup server on port
    let mut kill_cmd = std::process::Command::new(&config.client_path);
    kill_cmd.args(["-P", &port.to_string(), "kill-server"]);
    kill_cmd.env("HOME", temp_dir.path());
    let _ = kill_cmd.output();

    let test_out = TestOutput {
        stdout: String::from_utf8_lossy(&out2.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out2.stderr).to_string(),
        exit_code: out2.status.code(),
        status: out2.status,
    };
    assert_matches_golden("server_lifecycle_start_twice", &test_out);
}
