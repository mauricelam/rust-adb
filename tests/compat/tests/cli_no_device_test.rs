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

#[test]
fn test_no_args() {
    let config = Config::from_env();
    let output = compat::run_client_with_config(&config, &[]);
    assert_matches_golden("no_args", &output);
}

#[test]
fn test_help() {
    let config = Config::from_env();

    let output_help = compat::run_client_with_config(&config, &["help"]);
    assert_matches_golden("help", &output_help);

    let output_long = compat::run_client_with_config(&config, &["--help"]);
    assert_matches_golden("help_long", &output_long);

    let output_short = compat::run_client_with_config(&config, &["-h"]);
    assert_matches_golden("help_short", &output_short);
}

#[test]
fn test_version() {
    let config = Config::from_env();

    let output_version = compat::run_client_with_config(&config, &["version"]);
    assert_matches_golden("version", &output_version);

    let output_long = compat::run_client_with_config(&config, &["--version"]);
    assert_matches_golden("version_long", &output_long);
}

#[test]
fn test_unknown_command() {
    let config = Config::from_env();
    let output = compat::run_client_with_config(&config, &["nosuchcommand"]);
    assert_matches_golden("unknown_command", &output);
}

#[test]
fn test_global_option_errors() {
    let config = Config::from_env();

    let output_s = compat::run_client_with_config(&config, &["-s"]);
    assert_matches_golden("global_option_error_s", &output_s);

    let output_t = compat::run_client_with_config(&config, &["-t", "abc"]);
    assert_matches_golden("global_option_error_t", &output_t);

    let output_h = compat::run_client_with_config(&config, &["-H"]);
    assert_matches_golden("global_option_error_H", &output_h);

    let output_l_h =
        compat::run_client_with_config(&config, &["-L", "tcp:localhost:5037", "-H", "localhost"]);
    assert_matches_golden("global_option_error_L_H", &output_l_h);

    let output_p = compat::run_client_with_config(&config, &["-P", "99999"]);
    assert_matches_golden("global_option_error_P", &output_p);

    let output_d_e = compat::run_client_with_config(&config, &["-d", "-e"]);
    assert_matches_golden("global_option_error_d_e", &output_d_e);
}

#[test]
fn test_wrong_arg_counts() {
    let config = Config::from_env();

    let output_devices = compat::run_client_with_config(&config, &["devices", "extra"]);
    assert_matches_golden("wrong_arg_count_devices", &output_devices);

    let output_conn0 = compat::run_client_with_config(&config, &["connect"]);
    assert_matches_golden("wrong_arg_count_connect_0", &output_conn0);

    let output_conn2 = compat::run_client_with_config(&config, &["connect", "a", "b"]);
    assert_matches_golden("wrong_arg_count_connect_2", &output_conn2);

    let output_disconn2 = compat::run_client_with_config(&config, &["disconnect", "a", "b"]);
    assert_matches_golden("wrong_arg_count_disconnect_2", &output_disconn2);

    let output_track = compat::run_client_with_config(&config, &["track-devices", "--bogus"]);
    assert_matches_golden("wrong_arg_count_track_devices_bogus", &output_track);

    let output_fwd0 = compat::run_client_with_config(&config, &["forward"]);
    assert_matches_golden("wrong_arg_count_forward_0", &output_fwd0);

    let output_fwd_list = compat::run_client_with_config(&config, &["forward", "--list", "extra"]);
    assert_matches_golden("wrong_arg_count_forward_list_extra", &output_fwd_list);

    let output_fwd_rem = compat::run_client_with_config(&config, &["forward", "--remove"]);
    assert_matches_golden("wrong_arg_count_forward_remove_0", &output_fwd_rem);

    let output_fwd1 = compat::run_client_with_config(&config, &["forward", "a"]);
    assert_matches_golden("wrong_arg_count_forward_1", &output_fwd1);
}

#[test]
fn test_server_no_device() {
    let server = TestServer::start();

    let output_dev = server.run_client(&["devices"]);
    assert_matches_golden("server_no_device_devices", &output_dev);

    let output_dev_l = server.run_client(&["devices", "-l"]);
    assert_matches_golden("server_no_device_devices_l", &output_dev_l);

    let output_state = server.run_client(&["get-state"]);
    assert_matches_golden("server_no_device_get_state", &output_state);

    let output_serial = server.run_client(&["get-serialno"]);
    assert_matches_golden("server_no_device_get_serialno", &output_serial);

    let output_devpath = server.run_client(&["get-devpath"]);
    assert_matches_golden("server_no_device_get_devpath", &output_devpath);

    let output_trans_id = server.run_client(&["transport-id"]);
    assert_matches_golden("server_no_device_transport_id", &output_trans_id);

    let output_feat = server.run_client(&["features"]);
    assert_matches_golden("server_no_device_features", &output_feat);

    let output_shell = server.run_client(&["shell", "echo", "hi"]);
    assert_matches_golden("server_no_device_shell_echo_hi", &output_shell);

    let output_nosuchserial = server.run_client(&["-s", "nosuchserial", "get-state"]);
    assert_matches_golden(
        "server_no_device_nosuchserial_get_state",
        &output_nosuchserial,
    );

    let output_t99 = server.run_client(&["-t", "99", "get-state"]);
    assert_matches_golden("server_no_device_t99_get_state", &output_t99);
}

#[test]
fn test_server_lifecycle() {
    let config = Config::from_env();

    // kill-server with a server running
    {
        let server = TestServer::start_with_config(config.clone());
        let output = server.run_client(&["kill-server"]);
        assert_matches_golden("server_lifecycle_kill_running", &output);
    }

    // kill-server with none running
    {
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

    // start-server twice
    {
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
}
