use compat::{normalize_output, Config, GoldenCase, TestOutput, TestServer};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

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

fn run_client_with_env(
    server: &TestServer,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> TestOutput {
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
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().expect("Failed to run adb client");
    TestOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        status: output.status,
    }
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

#[test]
fn test_device_list_and_properties() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn = server.run_client(&["connect", &mock.addr]);
    assert!(conn.success(), "failed to connect to mock adbd");

    let devices = server.run_client(&["devices"]);
    assert_matches_golden("mock_devices", &devices);

    let devices_l = server.run_client(&["devices", "-l"]);
    assert_matches_golden("mock_devices_l", &devices_l);

    let get_state = server.run_client(&["get-state"]);
    assert_matches_golden("mock_get_state", &get_state);

    let get_serialno = server.run_client(&["get-serialno"]);
    assert_matches_golden("mock_get_serialno", &get_serialno);

    let get_devpath = server.run_client(&["get-devpath"]);
    assert_matches_golden("mock_get_devpath", &get_devpath);

    let transport_id = server.run_client(&["transport-id"]);
    assert_matches_golden("mock_transport_id", &transport_id);

    let features = server.run_client(&["features"]);
    assert_matches_golden("mock_features", &features);

    let host_features = server.run_client(&["host-features"]);
    assert_matches_golden("mock_host_features", &host_features);

    let s_variant = server.run_client(&["-s", &mock.addr, "get-state"]);
    assert_matches_golden("mock_s_get_state", &s_variant);

    let t_variant = server.run_client(&["-t", "1", "get-state"]);
    assert_matches_golden("mock_t_get_state", &t_variant);

    let d_variant = server.run_client(&["-d", "get-state"]);
    assert_matches_golden("mock_d_get_state", &d_variant);

    let e_variant = server.run_client(&["-e", "get-state"]);
    assert_matches_golden("mock_e_get_state", &e_variant);

    let env_serial_variant =
        run_client_with_env(&server, &["get-state"], &[("ANDROID_SERIAL", &mock.addr)]);
    assert_matches_golden("mock_s_get_state", &env_serial_variant);
}

#[test]
fn test_connection_management() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn1 = server.run_client(&["connect", &mock.addr]);
    assert_matches_golden("mock_connect_first", &conn1);

    let conn2 = server.run_client(&["connect", &mock.addr]);
    assert_matches_golden("mock_connect_second", &conn2);

    let conn_closed = server.run_client(&["connect", "127.0.0.1:59999"]);
    assert_matches_golden("mock_connect_closed_port", &conn_closed);

    let disconn_unknown = server.run_client(&["disconnect", "127.0.0.1:59999"]);
    assert_matches_golden("mock_disconnect_unknown", &disconn_unknown);

    let disconn_serial = server.run_client(&["disconnect", &mock.addr]);
    assert_matches_golden("mock_disconnect_serial", &disconn_serial);

    // Reconnect to test disconnect (no arg)
    let _ = server.run_client(&["connect", &mock.addr]);
    let disconn_all = server.run_client(&["disconnect"]);
    assert_matches_golden("mock_disconnect_all", &disconn_all);
}

#[test]
fn test_shell_v2() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let _ = server.run_client(&["connect", &mock.addr]);

    let echo = server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]);
    assert_matches_golden("mock_shell_echo", &echo);

    let exit_3 = server.run_client(&["-s", &mock.addr, "shell", "exit", "3"]);
    assert_matches_golden("mock_shell_exit_3", &exit_3);

    let stderr_cmd = server.run_client(&["-s", &mock.addr, "shell", "stderr", "oops"]);
    assert_matches_golden("mock_shell_stderr", &stderr_cmd);

    let no_pty = server.run_client(&["-s", &mock.addr, "shell", "-T", "echo", "hi"]);
    assert_matches_golden("mock_shell_no_pty_echo", &no_pty);

    let no_v2_flag = server.run_client(&["-s", &mock.addr, "shell", "-x", "echo", "hi"]);
    assert_matches_golden("mock_shell_no_v2_flag_echo", &no_v2_flag);

    let n_cat = server.run_client(&["-s", &mock.addr, "shell", "-n", "cat"]);
    assert_matches_golden("mock_shell_n_cat", &n_cat);

    let piped_cat = run_client_with_stdin(
        &server,
        &["-s", &mock.addr, "shell", "cat"],
        b"hello from stdin\n",
    );
    assert_matches_golden("mock_shell_piped_cat", &piped_cat);

    let exec_out = server.run_client(&["-s", &mock.addr, "exec-out", "echo", "hi"]);
    assert_matches_golden("mock_exec_out_echo", &exec_out);

    let exec_in = run_client_with_stdin(
        &server,
        &["-s", &mock.addr, "exec-in", "cat"],
        b"exec-in payload\n",
    );
    assert_matches_golden("mock_exec_in_cat", &exec_in);

    let unsupported = server.run_client(&["-s", &mock.addr, "shell", "unsupported_cmd"]);
    assert_matches_golden("mock_shell_unsupported", &unsupported);
}

#[test]
fn test_no_v2_shell_echo() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let echo = server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]);
    assert_matches_golden("mock_no_v2_shell_echo", &echo);
}

#[test]
fn test_no_v2_shell_exit_3() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let exit_3 = server.run_client(&["-s", &mock.addr, "shell", "exit", "3"]);
    assert_matches_golden("mock_no_v2_shell_exit_3", &exit_3);
}

