use compat::{normalize_output, Config, GoldenCase, IsolatedServer, TestOutput, TestServer};
use std::fs;
use std::path::PathBuf;

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
    let mut server = IsolatedServer::new();
    let start_output = server.start_server();
    assert!(
        start_output.success(),
        "start-server failed: stdout='{}', stderr='{}'",
        start_output.stdout,
        start_output.stderr
    );
    let output = server.run_client(&["kill-server"]);
    assert_matches_golden("server_lifecycle_kill_running", &output);
}

#[test]
fn test_server_lifecycle_kill_none() {
    let server = IsolatedServer::new();
    let output = server.run_client(&["kill-server"]);
    assert_matches_golden("server_lifecycle_kill_none", &output);
}

#[test]
fn test_server_lifecycle_start_twice() {
    let mut server = IsolatedServer::new();
    let start1 = server.start_server();
    assert!(
        start1.success(),
        "first start-server failed: stdout='{}', stderr='{}'",
        start1.stdout,
        start1.stderr
    );

    let start2 = server.run_client(&["start-server"]);
    assert_matches_golden("server_lifecycle_start_twice", &start2);
}
