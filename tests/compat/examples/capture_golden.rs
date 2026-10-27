use compat::{normalize_output, Config, GoldenCase, TestServer};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

struct TestCaseSpec {
    name: &'static str,
    args: Vec<&'static str>,
    env: Vec<(&'static str, &'static str)>,
    mode: TestMode,
}

enum TestMode {
    /// Run with a running daemon started by the harness
    WithServer,
    /// Run without a pre-started server (or server lifecycle test)
    WithoutServer,
    /// Custom lifecycle runner
    LifecycleStartTwice,
    LifecycleKillRunning,
    LifecycleKillNone,
}

fn get_reference_config() -> Config {
    let mut config = Config::from_env();
    if let Ok(ref_path) = std::env::var("ADB_REFERENCE") {
        let p = PathBuf::from(ref_path);
        config.client_path = p.clone();
        config.server_bin_path = p;
    }
    config
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let golden_dir = PathBuf::from(manifest_dir).join("golden");
    fs::create_dir_all(&golden_dir).expect("Failed to create golden directory");

    let config = get_reference_config();

    let cases = vec![
        // adb with no arguments
        TestCaseSpec {
            name: "no_args",
            args: vec![],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        // adb help, adb --help, adb -h
        TestCaseSpec {
            name: "help",
            args: vec!["help"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "help_long",
            args: vec!["--help"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "help_short",
            args: vec!["-h"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        // adb version and adb --version
        TestCaseSpec {
            name: "version",
            args: vec!["version"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "version_long",
            args: vec!["--version"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        // An unknown command
        TestCaseSpec {
            name: "unknown_command",
            args: vec!["nosuchcommand"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        // Global option errors
        TestCaseSpec {
            name: "global_option_error_s",
            args: vec!["-s"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "global_option_error_t",
            args: vec!["-t", "abc"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "global_option_error_H",
            args: vec!["-H"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "global_option_error_L_H",
            args: vec!["-L", "tcp:localhost:5037", "-H", "localhost"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "global_option_error_P",
            args: vec!["-P", "99999"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "global_option_error_d_e",
            args: vec!["-d", "-e"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        // Wrong argument counts
        TestCaseSpec {
            name: "wrong_arg_count_devices",
            args: vec!["devices", "extra"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_connect_0",
            args: vec!["connect"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_connect_2",
            args: vec!["connect", "a", "b"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_disconnect_2",
            args: vec!["disconnect", "a", "b"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_track_devices_bogus",
            args: vec!["track-devices", "--bogus"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_forward_0",
            args: vec!["forward"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_forward_list_extra",
            args: vec!["forward", "--list", "extra"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_forward_remove_0",
            args: vec!["forward", "--remove"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        TestCaseSpec {
            name: "wrong_arg_count_forward_1",
            args: vec!["forward", "a"],
            env: vec![],
            mode: TestMode::WithoutServer,
        },
        // With a server and no device
        TestCaseSpec {
            name: "server_no_device_devices",
            args: vec!["devices"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_devices_l",
            args: vec!["devices", "-l"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_get_state",
            args: vec!["get-state"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_get_serialno",
            args: vec!["get-serialno"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_get_devpath",
            args: vec!["get-devpath"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_transport_id",
            args: vec!["transport-id"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_features",
            args: vec!["features"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_shell_echo_hi",
            args: vec!["shell", "echo", "hi"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_nosuchserial_get_state",
            args: vec!["-s", "nosuchserial", "get-state"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        TestCaseSpec {
            name: "server_no_device_t99_get_state",
            args: vec!["-t", "99", "get-state"],
            env: vec![],
            mode: TestMode::WithServer,
        },
        // Server lifecycle
        TestCaseSpec {
            name: "server_lifecycle_kill_running",
            args: vec!["kill-server"],
            env: vec![],
            mode: TestMode::LifecycleKillRunning,
        },
        TestCaseSpec {
            name: "server_lifecycle_kill_none",
            args: vec!["kill-server"],
            env: vec![],
            mode: TestMode::LifecycleKillNone,
        },
        TestCaseSpec {
            name: "server_lifecycle_start_twice",
            args: vec!["start-server"],
            env: vec![],
            mode: TestMode::LifecycleStartTwice,
        },
    ];

    for spec in cases {
        println!("Capturing golden file for: {}", spec.name);
        let (raw_stdout, raw_stderr, exit_code) = match spec.mode {
            TestMode::WithoutServer => {
                let out = compat::run_client_with_config(&config, &spec.args);
                (out.stdout, out.stderr, out.exit_code)
            }
            TestMode::WithServer => {
                let server = TestServer::start_with_config(config.clone());
                let out = server.run_client(&spec.args);
                (out.stdout, out.stderr, out.exit_code)
            }
            TestMode::LifecycleKillRunning => {
                let server = TestServer::start_with_config(config.clone());
                let out = server.run_client(&["kill-server"]);
                (out.stdout, out.stderr, out.exit_code)
            }
            TestMode::LifecycleKillNone => {
                // Run kill-server when no daemon is listening on port
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
                (
                    String::from_utf8_lossy(&out.stdout).to_string(),
                    String::from_utf8_lossy(&out.stderr).to_string(),
                    out.status.code(),
                )
            }
            TestMode::LifecycleStartTwice => {
                let temp_dir = TempDir::new().expect("temp dir");
                let port = 58274;
                // First start-server
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

                // Second start-server
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

                (
                    String::from_utf8_lossy(&out2.stdout).to_string(),
                    String::from_utf8_lossy(&out2.stderr).to_string(),
                    out2.status.code(),
                )
            }
        };

        let golden_case = GoldenCase {
            args: spec.args.iter().map(|s| s.to_string()).collect(),
            env: spec
                .env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            stdout: normalize_output(&raw_stdout),
            stderr: normalize_output(&raw_stderr),
            exit_code,
        };

        let json_path = golden_dir.join(format!("{}.json", spec.name));
        let json_content = serde_json::to_string_pretty(&golden_case).unwrap() + "\n";
        fs::write(&json_path, json_content).expect("Write golden file");
    }

    println!("All golden files captured successfully.");
}
