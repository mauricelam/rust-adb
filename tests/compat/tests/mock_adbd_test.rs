use compat::TestServer;
use std::env;
use std::io::{BufRead, BufReader};
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
        let bin = get_mock_adbd_bin();
        let mut child = Command::new(&bin)
            .arg("--listen")
            .arg("127.0.0.1:0")
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
