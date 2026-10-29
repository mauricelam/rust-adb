use compat::{normalize_output, Config, GoldenCase, TestOutput, TestServer};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tempfile::TempDir;

fn get_mock_adbd_bin() -> PathBuf {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_dir).join("../..");
    let bin_name = if cfg!(target_os = "windows") {
        "mock-adbd.exe"
    } else {
        "mock-adbd"
    };
    let bin_path = root.join("target").join("debug").join(bin_name);
    if !bin_path.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "mock-adbd"])
            .status()
            .expect("Failed to build mock-adbd binary");
        assert!(status.success(), "Failed to build mock-adbd");
    }
    bin_path
}

pub struct MockAdbdProcess {
    pub child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    pub fn start() -> Self {
        Self::start_with_args(&[])
    }

    pub fn start_with_args(extra_args: &[&str]) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen").arg("127.0.0.1:0");
        for arg in extra_args {
            cmd.arg(arg);
        }
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn mock-adbd binary at {:?}: {}", bin, e));

        let stdout = child.stdout.take().expect("Failed to open stdout");
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("Failed to read first line from mock-adbd stdout");
        let addr = first_line.trim().to_string();

        MockAdbdProcess { child, addr }
    }
}

impl Drop for MockAdbdProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

fn save_golden(golden_dir: &std::path::Path, case_name: &str, args: &[&str], out: &TestOutput) {
    let golden_case = GoldenCase {
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
        stdout: normalize_output(&out.stdout),
        stderr: normalize_output(&out.stderr),
        exit_code: out.exit_code,
    };

    let json_path = golden_dir.join(format!("{case_name}.json"));
    let json_content = serde_json::to_string_pretty(&golden_case).unwrap() + "\n";
    fs::write(&json_path, json_content).expect("Write golden file");
    println!("Saved golden case: {case_name}");
    let _ = std::io::stdout().flush();
}

