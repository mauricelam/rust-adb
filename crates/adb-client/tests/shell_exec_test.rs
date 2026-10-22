use adb_client::{
    escape_arg, exec_in, exec_out, shell, ClientError, DefaultSessionFactory, ServerAddress,
    ShellOptions, TransportSelector,
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
    assert!(
        bin_path.exists(),
        "mock-adbd binary not found at {:?}",
        bin_path
    );
    bin_path
}

struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start(no_shell_v2: bool) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen").arg("127.0.0.1:0");
        if no_shell_v2 {
            cmd.arg("--no-shell-v2");
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

struct TestFixture {
    _mock: MockAdbdProcess,
    _server: TestServer,
    pub addr: ServerAddress,
    pub selector: TransportSelector,
}

fn setup_fixture(no_shell_v2: bool) -> TestFixture {
    let mock = MockAdbdProcess::start(no_shell_v2);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "adb connect failed: stdout: {}, stderr: {}",
        connect_out.stdout,
        connect_out.stderr
    );

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };
    let selector = TransportSelector::Serial(mock.addr.clone());

    TestFixture {
        _mock: mock,
        _server: server,
        addr,
        selector,
    }
}

#[test]
fn test_escape_arg_parity() {
    // Ported from original/adb_utils_test.cpp
    assert_eq!("''", escape_arg(""));
    assert_eq!("'abc'", escape_arg("abc"));
    assert_eq!("''\\'''", escape_arg("'"));
    assert_eq!("''\\'''\\'''", escape_arg("''"));
    assert_eq!("''\\''abc'\\'''", escape_arg("'abc'"));
    assert_eq!("''\\''abc'", escape_arg("'abc"));
    assert_eq!("'abc'\\'''", escape_arg("abc'"));
    assert_eq!("'abc'\\''def'", escape_arg("abc'def"));
    assert_eq!("'a'\\''b'\\''c'", escape_arg("a'b'c"));
    assert_eq!("'a'\\''bcde'\\''f'", escape_arg("a'bcde'f"));
    assert_eq!("' abc'", escape_arg(" abc"));
    assert_eq!("'\"abc'", escape_arg("\"abc"));
    assert_eq!("'\\abc'", escape_arg("\\abc"));
    assert_eq!("'(abc'", escape_arg("(abc"));
    assert_eq!("')abc'", escape_arg(")abc"));
    assert_eq!("'abc abc'", escape_arg("abc abc"));
    assert_eq!("'abc\"abc'", escape_arg("abc\"abc"));
    assert_eq!("'abc\\abc'", escape_arg("abc\\abc"));
    assert_eq!("'abc(abc'", escape_arg("abc(abc"));
    assert_eq!("'abc)abc'", escape_arg("abc)abc"));
    assert_eq!("'abc '", escape_arg("abc "));
    assert_eq!("'abc\"'", escape_arg("abc\""));
    assert_eq!("'abc\\'", escape_arg("abc\\"));
    assert_eq!("'abc('", escape_arg("abc("));
    assert_eq!("'abc)'", escape_arg("abc)"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_echo_hello() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let exit_code = shell(
        &fix.addr,
        &fix.selector,
        &["echo", "hello"],
        &ShellOptions::default(),
        Some("xterm-256color"),
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "hello\n");
    assert_eq!(String::from_utf8_lossy(&stderr), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_nonzero_exit() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let exit_code = shell(
        &fix.addr,
        &fix.selector,
        &["exit", "42"],
        &ShellOptions::default(),
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(exit_code, 42);
    assert_eq!(String::from_utf8_lossy(&stdout), "");
    assert_eq!(String::from_utf8_lossy(&stderr), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_stderr() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let exit_code = shell(
        &fix.addr,
        &fix.selector,
        &["stderr", "oops error"],
        &ShellOptions::default(),
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "");
    assert_eq!(String::from_utf8_lossy(&stderr), "oops error\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_close_stdin_flag() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let stdin_data = Cursor::new(b"should be ignored".to_vec());
    let options = ShellOptions {
        close_stdin: true,
        ..Default::default()
    };

    let exit_code = shell(
        &fix.addr,
        &fix.selector,
        &["cat"],
        &options,
        None,
        Some(stdin_data),
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "");
    assert_eq!(String::from_utf8_lossy(&stderr), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_force_raw_flag() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let options = ShellOptions {
        force_raw: true,
        ..Default::default()
    };

    let exit_code = shell(
        &fix.addr,
        &fix.selector,
        &["echo", "raw mode"],
        &options,
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "raw mode\n");
    assert_eq!(String::from_utf8_lossy(&stderr), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_disable_shell_protocol_refusal() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let options = ShellOptions {
        disable_shell_protocol: true,
        ..Default::default()
    };

    let err = shell(
        &fix.addr,
        &fix.selector,
        &["echo", "hello"],
        &options,
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .expect_err("should refuse raw request when shell v2 is disabled via -x");

    if let ClientError::InvalidUsage(msg) = err {
        assert_eq!(msg, "error: -x only supports allocating a pty");
    } else {
        panic!("expected ClientError::InvalidUsage, got {err:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_no_shell2_feature_refusal() {
    let fix = setup_fixture(true); // no shell_v2
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let err = shell(
        &fix.addr,
        &fix.selector,
        &["echo", "hello"],
        &ShellOptions::default(),
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .expect_err("should refuse raw request on device without shell_v2");

    if let ClientError::InvalidUsage(msg) = err {
        assert_eq!(msg, "error: device only supports allocating a pty");
    } else {
        panic!("expected ClientError::InvalidUsage, got {err:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_command_too_long() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let options = ShellOptions {
        disable_shell_protocol: true,
        ..Default::default()
    };

    let long_arg = "a".repeat(5000);
    let err = shell(
        &fix.addr,
        &fix.selector,
        &["echo", &long_arg],
        &options,
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .expect_err("should refuse over-long command");

    if let ClientError::InvalidUsage(msg) = err {
        assert_eq!(msg, "error: shell command too long");
    } else {
        panic!("expected ClientError::InvalidUsage, got {err:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_close_no_exit() {
    let fix = setup_fixture(false);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let factory = DefaultSessionFactory;

    let exit_code = shell(
        &fix.addr,
        &fix.selector,
        &["close-no-exit"],
        &ShellOptions::default(),
        None,
        None::<Cursor<Vec<u8>>>,
        &mut stdout,
        &mut stderr,
        &factory,
    )
    .await
    .unwrap();

    // Protocol closed before Exit packet -> 255
    assert_eq!(exit_code, 255);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_in_and_exec_out() {
    let fix = setup_fixture(false);
    let factory = DefaultSessionFactory;

    // 1. exec_in cat
    let stdin_data = Cursor::new(b"exec-in test data".to_vec());
    let code_in = exec_in(
        &fix.addr,
        &TransportSelector::Any,
        &["cat"],
        stdin_data,
        &factory,
    )
    .await
    .unwrap();
    assert_eq!(code_in, 0);

    // 2. exec_out echo
    let mut stdout_out = Vec::new();
    let code_out = exec_out(
        &fix.addr,
        &fix.selector,
        &["echo", "hello exec-out"],
        &mut stdout_out,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(code_out, 0);
    assert_eq!(String::from_utf8_lossy(&stdout_out), "hello exec-out\n");

    // 3. exec_out with argument escaping
    let mut stdout_escaped = Vec::new();
    let code_escaped = exec_out(
        &fix.addr,
        &fix.selector,
        &["echo", "foo bar", "it's working"],
        &mut stdout_escaped,
        &factory,
    )
    .await
    .unwrap();

    assert_eq!(code_escaped, 0);
    assert_eq!(
        String::from_utf8_lossy(&stdout_escaped),
        "foo bar it's working\n"
    );
}
