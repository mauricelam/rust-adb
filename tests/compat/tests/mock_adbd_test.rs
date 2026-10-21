use compat::{Config, TestServer};
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

fn get_mock_adbd_bin() -> PathBuf {
    if let Ok(bin) = env::var("CARGO_BIN_EXE_mock-adbd") {
        return PathBuf::from(bin);
    }
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

struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start() -> Self {
        Self::start_with_args(&[])
    }

    fn start_with_args(extra_args: &[&str]) -> Self {
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

#[test]
fn test_mock_adbd_connect_and_shell() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    // Run `adb connect <addr>`
    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "adb connect failed: stdout: {}, stderr: {}",
        connect_out.stdout,
        connect_out.stderr
    );

    // Run `adb devices`
    let devices_out = server.run_client(&["devices"]);
    assert!(
        devices_out.success(),
        "adb devices failed: stdout: {}, stderr: {}",
        devices_out.stdout,
        devices_out.stderr
    );
    assert!(
        devices_out
            .stdout
            .contains(&format!("{}\tdevice", mock.addr)),
        "adb devices output does not contain expected device state: stdout: {}",
        devices_out.stdout
    );

    // Run `adb -s <addr> shell echo hello`
    let shell_out = server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]);
    assert_eq!(
        shell_out.exit_code,
        Some(0),
        "adb shell echo hello failed: stdout: {}, stderr: {}",
        shell_out.stdout,
        shell_out.stderr
    );
    assert_eq!(
        shell_out.stdout.trim(),
        "hello",
        "Unexpected shell output: stdout: {}, stderr: {}",
        shell_out.stdout,
        shell_out.stderr
    );
}

#[test]
fn test_mock_adbd_shell_v2_commands() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "adb connect failed: stdout: {}, stderr: {}",
        connect_out.stdout,
        connect_out.stderr
    );

    // 1. adb shell exit 3
    let exit_out = server.run_client(&["-s", &mock.addr, "shell", "exit", "3"]);
    assert_eq!(
        exit_out.exit_code,
        Some(3),
        "adb shell exit 3 expected status 3, got: {:?}",
        exit_out.exit_code
    );

    // 2. adb shell stderr oops
    let stderr_out = server.run_client(&["-s", &mock.addr, "shell", "stderr", "oops"]);
    assert_eq!(
        stderr_out.exit_code,
        Some(0),
        "adb shell stderr oops failed: stdout: {}, stderr: {}",
        stderr_out.stdout,
        stderr_out.stderr
    );
    assert_eq!(
        stderr_out.stderr.trim(),
        "oops",
        "Unexpected stderr output: stdout: {}, stderr: {}",
        stderr_out.stdout,
        stderr_out.stderr
    );

    // 3. adb shell cat (piped stdin)
    let config = Config::from_env();
    let mut cat_cmd = Command::new(&config.client_path);
    cat_cmd
        .arg("-P")
        .arg(server.port.to_string())
        .args(["-s", &mock.addr, "shell", "cat"])
        .env("HOME", server.home_dir.path())
        .env("USERPROFILE", server.home_dir.path())
        .env("ANDROID_ADB_HOME", server.home_dir.path())
        .env("ANDROID_USER_HOME", server.home_dir.path())
        .env("ADB_MDNS_AUTO_CONNECT", "0")
        .env("ADB_MDNS_OPENSCREEN", "0")
        .env(
            "ADB_SERVER_SOCKET",
            format!("tcp:localhost:{}", server.port),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut cat_child = cat_cmd.spawn().expect("Failed to spawn adb shell cat");
    if let Some(mut stdin) = cat_child.stdin.take() {
        stdin.write_all(b"piped data\n").unwrap();
    }
    let cat_output = cat_child.wait_with_output().unwrap();
    assert!(cat_output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&cat_output.stdout).trim(),
        "piped data"
    );
}

#[test]
fn test_mock_adbd_no_shell_v2_fallback() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "adb connect failed: stdout: {}, stderr: {}",
        connect_out.stdout,
        connect_out.stderr
    );

    let shell_out = server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]);
    assert_eq!(
        shell_out.exit_code,
        Some(0),
        "adb shell echo hello failed: stdout: {}, stderr: {}",
        shell_out.stdout,
        shell_out.stderr
    );
    assert_eq!(
        shell_out.stdout.trim(),
        "hello",
        "Unexpected shell output: stdout: {}, stderr: {}",
        shell_out.stdout,
        shell_out.stderr
    );
}
