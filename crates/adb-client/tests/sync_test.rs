use adb_client::{
    AdbClient, DefaultSessionFactory, LaunchError, ServerAddress, ServerLauncher, SyncError,
    SyncSession, TransportSelector,
};
use compat::TestServer;
use mock_adbd::MockAdbdOptions;
use std::env;
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::pin::Pin;
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
    fn start_with_options(options: MockAdbdOptions) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen").arg("127.0.0.1:0");
        if !options.enable_shell_v2 {
            cmd.arg("--no-shell-v2");
        }
        if !options.enable_stat_v2 {
            cmd.arg("--no-stat-v2");
        }
        if !options.enable_ls_v2 {
            cmd.arg("--no-ls-v2");
        }
        if !options.enable_sendrecv_v2 {
            cmd.arg("--no-sendrecv-v2");
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
async fn test_sync_ls_parity_v2_and_v1() {
    // Test with v2 features enabled
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "connect failed: {}",
        connect_out.stderr
    );

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock.addr.clone());

    // 1. Test ls on empty directory /data/empty
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .result
        .expect("sync session connect failed");

    let entries_empty = sync.ls("/data/empty").await.expect("ls /data/empty failed");
    let ref_empty = server.run_client(&["-s", &mock.addr, "ls", "/data/empty"]);
    assert!(ref_empty.success());

    let norm = |s: &str| s.replace("\r\n", "\n");

    let rust_empty_out = entries_empty
        .iter()
        .map(|e| e.format_line())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(rust_empty_out, norm(&ref_empty.stdout));

    // 2. Test ls on directory with files, subdirectories, symlink and multi-byte UTF-8 names (/data)
    let entries_data = sync.ls("/data").await.expect("ls /data failed");
    let ref_data = server.run_client(&["-s", &mock.addr, "ls", "/data"]);
    assert!(ref_data.success());

    let rust_data_out = entries_data
        .iter()
        .map(|e| e.format_line())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(rust_data_out, norm(&ref_data.stdout));

    // 3. Test ls on missing path /nonexistent
    let entries_missing = sync
        .ls("/nonexistent")
        .await
        .expect("ls /nonexistent failed");
    let ref_missing = server.run_client(&["-s", &mock.addr, "ls", "/nonexistent"]);
    assert!(ref_missing.success());

    let rust_missing_out = entries_missing
        .iter()
        .map(|e| e.format_line())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(rust_missing_out, norm(&ref_missing.stdout));

    // Repeat with v2 features disabled (ls_v1)
    let mock_v1 = MockAdbdProcess::start_with_options(MockAdbdOptions {
        enable_ls_v2: false,
        enable_stat_v2: false,
        ..Default::default()
    });
    let connect_out_v1 = server.run_client(&["connect", &mock_v1.addr]);
    assert!(connect_out_v1.success());

    let selector_v1 = TransportSelector::Serial(mock_v1.addr.clone());
    let mut sync_v1 = SyncSession::connect_with_selector(&client, &selector_v1)
        .await
        .result
        .expect("sync v1 session connect failed");

    let entries_v1 = sync_v1.ls("/data").await.expect("ls /data v1 failed");
    let ref_v1 = server.run_client(&["-s", &mock_v1.addr, "ls", "/data"]);
    assert!(ref_v1.success());

    let rust_v1_out = entries_v1
        .iter()
        .map(|e| e.format_line())
        .collect::<Vec<_>>()
        .join("");
    assert_eq!(rust_v1_out, norm(&ref_v1.stdout));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_sync_stat_lstat_fallback() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
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

    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .result
        .expect("sync connect failed");

    // 1. stat /data/f_1.bin
    let st = sync.stat("/data/f_1.bin").await.expect("stat failed");
    assert!(st.is_file());
    assert_eq!(st.size, 1);
    assert_eq!(st.mtime, 2000);

    // 2. stat missing file
    let err_missing = sync
        .stat("/data/no_such_file")
        .await
        .expect_err("stat missing file should fail");
    assert_eq!(err_missing.raw_os_error(), Some(2)); // ENOENT = 2

    // 3. Test with stat_v2 disabled (v1 fallback)
    let mock_v1 = MockAdbdProcess::start_with_options(MockAdbdOptions {
        enable_stat_v2: false,
        ..Default::default()
    });
    let _ = server.run_client(&["connect", &mock_v1.addr]);
    let selector_v1 = TransportSelector::Serial(mock_v1.addr.clone());

    let mut sync_v1 = SyncSession::connect_with_selector(&client, &selector_v1)
        .await
        .result
        .expect("sync v1 connect failed");

    // stat without stat_v2 returns NotSupported
    let err_nosup = sync_v1
        .stat("/data/f_1.bin")
        .await
        .expect_err("stat without v2 should fail");
    assert!(matches!(err_nosup, SyncError::NotSupported(_)));

    // lstat uses v1
    let st_lstat_v1 = sync_v1
        .lstat("/data/f_1.bin")
        .await
        .expect("lstat v1 failed");
    assert!(st_lstat_v1.is_file());
    assert_eq!(st_lstat_v1.size, 1);

    // stat_with_fallback falls back to lstat
    let st_fb = sync_v1
        .stat_with_fallback("/data/f_1.bin")
        .await
        .expect("stat_with_fallback failed");
    assert!(st_fb.is_file());
    assert_eq!(st_fb.size, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_sync_overlong_path_refusal() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "connect failed: {}",
        connect_out.stderr
    );

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .result
        .expect("sync connect failed");

    let long_path = "a/".repeat(600); // 1200 bytes > 1024
    let err = sync
        .ls(&long_path)
        .await
        .expect_err("overlong path should fail");
    assert!(err
        .to_string()
        .contains("SendRequest failed: path too long: 1200"));
    assert_eq!(err.raw_os_error(), Some(36)); // ENAMETOOLONG = 36
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_symlink_to_dir_fallback_and_stat_fields() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions {
        enable_stat_v2: false,
        ..Default::default()
    });
    let server = TestServer::start();

    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "connect failed: {}",
        connect_out.stderr
    );

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock.addr.clone());

    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .result
        .expect("sync connect failed");

    // Query stat /data using fallback
    let st = sync
        .stat_with_fallback("/data")
        .await
        .expect("stat_with_fallback /data failed");
    assert!(st.is_dir());
}

#[test]
fn test_errno_translation_and_string() {
    use adb_client::{errno_from_wire, errno_to_string};

    let wire_enoent = 2;
    let host_enoent = errno_from_wire(wire_enoent);
    #[cfg(unix)]
    assert_eq!(host_enoent, libc::ENOENT);
    #[cfg(windows)]
    assert_eq!(host_enoent, 2);

    let err_str = errno_to_string(host_enoent);
    assert!(!err_str.is_empty());

    assert_eq!(errno_to_string(0), "Success");
}
