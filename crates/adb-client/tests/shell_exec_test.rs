use adb_client::{
    escape_arg, AdbClient, ClientError, ServerAddress, ShellOptions, TransportSelector,
};
use compat::TestServer;
use std::env;
use std::io::{BufRead, BufReader, Cursor};
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

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[test]
fn test_escape_arg_unit() {
    assert_eq!(escape_arg(""), "''");
    assert_eq!(escape_arg("abc"), "'abc'");
    assert_eq!(escape_arg("'"), "''\\'''");
    assert_eq!(escape_arg("''"), "''\\'''\\'''");
    assert_eq!(escape_arg("'abc'"), "''\\''abc'\\'''");
    assert_eq!(escape_arg(" abc"), "' abc'");
    assert_eq!(escape_arg("\"abc"), "'\"abc'");
    assert_eq!(escape_arg("\\abc"), "'\\abc'");
    assert_eq!(escape_arg("(abc)"), "'(abc)'");
    assert_eq!(escape_arg("foo bar"), "'foo bar'");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_echo_hello_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = client
        .shell(
            &selector,
            &["echo", "hello"],
            &ShellOptions::default(),
            None::<&[u8]>,
            Some(&mut stdout),
            Some(&mut stderr),
        )
        .await;

    let rust_status = outcome.result.unwrap();
    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]);

    assert_eq!(rust_status, ref_out.exit_code.unwrap_or(-1));
    assert_eq!(
        norm(&String::from_utf8_lossy(&stdout)),
        norm(&ref_out.stdout)
    );
    assert_eq!(
        norm(&String::from_utf8_lossy(&stderr)),
        norm(&ref_out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_exit_code_v2_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = client
        .shell(
            &selector,
            &["exit", "3"],
            &ShellOptions::default(),
            None::<&[u8]>,
            Some(&mut stdout),
            Some(&mut stderr),
        )
        .await;

    let rust_status = outcome.result.unwrap();
    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "exit", "3"]);

    assert_eq!(rust_status, 3);
    assert_eq!(rust_status, ref_out.exit_code.unwrap_or(-1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_stderr_v2_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let outcome = client
        .shell(
            &selector,
            &["stderr", "oops"],
            &ShellOptions::default(),
            None::<&[u8]>,
            Some(&mut stdout),
            Some(&mut stderr),
        )
        .await;

    let rust_status = outcome.result.unwrap();
    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "stderr", "oops"]);

    assert_eq!(rust_status, 0);
    assert_eq!(rust_status, ref_out.exit_code.unwrap_or(-1));
    assert_eq!(
        norm(&String::from_utf8_lossy(&stdout)),
        norm(&ref_out.stdout)
    );
    assert_eq!(
        norm(&String::from_utf8_lossy(&stderr)),
        norm(&ref_out.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_piped_stdin_cat_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let stdin_data = b"piped stdin content\n";
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = client
        .shell(
            &selector,
            &["cat"],
            &ShellOptions::default(),
            Some(Cursor::new(stdin_data.to_vec())),
            Some(&mut stdout),
            Some(&mut stderr),
        )
        .await;

    let rust_status = outcome.result.unwrap();
    assert_eq!(rust_status, 0);
    assert_eq!(
        norm(&String::from_utf8_lossy(&stdout)),
        "piped stdin content\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_no_stdin_option() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let opts = ShellOptions {
        no_stdin: true,
        ..Default::default()
    };

    let outcome = client
        .shell(
            &selector,
            &["echo", "hello"],
            &opts,
            Some(Cursor::new(b"ignored input")),
            Some(&mut stdout),
            Some(&mut stderr),
        )
        .await;

    assert_eq!(outcome.result.unwrap(), 0);
    assert_eq!(norm(&String::from_utf8_lossy(&stdout)), "hello\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_flag_x_refusal_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let opts = ShellOptions {
        disable_shell_protocol: true,
        no_pty: true,
        ..Default::default()
    };

    let outcome = client
        .shell(
            &selector,
            &["echo", "hello"],
            &opts,
            None::<&[u8]>,
            None::<&mut Vec<u8>>,
            None::<&mut Vec<u8>>,
        )
        .await;

    let err = outcome.result.unwrap_err();
    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "-x", "-T", "echo", "hello"]);

    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "error: -x only supports allocating a pty")
    );
    assert_eq!(norm(err.to_string().trim()), norm(ref_out.stderr.trim()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_device_without_shell_v2_refusal_parity() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let opts = ShellOptions {
        no_pty: true,
        ..Default::default()
    };

    let outcome = client
        .shell(
            &selector,
            &["echo", "hello"],
            &opts,
            None::<&[u8]>,
            None::<&mut Vec<u8>>,
            None::<&mut Vec<u8>>,
        )
        .await;

    let err = outcome.result.unwrap_err();
    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "-T", "echo", "hello"]);

    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "error: device only supports allocating a pty")
    );
    assert_eq!(norm(err.to_string().trim()), norm(ref_out.stderr.trim()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_command_too_long_refusal() {
    let mock = MockAdbdProcess::start_with_args(&["--no-shell-v2"]);
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let long_cmd = "a".repeat(5000);
    let opts = ShellOptions {
        disable_shell_protocol: true,
        ..Default::default()
    };

    let outcome = client
        .shell(
            &selector,
            &[&long_cmd],
            &opts,
            None::<&[u8]>,
            None::<&mut Vec<u8>>,
            None::<&mut Vec<u8>>,
        )
        .await;

    let err = outcome.result.unwrap_err();
    assert!(
        matches!(&err, ClientError::InvalidUsage(msg) if msg == "error: -x only supports allocating a pty" || msg == "error: shell command too long")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_connection_closed_no_exit_packet() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let outcome = client
        .shell(
            &selector,
            &["no-exit-packet"],
            &ShellOptions::default(),
            None::<&[u8]>,
            Some(&mut stdout),
            Some(&mut stderr),
        )
        .await;

    let status = outcome.result.unwrap();
    assert_eq!(status, 255);
    assert_eq!(norm(&String::from_utf8_lossy(&stdout)), "abrupt end\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_out_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut stdout = Vec::new();
    let exec_out_res = client
        .exec_out(&selector, &["echo", "exec hello"], &mut stdout)
        .await;

    assert_eq!(exec_out_res.result.unwrap(), 0);
    let ref_out = server.run_client(&["-s", &mock.addr, "exec-out", "echo", "exec hello"]);
    assert_eq!(
        norm(&String::from_utf8_lossy(&stdout)),
        norm(&ref_out.stdout)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_in_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn_res = server.run_client(&["connect", &mock.addr]);
    assert!(conn_res.success());

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let stdin_data = Cursor::new(b"exec stdin data\n");
    let exec_in_res = client.exec_in(&selector, &["cat"], stdin_data).await;
    assert_eq!(exec_in_res.result.unwrap(), 0);
}
