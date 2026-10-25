use adb_client::{
    AdbClient, CompressionType, DefaultSessionFactory, LaunchError, PullOptions, ServerAddress,
    ServerLauncher, SyncSession, TransportSelector,
};
use compat::TestServer;
use mock_adbd::MockAdbdOptions;
use std::env;
use std::future::Future;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::time::UNIX_EPOCH;
use tempfile::TempDir;

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
async fn test_pull_file_sizes_and_attrs_parity_v2_and_v1() {
    for enable_v2 in [true, false] {
        let mock = MockAdbdProcess::start_with_options(MockAdbdOptions {
            enable_sendrecv_v2: enable_v2,
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

        let files = [
            ("/data/f_0.bin", 0),
            ("/data/f_1.bin", 1),
            ("/data/f_65535.bin", 65535),
            ("/data/f_65536.bin", 65536),
            ("/data/f_300k.bin", 307200),
        ];

        for (remote_path, expected_len) in files {
            let rust_dir = TempDir::new().unwrap();
            let ref_dir = TempDir::new().unwrap();

            let filename = Path::new(remote_path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            let rust_dst = rust_dir.path().join(filename);
            let ref_dst = ref_dir.path().join(filename);

            // Pull using SyncSession
            let mut sync = SyncSession::connect_with_selector(&client, &selector)
                .await
                .expect("sync connect failed");

            let opts = PullOptions {
                copy_attrs: true,
                compression: CompressionType::None,
            };

            let result = sync
                .pull(&[remote_path], &rust_dst, &opts)
                .await
                .expect("pull failed");

            assert!(
                result.success,
                "pull was not successful: {:?}",
                result.errors
            );
            assert_eq!(result.bytes_transferred, expected_len as u64);

            // Pull using reference client
            let ref_res = server.run_client(&[
                "-s",
                &mock.addr,
                "pull",
                "-a",
                remote_path,
                ref_dst.to_str().unwrap(),
            ]);
            assert!(ref_res.success(), "ref pull failed: {}", ref_res.stderr);

            // Compare file bytes
            let rust_bytes = std::fs::read(&rust_dst).expect("read rust file failed");
            let ref_bytes = std::fs::read(&ref_dst).expect("read ref file failed");
            assert_eq!(
                rust_bytes, ref_bytes,
                "file bytes mismatch for {remote_path} (v2={enable_v2})"
            );
            assert_eq!(rust_bytes.len(), expected_len);

            // Compare modified time and permissions with -a
            let rust_meta = std::fs::metadata(&rust_dst).unwrap();
            let ref_meta = std::fs::metadata(&ref_dst).unwrap();

            let rust_mtime = rust_meta
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let ref_mtime = ref_meta
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            assert_eq!(rust_mtime, ref_mtime, "mtime mismatch for {remote_path}");

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    rust_meta.permissions().mode() & 0o777,
                    ref_meta.permissions().mode() & 0o777,
                    "mode mismatch for {remote_path}"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pull_destination_rules() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
    let server = TestServer::start();

    let _ = server.run_client(&["connect", &mock.addr]);

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
        .expect("sync connect failed");

    let opts = PullOptions::default();

    // Rule A: Destination is an existing directory
    let temp_dir = TempDir::new().unwrap();
    let res_dir = sync
        .pull(&["/data/f_1.bin"], temp_dir.path(), &opts)
        .await
        .expect("pull into existing dir failed");
    assert!(res_dir.success);
    let expected_file = temp_dir.path().join("f_1.bin");
    assert!(expected_file.exists());
    assert_eq!(std::fs::read(&expected_file).unwrap(), vec![0x42]);

    // Rule B: Missing parent directory
    let missing_parent = temp_dir.path().join("nonexistent_parent").join("dst.bin");
    let res_missing_parent = sync
        .pull(&["/data/f_1.bin"], &missing_parent, &opts)
        .await
        .expect("pull with missing parent returned err");
    assert!(!res_missing_parent.success);
    assert!(
        res_missing_parent.errors[0].contains("cannot create file/directory"),
        "unexpected error text: {:?}",
        res_missing_parent.errors
    );

    // Rule C: Multiple sources into non-directory target
    let single_file_dst = temp_dir.path().join("single_dst.bin");
    std::fs::write(&single_file_dst, b"dummy").unwrap();
    let res_multi = sync
        .pull(&["/data/f_1.bin", "/data/f_0.bin"], &single_file_dst, &opts)
        .await
        .expect("pull multi into non-dir returned err");
    assert!(!res_multi.success);
    assert!(
        res_multi.errors[0].contains("is not a directory"),
        "unexpected error text: {:?}",
        res_multi.errors
    );

    // Rule D: Trailing separator on file destination that exists but is not a directory
    let trailing_sep_dst = format!("{}/", single_file_dst.display());
    let res_trailing = sync
        .pull(&["/data/f_1.bin"], Path::new(&trailing_sep_dst), &opts)
        .await
        .expect("pull trailing sep returned err");
    assert!(!res_trailing.success);
    assert!(
        res_trailing.errors[0].contains("Not a directory"),
        "unexpected error text: {:?}",
        res_trailing.errors
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pull_remote_missing_and_fail_and_cleanup() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
    let server = TestServer::start();

    let _ = server.run_client(&["connect", &mock.addr]);

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
        .expect("sync connect failed");

    let temp_dir = TempDir::new().unwrap();
    let opts = PullOptions::default();

    // 1. Remote file missing
    let dst_missing = temp_dir.path().join("missing.bin");
    let res_missing = sync
        .pull(&["/data/no_such_file.bin"], &dst_missing, &opts)
        .await
        .expect("pull missing remote returned err");
    assert!(!res_missing.success);
    assert!(
        res_missing.errors[0].contains("remote object '/data/no_such_file.bin' does not exist"),
        "unexpected error: {:?}",
        res_missing.errors
    );
    assert!(!dst_missing.exists(), "no partial file should exist");

    // 2. Device FAIL response
    let dst_fail = temp_dir.path().join("fail.bin");
    let res_fail = sync
        .pull(&["/data/fail.bin"], &dst_fail, &opts)
        .await
        .expect("pull fail returned err");
    assert!(!res_fail.success);
    assert!(
        res_fail.errors[0].contains("failed to copy '/data/fail.bin' to"),
        "unexpected error: {:?}",
        res_fail.errors
    );
    assert!(
        !dst_fail.exists(),
        "no partial file should remain after FAIL"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pull_mid_transfer_disconnect() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
    let server = TestServer::start();

    let _ = server.run_client(&["connect", &mock.addr]);

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
        .expect("sync connect failed");

    let temp_dir = TempDir::new().unwrap();
    let dst_disc = temp_dir.path().join("partial.bin");
    let opts = PullOptions::default();

    let res_disc = sync
        .pull(&["/data/disconnect.bin"], &dst_disc, &opts)
        .await
        .expect("pull disconnect returned result");

    assert!(!res_disc.success);
    assert!(
        !dst_disc.exists(),
        "partial file must be removed on disconnect"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pull_compression_rejection() {
    let mock = MockAdbdProcess::start_with_options(MockAdbdOptions::default());
    let server = TestServer::start();

    let _ = server.run_client(&["connect", &mock.addr]);

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
        .expect("sync connect failed");

    let temp_dir = TempDir::new().unwrap();
    let dst = temp_dir.path().join("out.bin");

    let opts = PullOptions {
        copy_attrs: false,
        compression: CompressionType::Brotli,
    };

    let err = sync
        .pull(&["/data/f_1.bin"], &dst, &opts)
        .await
        .expect_err("compressed pull should fail");

    assert!(err.to_string().contains("compression"));
}
