use adb_client::{
    escape_arg, AdbClient, DefaultSessionFactory, LaunchError, ServerAddress, ServerLauncher,
    ShellOptions, TransportSelector,
};
use compat::TestServer;
use std::env;
use std::future::Future;
use std::io::{BufRead, BufReader, Cursor};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};

fn wrap(x: &str) -> String {
    format!("'{x}'")
}

#[test]
fn test_escape_arg() {
    assert_eq!("''", escape_arg(""));
    assert_eq!("'abc'", escape_arg("abc"));

    let q = "'\\''";
    assert_eq!(wrap(q), escape_arg("'"));
    assert_eq!(wrap(&format!("{q}{q}")), escape_arg("''"));
    assert_eq!(wrap(&format!("{q}abc{q}")), escape_arg("'abc'"));
    assert_eq!(wrap(&format!("{q}abc")), escape_arg("'abc"));
    assert_eq!(wrap(&format!("abc{q}")), escape_arg("abc'"));
    assert_eq!(wrap(&format!("abc{q}def")), escape_arg("abc'def"));
    assert_eq!(wrap(&format!("a{q}b{q}c")), escape_arg("a'b'c"));
    assert_eq!(wrap(&format!("a{q}bcde{q}f")), escape_arg("a'bcde'f"));

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
    fn start(extra_args: &[&str]) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen")
            .arg("127.0.0.1:0")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for arg in extra_args {
            cmd.arg(arg);
        }

        let mut child = cmd
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

struct DummyLauncher;

impl ServerLauncher for DummyLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a str,
        _one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move {
            Err(LaunchError::Unsupported(
                "DummyLauncher cannot start daemon".to_string(),
            ))
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_echo_hello_parity() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success(), "adb connect failed");

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();

    let exit_code = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["echo", "hello"],
            &ShellOptions::default(),
            Option::<Cursor<Vec<u8>>>::None,
            Some(&mut stdout_buf),
            Some(&mut stderr_buf),
        )
        .await
        .result
        .expect("shell echo failed");

    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "echo", "hello"]);

    let norm = |s: &str| s.replace("\r\n", "\n");

    assert_eq!(exit_code, 0);
    assert_eq!(exit_code, ref_out.exit_code.unwrap_or(-1));
    assert_eq!(
        norm(&String::from_utf8_lossy(&stdout_buf)),
        norm(&ref_out.stdout)
    );
    assert_eq!(stderr_buf, b"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_exit_code_parity() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let exit_code = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["exit", "42"],
            &ShellOptions::default(),
            Option::<Cursor<Vec<u8>>>::None,
            Option::<Vec<u8>>::None,
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect("shell exit failed");

    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "exit", "42"]);

    assert_eq!(exit_code, 42);
    assert_eq!(exit_code, ref_out.exit_code.unwrap_or(-1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_stderr() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();

    let exit_code = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["stderr", "oops error"],
            &ShellOptions::default(),
            Option::<Cursor<Vec<u8>>>::None,
            Some(&mut stdout_buf),
            Some(&mut stderr_buf),
        )
        .await
        .result
        .expect("shell stderr failed");

    assert_eq!(exit_code, 0);
    assert_eq!(stdout_buf, b"");
    assert_eq!(String::from_utf8_lossy(&stderr_buf), "oops error\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_stdin_cat() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let stdin_data = b"piped input line 1\npiped input line 2\n";
    let mut stdout_buf = Vec::new();

    let exit_code = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["cat"],
            &ShellOptions::default(),
            Some(Cursor::new(stdin_data.to_vec())),
            Some(&mut stdout_buf),
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect("shell cat failed");

    assert_eq!(exit_code, 0);
    assert_eq!(stdout_buf, stdin_data);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_v2_n_option_closes_stdin() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let stdin_data = b"this should be ignored";
    let mut stdout_buf = Vec::new();

    let options = ShellOptions {
        close_stdin: true,
        ..Default::default()
    };

    let exit_code = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["cat"],
            &options,
            Some(Cursor::new(stdin_data.to_vec())),
            Some(&mut stdout_buf),
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect("shell -n failed");

    assert_eq!(exit_code, 0);
    assert_eq!(stdout_buf, b"");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_disable_v2_x_option_refusal() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let options = ShellOptions {
        disable_shell_protocol: true,
        no_pty: true,
        ..Default::default()
    };

    let err = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["echo", "hello"],
            &options,
            Option::<Cursor<Vec<u8>>>::None,
            Option::<Vec<u8>>::None,
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect_err("shell -x should fail");

    assert_eq!(err.to_string(), "-x only supports allocating a pty");

    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "-x", "-T", "echo", "hello"]);
    assert_eq!(ref_out.exit_code, Some(1));
    assert_eq!(
        ref_out.stderr.trim(),
        "error: -x only supports allocating a pty"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_no_v2_device_refusal() {
    let mock = MockAdbdProcess::start(&["--no-shell-v2"]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let options = ShellOptions {
        no_pty: true,
        ..Default::default()
    };

    let err = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["echo", "hello"],
            &options,
            Option::<Cursor<Vec<u8>>>::None,
            Option::<Vec<u8>>::None,
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect_err("shell -T on non-v2 device should fail");

    assert_eq!(err.to_string(), "device only supports allocating a pty");

    let ref_out = server.run_client(&["-s", &mock.addr, "shell", "-T", "echo", "hello"]);
    assert_eq!(ref_out.exit_code, Some(1));
    assert_eq!(
        ref_out.stderr.trim(),
        "error: device only supports allocating a pty"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_overlong_command_refusal() {
    let mock = MockAdbdProcess::start(&["--no-shell-v2"]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let long_cmd = "a".repeat(4100);

    let err = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &[&long_cmd],
            &ShellOptions::default(),
            Option::<Cursor<Vec<u8>>>::None,
            Option::<Vec<u8>>::None,
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect_err("overlong shell command should fail");

    assert_eq!(err.to_string(), "shell command too long");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_closed_without_exit_packet() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let mut stdout_buf = Vec::new();

    let exit_code = client
        .shell(
            &TransportSelector::Serial(mock.addr.clone()),
            &["no_exit"],
            &ShellOptions::default(),
            Option::<Cursor<Vec<u8>>>::None,
            Some(&mut stdout_buf),
            Option::<Vec<u8>>::None,
        )
        .await
        .result
        .expect("shell no_exit should complete");

    assert_eq!(exit_code, 255);
    assert_eq!(
        String::from_utf8_lossy(&stdout_buf),
        "no exit packet stdout\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_exec_in_and_exec_out_parity() {
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    // 1. exec_out
    let mut out_buf = Vec::new();
    let exit_code = client
        .exec_out(
            &TransportSelector::Serial(mock.addr.clone()),
            &["echo", "hello world", "foo bar"],
            &mut out_buf,
        )
        .await
        .result
        .expect("exec_out failed");

    let ref_out = server.run_client(&[
        "-s",
        &mock.addr,
        "exec-out",
        "echo",
        "hello world",
        "foo bar",
    ]);

    let norm = |s: &str| s.replace("\r\n", "\n");

    assert_eq!(exit_code, 0);
    assert_eq!(exit_code, ref_out.exit_code.unwrap_or(-1));
    assert_eq!(
        norm(&String::from_utf8_lossy(&out_buf)),
        norm(&ref_out.stdout)
    );

    // 2. exec_in
    let stdin_bytes = b"exec in test stream\n";
    let exit_code_in = client
        .exec_in(
            &TransportSelector::Serial(mock.addr.clone()),
            &["cat"],
            Cursor::new(stdin_bytes.to_vec()),
        )
        .await
        .result
        .expect("exec_in failed");

    assert_eq!(exit_code_in, 0);
}