#[test]
fn test_no_v2_shell_stderr() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let stderr_cmd = server.run_client(&["-s", &mock.addr, "shell", "stderr", "oops"]);
    assert_matches_golden("mock_no_v2_shell_stderr", &stderr_cmd);
}

#[test]
fn test_no_v2_shell_no_pty() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let no_pty = server.run_client(&["-s", &mock.addr, "shell", "-T", "echo", "hi"]);
    assert_matches_golden("mock_no_v2_shell_no_pty_echo", &no_pty);
}

#[test]
fn test_no_v2_shell_no_v2_flag() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let no_v2_flag = server.run_client(&["-s", &mock.addr, "shell", "-x", "echo", "hi"]);
    assert_matches_golden("mock_no_v2_shell_no_v2_flag_echo", &no_v2_flag);
}

#[test]
fn test_no_v2_shell_exec_out() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let exec_out = server.run_client(&["-s", &mock.addr, "exec-out", "echo", "hi"]);
    assert_matches_golden("mock_no_v2_exec_out_echo", &exec_out);
}

#[test]
fn test_no_v2_shell_exec_in() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let exec_in = run_client_with_stdin(
        &server,
        &["-s", &mock.addr, "exec-in", "cat"],
        b"exec-in payload\n",
    );
    assert_matches_golden("mock_no_v2_exec_in_cat", &exec_in);
}

#[test]
fn test_no_v2_shell_unsupported() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock.addr]);
    let unsupported = server.run_client(&["-s", &mock.addr, "shell", "unsupported_cmd"]);
    assert_matches_golden("mock_no_v2_shell_unsupported", &unsupported);
}

#[test]
fn test_forward() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let _ = server.run_client(&["connect", &mock.addr]);

    let fwd_list_empty = server.run_client(&["-s", &mock.addr, "forward", "--list"]);
    assert_matches_golden("mock_forward_list_empty", &fwd_list_empty);

    let fwd_add = server.run_client(&["-s", &mock.addr, "forward", "tcp:0", "tcp:1234"]);
    assert_matches_golden("mock_forward_add_zero", &fwd_add);

    let alloc_port = fwd_add.stdout.trim().to_string();

    let fwd_list_one = server.run_client(&["-s", &mock.addr, "forward", "--list"]);
    assert_matches_golden("mock_forward_list_one", &fwd_list_one);

    let fwd_no_rebind = server.run_client(&[
        "-s",
        &mock.addr,
        "forward",
        "--no-rebind",
        &format!("tcp:{alloc_port}"),
        "tcp:1234",
    ]);
    assert_matches_golden("mock_forward_no_rebind_existing", &fwd_no_rebind);

    let fwd_remove = server.run_client(&[
        "-s",
        &mock.addr,
        "forward",
        "--remove",
        &format!("tcp:{alloc_port}"),
    ]);
    assert_matches_golden("mock_forward_remove", &fwd_remove);

    let _ = server.run_client(&["-s", &mock.addr, "forward", "tcp:0", "tcp:1234"]);
    let fwd_remove_all = server.run_client(&["-s", &mock.addr, "forward", "--remove-all"]);
    assert_matches_golden("mock_forward_remove_all", &fwd_remove_all);
}

#[test]
fn test_sync() {
    let mock_v2 = MockAdbdProcess::start();
    let server1 = TestServer::start();
    let _ = server1.run_client(&["connect", &mock_v2.addr]);

    let ls_root_v2 = server1.run_client(&["-s", &mock_v2.addr, "ls", "/"]);
    assert_matches_golden("mock_sync_ls_root_v2", &ls_root_v2);

    let ls_data_v2 = server1.run_client(&["-s", &mock_v2.addr, "ls", "/data"]);
    assert_matches_golden("mock_sync_ls_data_v2", &ls_data_v2);

    let mock_v1 = MockAdbdProcess::start_with_args(&["--no-ls-v2", "--no-stat-v2"]);
    let server2 = TestServer::start();
    let _ = server2.run_client(&["connect", &mock_v1.addr]);

    let ls_root_v1 = server2.run_client(&["-s", &mock_v1.addr, "ls", "/"]);
    assert_matches_golden("mock_sync_ls_root_v1", &ls_root_v1);

    let ls_data_v1 = server2.run_client(&["-s", &mock_v1.addr, "ls", "/data"]);
    assert_matches_golden("mock_sync_ls_data_v1", &ls_data_v1);
}

#[test]
fn test_failure_cases() {
    // 1. Two devices connected and no selector
    {
        let mock1 = MockAdbdProcess::start();
        let mock2 = MockAdbdProcess::start();
        let server = TestServer::start();

        let _ = server.run_client(&["connect", &mock1.addr]);
        let _ = server.run_client(&["connect", &mock2.addr]);

        let two_devs = server.run_client(&["get-state"]);
        assert_matches_golden("mock_two_devices_no_selector", &two_devs);
    }

    // 2. Offline device: mock-adbd handles CNXN handshake upon TCP connection and does not simulate an offline state.
    // Therefore, offline device testing with mock-adbd is skipped as noted in task scope.

    // 3. Device that closes connection in middle of shell
    {
        let mut mock = MockAdbdProcess::start();
        let server = TestServer::start();
        let mock_addr = mock.addr.clone();

        let _ = server.run_client(&["connect", &mock_addr]);

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = mock.child.kill();
        });

        let closed_shell = server.run_client(&["-s", &mock_addr, "shell", "cat"]);
        assert_matches_golden("mock_shell_closed_connection", &closed_shell);
    }
}