fn run_client_with_stdin(server: &TestServer, args: &[&str], input: &[u8]) -> TestOutput {
    let config = Config::from_env();
    let mut cmd = Command::new(&config.client_path);
    cmd.arg("-P").arg(server.port.to_string());
    cmd.args(args);
    cmd.env("HOME", server.home_dir.path());
    cmd.env("USERPROFILE", server.home_dir.path());
    cmd.env("ANDROID_ADB_HOME", server.home_dir.path());
    cmd.env("ANDROID_USER_HOME", server.home_dir.path());
    cmd.env("ADB_MDNS_AUTO_CONNECT", "0");
    cmd.env("ADB_MDNS_OPENSCREEN", "0");
    cmd.env(
        "ADB_SERVER_SOCKET",
        format!("tcp:localhost:{}", server.port),
    );
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("Failed to spawn adb client");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let output = child
        .wait_with_output()
        .expect("Failed to wait on adb client");
    TestOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        status: output.status,
    }
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let golden_dir = PathBuf::from(manifest_dir).join("golden");
    fs::create_dir_all(&golden_dir).expect("Failed to create golden directory");

    let config = get_reference_config();

    // 1. Without server
    {
        let no_args = compat::run_client_with_config(&config, &[]);
        save_golden(&golden_dir, "no_args", &[], &no_args);

        let help = compat::run_client_with_config(&config, &["help"]);
        save_golden(&golden_dir, "help", &["help"], &help);

        let help_long = compat::run_client_with_config(&config, &["--help"]);
        save_golden(&golden_dir, "help_long", &["--help"], &help_long);

        let help_short = compat::run_client_with_config(&config, &["-h"]);
        save_golden(&golden_dir, "help_short", &["-h"], &help_short);

        let version = compat::run_client_with_config(&config, &["version"]);
        save_golden(&golden_dir, "version", &["version"], &version);

        let version_long = compat::run_client_with_config(&config, &["--version"]);
        save_golden(&golden_dir, "version_long", &["--version"], &version_long);

        let unknown = compat::run_client_with_config(&config, &["nosuchcommand"]);
        save_golden(&golden_dir, "unknown_command", &["nosuchcommand"], &unknown);

        save_golden(
            &golden_dir,
            "global_option_error_s",
            &["-s"],
            &compat::run_client_with_config(&config, &["-s"]),
        );
        save_golden(
            &golden_dir,
            "global_option_error_t",
            &["-t", "abc"],
            &compat::run_client_with_config(&config, &["-t", "abc"]),
        );
        save_golden(
            &golden_dir,
            "global_option_error_H",
            &["-H"],
            &compat::run_client_with_config(&config, &["-H"]),
        );
        save_golden(
            &golden_dir,
            "global_option_error_L_H",
            &["-L", "tcp:localhost:5037", "-H", "localhost"],
            &compat::run_client_with_config(
                &config,
                &["-L", "tcp:localhost:5037", "-H", "localhost"],
            ),
        );
        save_golden(
            &golden_dir,
            "global_option_error_P",
            &["-P", "99999"],
            &compat::run_client_with_config(&config, &["-P", "99999"]),
        );
        save_golden(
            &golden_dir,
            "global_option_error_d_e",
            &["-d", "-e"],
            &compat::run_client_with_config(&config, &["-d", "-e"]),
        );

        save_golden(
            &golden_dir,
            "wrong_arg_count_devices",
            &["devices", "extra"],
            &compat::run_client_with_config(&config, &["devices", "extra"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_connect_0",
            &["connect"],
            &compat::run_client_with_config(&config, &["connect"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_connect_2",
            &["connect", "a", "b"],
            &compat::run_client_with_config(&config, &["connect", "a", "b"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_disconnect_2",
            &["disconnect", "a", "b"],
            &compat::run_client_with_config(&config, &["disconnect", "a", "b"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_track_devices_bogus",
            &["track-devices", "--bogus"],
            &compat::run_client_with_config(&config, &["track-devices", "--bogus"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_forward_0",
            &["forward"],
            &compat::run_client_with_config(&config, &["forward"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_forward_list_extra",
            &["forward", "--list", "extra"],
            &compat::run_client_with_config(&config, &["forward", "--list", "extra"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_forward_remove_0",
            &["forward", "--remove"],
            &compat::run_client_with_config(&config, &["forward", "--remove"]),
        );
        save_golden(
            &golden_dir,
            "wrong_arg_count_forward_1",
            &["forward", "a"],
            &compat::run_client_with_config(&config, &["forward", "a"]),
        );
    }

    // 2. Server without device
    {
        let server = TestServer::start_with_config(config.clone());
        save_golden(
            &golden_dir,
            "server_no_device_devices",
            &["devices"],
            &server.run_client(&["devices"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_devices_l",
            &["devices", "-l"],
            &server.run_client(&["devices", "-l"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_get_state",
            &["get-state"],
            &server.run_client(&["get-state"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_get_serialno",
            &["get-serialno"],
            &server.run_client(&["get-serialno"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_get_devpath",
            &["get-devpath"],
            &server.run_client(&["get-devpath"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_transport_id",
            &["transport-id"],
            &server.run_client(&["transport-id"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_features",
            &["features"],
            &server.run_client(&["features"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_shell_echo_hi",
            &["shell", "echo", "hi"],
            &server.run_client(&["shell", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_nosuchserial_get_state",
            &["-s", "nosuchserial", "get-state"],
            &server.run_client(&["-s", "nosuchserial", "get-state"]),
        );
        save_golden(
            &golden_dir,
            "server_no_device_t99_get_state",
            &["-t", "99", "get-state"],
            &server.run_client(&["-t", "99", "get-state"]),
        );
    }

    // 3. Server lifecycle
    {
        let server = TestServer::start_with_config(config.clone());
        save_golden(
            &golden_dir,
            "server_lifecycle_kill_running",
            &["kill-server"],
            &server.run_client(&["kill-server"]),
        );

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
        let kill_none = TestOutput {
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            exit_code: out.status.code(),
            status: out.status,
        };
        save_golden(
            &golden_dir,
            "server_lifecycle_kill_none",
            &["kill-server"],
            &kill_none,
        );

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

        let mut kill_cmd = std::process::Command::new(&config.client_path);
        kill_cmd.args(["-P", &port.to_string(), "kill-server"]);
        kill_cmd.env("HOME", temp_dir.path());
        let _ = kill_cmd.output();

        let start_twice = TestOutput {
            stdout: String::from_utf8_lossy(&out2.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out2.stderr).to_string(),
            exit_code: out2.status.code(),
            status: out2.status,
        };
        save_golden(
            &golden_dir,
            "server_lifecycle_start_twice",
            &["start-server"],
            &start_twice,
        );
    }

    // 4. Mock device tests (Group 1: List & properties)
    {
        let mock = MockAdbdProcess::start();
        let server = TestServer::start_with_config(config.clone());
        let _ = server.run_client(&["connect", &mock.addr]);

        save_golden(
            &golden_dir,
            "mock_devices",
            &["devices"],
            &server.run_client(&["devices"]),
        );
        save_golden(
            &golden_dir,
            "mock_devices_l",
            &["devices", "-l"],
            &server.run_client(&["devices", "-l"]),
        );
        save_golden(
            &golden_dir,
            "mock_get_state",
            &["get-state"],
            &server.run_client(&["get-state"]),
        );
        save_golden(
            &golden_dir,
            "mock_get_serialno",
            &["get-serialno"],
            &server.run_client(&["get-serialno"]),
        );
        save_golden(
            &golden_dir,
            "mock_get_devpath",
            &["get-devpath"],
            &server.run_client(&["get-devpath"]),
        );
        save_golden(
            &golden_dir,
            "mock_transport_id",
            &["transport-id"],
            &server.run_client(&["transport-id"]),
        );
        save_golden(
            &golden_dir,
            "mock_features",
            &["features"],
            &server.run_client(&["features"]),
        );
        save_golden(
            &golden_dir,
            "mock_host_features",
            &["host-features"],
            &server.run_client(&["host-features"]),
        );
        save_golden(
            &golden_dir,
            "mock_s_get_state",
            &["-s", "<mock_addr>", "get-state"],
            &server.run_client(&["-s", &mock.addr, "get-state"]),
        );
        save_golden(
            &golden_dir,
            "mock_t_get_state",
            &["-t", "1", "get-state"],
            &server.run_client(&["-t", "1", "get-state"]),
        );
        save_golden(
            &golden_dir,
            "mock_d_get_state",
            &["-d", "get-state"],
            &server.run_client(&["-d", "get-state"]),
        );
        save_golden(
            &golden_dir,
            "mock_e_get_state",
            &["-e", "get-state"],
            &server.run_client(&["-e", "get-state"]),
        );

        // Shell v2
        save_golden(
            &golden_dir,
            "mock_shell_echo",
            &["-s", "<mock_addr>", "shell", "echo", "hello"],
            &server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]),
        );
        save_golden(
            &golden_dir,
            "mock_shell_exit_3",
            &["-s", "<mock_addr>", "shell", "exit", "3"],
            &server.run_client(&["-s", &mock.addr, "shell", "exit", "3"]),
        );
        save_golden(
            &golden_dir,
            "mock_shell_stderr",
            &["-s", "<mock_addr>", "shell", "stderr", "oops"],
            &server.run_client(&["-s", &mock.addr, "shell", "stderr", "oops"]),
        );
        save_golden(
            &golden_dir,
            "mock_shell_no_pty_echo",
            &["-s", "<mock_addr>", "shell", "-T", "echo", "hi"],
            &server.run_client(&["-s", &mock.addr, "shell", "-T", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "mock_shell_no_v2_flag_echo",
            &["-s", "<mock_addr>", "shell", "-x", "echo", "hi"],
            &server.run_client(&["-s", &mock.addr, "shell", "-x", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "mock_shell_n_cat",
            &["-s", "<mock_addr>", "shell", "-n", "cat"],
            &server.run_client(&["-s", &mock.addr, "shell", "-n", "cat"]),
        );
        save_golden(
            &golden_dir,
            "mock_shell_piped_cat",
            &["-s", "<mock_addr>", "shell", "cat"],
            &run_client_with_stdin(
                &server,
                &["-s", &mock.addr, "shell", "cat"],
                b"hello from stdin\n",
            ),
        );
        save_golden(
            &golden_dir,
            "mock_exec_out_echo",
            &["-s", "<mock_addr>", "exec-out", "echo", "hi"],
            &server.run_client(&["-s", &mock.addr, "exec-out", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "mock_exec_in_cat",
            &["-s", "<mock_addr>", "exec-in", "cat"],
            &run_client_with_stdin(
                &server,
                &["-s", &mock.addr, "exec-in", "cat"],
                b"exec-in payload\n",
            ),
        );
        save_golden(
            &golden_dir,
            "mock_shell_unsupported",
            &["-s", "<mock_addr>", "shell", "unsupported_cmd"],
            &server.run_client(&["-s", &mock.addr, "shell", "unsupported_cmd"]),
        );

        // Forward
        save_golden(
            &golden_dir,
            "mock_forward_list_empty",
            &["-s", "<mock_addr>", "forward", "--list"],
            &server.run_client(&["-s", &mock.addr, "forward", "--list"]),
        );
        let fwd_add = server.run_client(&["-s", &mock.addr, "forward", "tcp:0", "tcp:1234"]);
        save_golden(
            &golden_dir,
            "mock_forward_add_zero",
            &["-s", "<mock_addr>", "forward", "tcp:0", "tcp:1234"],
            &fwd_add,
        );

        let alloc_port = fwd_add.stdout.trim().to_string();
        save_golden(
            &golden_dir,
            "mock_forward_list_one",
            &["-s", "<mock_addr>", "forward", "--list"],
            &server.run_client(&["-s", &mock.addr, "forward", "--list"]),
        );

        let fwd_no_rebind = server.run_client(&[
            "-s",
            &mock.addr,
            "forward",
            "--no-rebind",
            &format!("tcp:{alloc_port}"),
            "tcp:1234",
        ]);
        save_golden(
            &golden_dir,
            "mock_forward_no_rebind_existing",
            &[
                "-s",
                "<mock_addr>",
                "forward",
                "--no-rebind",
                "tcp:<alloc_port>",
                "tcp:1234",
            ],
            &fwd_no_rebind,
        );

        let fwd_remove = server.run_client(&[
            "-s",
            &mock.addr,
            "forward",
            "--remove",
            &format!("tcp:{alloc_port}"),
        ]);
        save_golden(
            &golden_dir,
            "mock_forward_remove",
            &[
                "-s",
                "<mock_addr>",
                "forward",
                "--remove",
                "tcp:<alloc_port>",
            ],
            &fwd_remove,
        );

        let _ = server.run_client(&["-s", &mock.addr, "forward", "tcp:0", "tcp:1234"]);
        save_golden(
            &golden_dir,
            "mock_forward_remove_all",
            &["-s", "<mock_addr>", "forward", "--remove-all"],
            &server.run_client(&["-s", &mock.addr, "forward", "--remove-all"]),
        );

        // Sync
        save_golden(
            &golden_dir,
            "mock_sync_ls_root_v2",
            &["-s", "<mock_addr>", "ls", "/"],
            &server.run_client(&["-s", &mock.addr, "ls", "/"]),
        );
        save_golden(
            &golden_dir,
            "mock_sync_ls_data_v2",
            &["-s", "<mock_addr>", "ls", "/data"],
            &server.run_client(&["-s", &mock.addr, "ls", "/data"]),
        );

        let tmp_dir = TempDir::new().expect("temp dir");

        let test_files = [
            ("f_0_bin", "f_0.bin", vec![]),
            ("f_1_bin", "f_1.bin", vec![0x42]),
            ("f_65535_bin", "f_65535.bin", vec![0xAA; 65535]),
            ("f_65536_bin", "f_65536.bin", vec![0xBB; 65536]),
            (
                "f_300k_bin",
                "f_300k.bin",
                (0..307200).map(|i| (i % 256) as u8).collect(),
            ),
        ];

        for (file_key, filename, expected_bytes) in &test_files {
            let remote_path = format!("/data/{filename}");
            let local_path = tmp_dir.path().join(filename);

            let pull_out = server.run_client(&[
                "-s",
                &mock.addr,
                "pull",
                &remote_path,
                local_path.to_str().unwrap(),
            ]);
            save_golden(
                &golden_dir,
                &format!("sync_v2_pull_{file_key}"),
                &["-s", "<mock_addr>", "pull", &remote_path, "<local_path>"],
                &pull_out,
            );

            let pushed_filename = format!("pushed_{filename}");
            let local_src = tmp_dir.path().join(&pushed_filename);
            fs::write(&local_src, expected_bytes).unwrap();

            let remote_dest = format!("/data/{pushed_filename}");
            let push_out = server.run_client(&[
                "-s",
                &mock.addr,
                "push",
                local_src.to_str().unwrap(),
                &remote_dest,
            ]);
            save_golden(
                &golden_dir,
                &format!("sync_v2_push_{file_key}"),
                &["-s", "<mock_addr>", "push", "<local_src>", &remote_dest],
                &push_out,
            );
        }

        let pull_a_local = tmp_dir.path().join("pull_a.bin");
        let pull_a_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "-a",
            "/data/f_1.bin",
            pull_a_local.to_str().unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_pull_a",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "-a",
                "/data/f_1.bin",
                "<local_path>",
            ],
            &pull_a_out,
        );

        let missing_remote_local = tmp_dir.path().join("missing_remote.bin");
        let missing_pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "/data/nonexistent.bin",
            missing_remote_local.to_str().unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_pull_missing_remote",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "/data/nonexistent.bin",
                "<local_path>",
            ],
            &missing_pull_out,
        );

        let missing_local_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            tmp_dir.path().join("nosuchlocal.bin").to_str().unwrap(),
            "/data/nosuchlocal.bin",
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_push_missing_local",
            &[
                "-s",
                "<mock_addr>",
                "push",
                "<missing_local>",
                "/data/nosuchlocal.bin",
            ],
            &missing_local_out,
        );

        let bad_dst_pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "/data/f_1.bin",
            tmp_dir
                .path()
                .join("nonexistent_dir")
                .join("file.bin")
                .to_str()
                .unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_pull_bad_dst_dir",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "/data/f_1.bin",
                "<bad_dst_dir>",
            ],
            &bad_dst_pull_out,
        );

        let two_src_file = tmp_dir.path().join("single_target.bin");
        fs::write(&two_src_file, b"data").unwrap();
        let two_src_pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "/data/f_0.bin",
            "/data/f_1.bin",
            two_src_file.to_str().unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_pull_two_sources_into_file",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "/data/f_0.bin",
                "/data/f_1.bin",
                "<single_file>",
            ],
            &two_src_pull_out,
        );

        let src1 = tmp_dir.path().join("src1.bin");
        let src2 = tmp_dir.path().join("src2.bin");
        fs::write(&src1, b"1").unwrap();
        fs::write(&src2, b"2").unwrap();
        let two_src_push_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            src1.to_str().unwrap(),
            src2.to_str().unwrap(),
            "/data/f_0.bin",
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_push_two_sources_into_file",
            &[
                "-s",
                "<mock_addr>",
                "push",
                "<src1>",
                "<src2>",
                "/data/f_0.bin",
            ],
            &two_src_push_out,
        );

        let push_fail_src = tmp_dir.path().join("push_fail.bin");
        fs::write(&push_fail_src, b"fail content").unwrap();
        let push_fail_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            push_fail_src.to_str().unwrap(),
            "/nonexistent_dir/file.bin",
        ]);
        save_golden(
            &golden_dir,
            "sync_v2_push_fail_device",
            &["-s", "<mock_addr>", "push", "<src>", "/nonexistent_dir/file.bin"],
            &push_fail_out,
        );
    }

    // 5. Connection management specs
    {
        let mock = MockAdbdProcess::start();
        let server = TestServer::start_with_config(config.clone());

        let conn1 = server.run_client(&["connect", &mock.addr]);
        save_golden(
            &golden_dir,
            "mock_connect_first",
            &["connect", "<mock_addr>"],
            &conn1,
        );

        let conn2 = server.run_client(&["connect", &mock.addr]);
        save_golden(
            &golden_dir,
            "mock_connect_second",
            &["connect", "<mock_addr>"],
            &conn2,
        );

        let conn_closed = server.run_client(&["connect", "127.0.0.1:59999"]);
        save_golden(
            &golden_dir,
            "mock_connect_closed_port",
            &["connect", "127.0.0.1:59999"],
            &conn_closed,
        );

        let disconn_unknown = server.run_client(&["disconnect", "127.0.0.1:59999"]);
        save_golden(
            &golden_dir,
            "mock_disconnect_unknown",
            &["disconnect", "127.0.0.1:59999"],
            &disconn_unknown,
        );

        let disconn_serial = server.run_client(&["disconnect", &mock.addr]);
        save_golden(
            &golden_dir,
            "mock_disconnect_serial",
            &["disconnect", "<mock_addr>"],
            &disconn_serial,
        );

        let _ = server.run_client(&["connect", &mock.addr]);
        let disconn_all = server.run_client(&["disconnect"]);
        save_golden(
            &golden_dir,
            "mock_disconnect_all",
            &["disconnect"],
            &disconn_all,
        );
    }

    // 6. Shell --no-shell-v2
    {
        let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
        let server = TestServer::start_with_config(config.clone());
        let _ = server.run_client(&["connect", &mock.addr]);

        save_golden(
            &golden_dir,
            "mock_no_v2_shell_echo",
            &["-s", "<mock_addr>", "shell", "echo", "hello"],
            &server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_shell_exit_3",
            &["-s", "<mock_addr>", "shell", "exit", "3"],
            &server.run_client(&["-s", &mock.addr, "shell", "exit", "3"]),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_shell_stderr",
            &["-s", "<mock_addr>", "shell", "stderr", "oops"],
            &server.run_client(&["-s", &mock.addr, "shell", "stderr", "oops"]),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_shell_no_pty_echo",
            &["-s", "<mock_addr>", "shell", "-T", "echo", "hi"],
            &server.run_client(&["-s", &mock.addr, "shell", "-T", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_shell_no_v2_flag_echo",
            &["-s", "<mock_addr>", "shell", "-x", "echo", "hi"],
            &server.run_client(&["-s", &mock.addr, "shell", "-x", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_exec_out_echo",
            &["-s", "<mock_addr>", "exec-out", "echo", "hi"],
            &server.run_client(&["-s", &mock.addr, "exec-out", "echo", "hi"]),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_exec_in_cat",
            &["-s", "<mock_addr>", "exec-in", "cat"],
            &run_client_with_stdin(
                &server,
                &["-s", &mock.addr, "exec-in", "cat"],
                b"exec-in payload\n",
            ),
        );
        save_golden(
            &golden_dir,
            "mock_no_v2_shell_unsupported",
            &["-s", "<mock_addr>", "shell", "unsupported_cmd"],
            &server.run_client(&["-s", &mock.addr, "shell", "unsupported_cmd"]),
        );
    }

    // 7. Sync v1 fallback
    {
        let mock = MockAdbdProcess::start_with_args(&["--no-ls-v2", "--no-stat-v2"]);
        let server = TestServer::start_with_config(config.clone());
        let _ = server.run_client(&["connect", &mock.addr]);

        save_golden(
            &golden_dir,
            "mock_sync_ls_root_v1",
            &["-s", "<mock_addr>", "ls", "/"],
            &server.run_client(&["-s", &mock.addr, "ls", "/"]),
        );
        save_golden(
            &golden_dir,
            "mock_sync_ls_data_v1",
            &["-s", "<mock_addr>", "ls", "/data"],
            &server.run_client(&["-s", &mock.addr, "ls", "/data"]),
        );

        let tmp_dir = TempDir::new().expect("temp dir");

        let test_files = [
            ("f_0_bin", "f_0.bin", vec![]),
            ("f_1_bin", "f_1.bin", vec![0x42]),
            ("f_65535_bin", "f_65535.bin", vec![0xAA; 65535]),
            ("f_65536_bin", "f_65536.bin", vec![0xBB; 65536]),
            (
                "f_300k_bin",
                "f_300k.bin",
                (0..307200).map(|i| (i % 256) as u8).collect(),
            ),
        ];

        for (file_key, filename, expected_bytes) in &test_files {
            let remote_path = format!("/data/{filename}");
            let local_path = tmp_dir.path().join(filename);

            let pull_out = server.run_client(&[
                "-s",
                &mock.addr,
                "pull",
                &remote_path,
                local_path.to_str().unwrap(),
            ]);
            save_golden(
                &golden_dir,
                &format!("sync_v1_pull_{file_key}"),
                &["-s", "<mock_addr>", "pull", &remote_path, "<local_path>"],
                &pull_out,
            );

            let pushed_filename = format!("pushed_{filename}");
            let local_src = tmp_dir.path().join(&pushed_filename);
            fs::write(&local_src, expected_bytes).unwrap();

            let remote_dest = format!("/data/{pushed_filename}");
            let push_out = server.run_client(&[
                "-s",
                &mock.addr,
                "push",
                local_src.to_str().unwrap(),
                &remote_dest,
            ]);
            save_golden(
                &golden_dir,
                &format!("sync_v1_push_{file_key}"),
                &["-s", "<mock_addr>", "push", "<local_src>", &remote_dest],
                &push_out,
            );
        }

        let pull_a_local = tmp_dir.path().join("pull_a.bin");
        let pull_a_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "-a",
            "/data/f_1.bin",
            pull_a_local.to_str().unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_pull_a",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "-a",
                "/data/f_1.bin",
                "<local_path>",
            ],
            &pull_a_out,
        );

        let missing_remote_local = tmp_dir.path().join("missing_remote.bin");
        let missing_pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "/data/nonexistent.bin",
            missing_remote_local.to_str().unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_pull_missing_remote",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "/data/nonexistent.bin",
                "<local_path>",
            ],
            &missing_pull_out,
        );

        let missing_local_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            tmp_dir.path().join("nosuchlocal.bin").to_str().unwrap(),
            "/data/nosuchlocal.bin",
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_push_missing_local",
            &[
                "-s",
                "<mock_addr>",
                "push",
                "<missing_local>",
                "/data/nosuchlocal.bin",
            ],
            &missing_local_out,
        );

        let bad_dst_pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "/data/f_1.bin",
            tmp_dir
                .path()
                .join("nonexistent_dir")
                .join("file.bin")
                .to_str()
                .unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_pull_bad_dst_dir",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "/data/f_1.bin",
                "<bad_dst_dir>",
            ],
            &bad_dst_pull_out,
        );

        let two_src_file = tmp_dir.path().join("single_target.bin");
        fs::write(&two_src_file, b"data").unwrap();
        let two_src_pull_out = server.run_client(&[
            "-s",
            &mock.addr,
            "pull",
            "/data/f_0.bin",
            "/data/f_1.bin",
            two_src_file.to_str().unwrap(),
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_pull_two_sources_into_file",
            &[
                "-s",
                "<mock_addr>",
                "pull",
                "/data/f_0.bin",
                "/data/f_1.bin",
                "<single_file>",
            ],
            &two_src_pull_out,
        );

        let src1 = tmp_dir.path().join("src1.bin");
        let src2 = tmp_dir.path().join("src2.bin");
        fs::write(&src1, b"1").unwrap();
        fs::write(&src2, b"2").unwrap();
        let two_src_push_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            src1.to_str().unwrap(),
            src2.to_str().unwrap(),
            "/data/f_0.bin",
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_push_two_sources_into_file",
            &[
                "-s",
                "<mock_addr>",
                "push",
                "<src1>",
                "<src2>",
                "/data/f_0.bin",
            ],
            &two_src_push_out,
        );

        let push_fail_src = tmp_dir.path().join("push_fail.bin");
        fs::write(&push_fail_src, b"fail content").unwrap();
        let push_fail_out = server.run_client(&[
            "-s",
            &mock.addr,
            "push",
            push_fail_src.to_str().unwrap(),
            "/nonexistent_dir/file.bin",
        ]);
        save_golden(
            &golden_dir,
            "sync_v1_push_fail_device",
            &["-s", "<mock_addr>", "push", "<src>", "/nonexistent_dir/file.bin"],
            &push_fail_out,
        );
    }

    // 8. Special failure cases
    {
        let mock1 = MockAdbdProcess::start();
        let mock2 = MockAdbdProcess::start();
        let server = TestServer::start_with_config(config.clone());
        let _ = server.run_client(&["connect", &mock1.addr]);
        let _ = server.run_client(&["connect", &mock2.addr]);

        save_golden(
            &golden_dir,
            "mock_two_devices_no_selector",
            &["get-state"],
            &server.run_client(&["get-state"]),
        );
    }

    {
        let mut mock = MockAdbdProcess::start();
        let server = TestServer::start_with_config(config.clone());
        let mock_addr = mock.addr.clone();
        let _ = server.run_client(&["connect", &mock_addr]);

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = mock.child.kill();
        });

        save_golden(
            &golden_dir,
            "mock_shell_closed_connection",
            &["-s", "<mock_addr>", "shell", "cat"],
            &server.run_client(&["-s", &mock_addr, "shell", "cat"]),
        );
    }

    println!("All golden files captured successfully.");
}
