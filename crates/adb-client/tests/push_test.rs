use adb_client::{
    AdbClient, CompressionType, DefaultSessionFactory, LaunchError, PushOptions, ServerAddress,
    ServerLauncher, SyncSession, TransportSelector,
};
use compat::TestServer;
use mock_adbd::fs::Node;
use mock_adbd::{InMemoryFs, MockAdbdOptions, MockAdbdServer};
use std::future::Future;
use std::pin::Pin;
use tempfile::TempDir;

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
async fn test_push_file_sizes_and_attrs_parity_v2_and_v1() {
    for enable_v2 in [true, false] {
        let fs = InMemoryFs::new();
        let mock_rust = MockAdbdServer::bind_with_options(
            "127.0.0.1:0",
            MockAdbdOptions {
                enable_sendrecv_v2: enable_v2,
                fs: fs.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let mock_ref = MockAdbdServer::bind_with_options(
            "127.0.0.1:0",
            MockAdbdOptions {
                enable_sendrecv_v2: enable_v2,
                fs: fs.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let mock_rust_addr = mock_rust.local_addr().to_string();
        let mock_ref_addr = mock_ref.local_addr().to_string();

        let server = TestServer::start();

        let _ = server.run_client(&["connect", &mock_rust_addr]);
        let _ = server.run_client(&["connect", &mock_ref_addr]);

        let addr = ServerAddress {
            host: Some("127.0.0.1".into()),
            port: server.port,
        };

        let client = AdbClient::builder(addr)
            .session_factory(DefaultSessionFactory::new())
            .launcher(DummyLauncher)
            .build();

        let selector_rust = TransportSelector::Serial(mock_rust_addr.clone());

        let temp_dir = TempDir::new().unwrap();

        let files = [
            ("f_0.bin", vec![]),
            ("f_1.bin", vec![0x55]),
            ("f_65535.bin", vec![0xAA; 65535]),
            ("f_65536.bin", vec![0xBB; 65536]),
            ("f_300k.bin", (0..307200).map(|i| (i % 256) as u8).collect()),
        ];

        for (filename, contents) in files {
            let local_path = temp_dir.path().join(filename);
            std::fs::write(&local_path, &contents).unwrap();

            let target_remote = format!("/data/{filename}");

            // Push using Rust SyncSession into mock_rust
            let mut sync = SyncSession::connect_with_selector(&client, &selector_rust)
                .await
                .expect("sync connect failed");

            let push_opts = PushOptions::default();
            let res = sync
                .push(&[local_path.to_str().unwrap()], &target_remote, &push_opts)
                .await
                .expect("push failed");

            assert!(res.success, "rust push failed: {:?}", res.errors);
            assert_eq!(res.bytes_transferred, contents.len() as u64);

            // Push using Reference client into mock_ref
            let ref_res = server.run_client(&[
                "-s",
                &mock_ref_addr,
                "push",
                local_path.to_str().unwrap(),
                &target_remote,
            ]);
            assert!(ref_res.success(), "ref push failed: {}", ref_res.stderr);

            // Verify stored data in mock_rust vs mock_ref in memory FS
            let rust_node = mock_rust
                .fs()
                .get(&target_remote)
                .expect("rust pushed file missing in mock fs");
            let ref_node = mock_ref
                .fs()
                .get(&target_remote)
                .expect("ref pushed file missing in mock fs");

            match (rust_node, ref_node) {
                (Node::File(rf), Node::File(ff)) => {
                    assert_eq!(rf.data, ff.data, "data mismatch for {filename}");
                    assert_eq!(rf.data, contents, "data content mismatch for {filename}");
                    assert_eq!(
                        rf.mode & 0o777,
                        ff.mode & 0o777,
                        "mode mismatch for {filename}"
                    );
                }
                _ => panic!("Expected File node"),
            }
        }

        mock_rust.shutdown();
        mock_ref.shutdown();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_push_symlink_and_readonly_mode() {
    let fs = InMemoryFs::new();
    let mock = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            fs: fs.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mock_addr = mock.local_addr().to_string();

    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock_addr]);

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock_addr.clone());
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .unwrap();

    let temp_dir = TempDir::new().unwrap();

    // Readonly file
    let ro_file = temp_dir.path().join("readonly.txt");
    std::fs::write(&ro_file, b"read only content").unwrap();
    let mut perms = std::fs::metadata(&ro_file).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&ro_file, perms).unwrap();

    let push_opts = PushOptions::default();
    let res_ro = sync
        .push(
            &[ro_file.to_str().unwrap()],
            "/data/readonly.txt",
            &push_opts,
        )
        .await
        .unwrap();
    assert!(res_ro.success);

    let ro_node = fs.get("/data/readonly.txt").unwrap();
    assert_eq!(ro_node.size(), 17);

    // Symlink
    #[cfg(unix)]
    {
        let symlink_file = temp_dir.path().join("link.txt");
        std::os::unix::fs::symlink("target_path.txt", &symlink_file).unwrap();

        let res_sym = sync
            .push(
                &[symlink_file.to_str().unwrap()],
                "/data/link.txt",
                &push_opts,
            )
            .await
            .unwrap();
        assert!(res_sym.success);

        let sym_node = fs.get("/data/link.txt").unwrap();
        if let Node::File(f) = sym_node {
            // C++ sends symlink as target_path + \0
            assert_eq!(f.data, b"target_path.txt\0");
        } else {
            panic!("expected file node for pushed symlink");
        }
    }

    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_push_destination_rules() {
    let fs = InMemoryFs::new();
    let mock = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            fs: fs.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mock_addr = mock.local_addr().to_string();

    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock_addr]);

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock_addr.clone());
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .unwrap();

    let temp_dir = TempDir::new().unwrap();
    let src1 = temp_dir.path().join("file1.txt");
    let src2 = temp_dir.path().join("file2.txt");
    std::fs::write(&src1, b"hello 1").unwrap();
    std::fs::write(&src2, b"hello 2").unwrap();

    let opts = PushOptions::default();

    // Rule A: Destination is existing directory (/data)
    let res_dir = sync
        .push(&[src1.to_str().unwrap()], "/data", &opts)
        .await
        .unwrap();
    assert!(res_dir.success);
    assert!(fs.get("/data/file1.txt").is_some());

    // Rule B: Missing parent directory on device
    let res_no_parent = sync
        .push(&[src1.to_str().unwrap()], "/nonexistent/path.txt", &opts)
        .await
        .unwrap();
    assert!(!res_no_parent.success);
    assert!(
        res_no_parent.errors[0].contains("remote failed to create file")
            || res_no_parent.errors[0].contains("No such file or directory"),
        "unexpected error: {:?}",
        res_no_parent.errors
    );

    // Rule C: Multiple sources to single file
    let res_multi = sync
        .push(
            &[src1.to_str().unwrap(), src2.to_str().unwrap()],
            "/data/f_1.bin",
            &opts,
        )
        .await
        .unwrap();
    assert!(!res_multi.success);
    assert!(
        res_multi.errors[0].contains("is not a directory"),
        "unexpected error: {:?}",
        res_multi.errors
    );

    // Rule D: Trailing slash on existing file destination
    let res_trailing = sync
        .push(&[src1.to_str().unwrap()], "/data/f_1.bin/", &opts)
        .await
        .unwrap();
    assert!(!res_trailing.success);
    assert!(
        res_trailing.errors[0].contains("Not a directory"),
        "unexpected error: {:?}",
        res_trailing.errors
    );

    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_push_unreadable_local_file() {
    let fs = InMemoryFs::new();
    let mock = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            fs: fs.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mock_addr = mock.local_addr().to_string();

    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock_addr]);

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock_addr.clone());
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .unwrap();

    let temp_dir = TempDir::new().unwrap();
    let missing_local = temp_dir.path().join("missing.txt");

    let opts = PushOptions::default();
    let res = sync
        .push(&[missing_local.to_str().unwrap()], "/data/dst.txt", &opts)
        .await
        .unwrap();

    assert!(!res.success);
    assert!(
        res.errors[0].contains("cannot stat"),
        "unexpected error: {:?}",
        res.errors
    );

    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_push_deferred_acknowledgements_128_files() {
    let fs = InMemoryFs::new();
    let mock = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            fs: fs.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mock_addr = mock.local_addr().to_string();

    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock_addr]);

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock_addr.clone());
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .unwrap();

    let temp_dir = TempDir::new().unwrap();
    let count = 150;
    let mut sources = Vec::new();

    for i in 0..count {
        let path = temp_dir.path().join(format!("file_{i}.txt"));
        std::fs::write(&path, format!("data {i}")).unwrap();
        sources.push(path);
    }

    let source_strs: Vec<&str> = sources.iter().map(|p| p.to_str().unwrap()).collect();
    let opts = PushOptions::default();

    let res = sync.push(&source_strs, "/data", &opts).await.unwrap();

    assert!(res.success, "push failed: {:?}", res.errors);
    assert_eq!(res.outcomes.len(), count);

    for i in 0..count {
        assert!(
            fs.get(&format!("/data/file_{i}.txt")).is_some(),
            "missing pushed file /data/file_{i}.txt in mock fs"
        );
    }

    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_push_failing_device_mid_batch() {
    let fs = InMemoryFs::new();
    let mock = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            fs: fs.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mock_addr = mock.local_addr().to_string();

    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock_addr]);

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock_addr.clone());
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .unwrap();

    let temp_dir = TempDir::new().unwrap();
    let mut sources = Vec::new();

    for i in 1..=5 {
        let path = temp_dir.path().join(format!("f_{i}.txt"));
        std::fs::write(&path, format!("content {i}")).unwrap();
        sources.push(path);
    }

    let source_strs: Vec<&str> = sources.iter().map(|p| p.to_str().unwrap()).collect();
    let opts = PushOptions::default();

    // Push 5 files where file 3 targets /nonexistent/file3.txt (which fails on device)
    // First two files go to /data, file 3 to /nonexistent/file3.txt
    let res1 = sync.push(&source_strs[0..2], "/data", &opts).await.unwrap();
    assert!(res1.success);

    let res_fail = sync
        .push(&source_strs[2..3], "/nonexistent/f_3.txt", &opts)
        .await
        .unwrap();
    assert!(!res_fail.success);
    assert!(res_fail.errors[0].contains("remote failed to create file"));

    mock.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_push_options_rejections() {
    let mock = MockAdbdServer::bind_with_options("127.0.0.1:0", MockAdbdOptions::default())
        .await
        .unwrap();
    let mock_addr = mock.local_addr().to_string();

    let server = TestServer::start();
    let _ = server.run_client(&["connect", &mock_addr]);

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let selector = TransportSelector::Serial(mock_addr.clone());
    let mut sync = SyncSession::connect_with_selector(&client, &selector)
        .await
        .unwrap();

    let temp_dir = TempDir::new().unwrap();
    let f = temp_dir.path().join("test.txt");
    std::fs::write(&f, b"test").unwrap();

    // 1. Compression
    let err_comp = sync
        .push(
            &[f.to_str().unwrap()],
            "/data/test.txt",
            &PushOptions {
                compression: CompressionType::Brotli,
                ..Default::default()
            },
        )
        .await
        .expect_err("compression should be rejected");
    assert!(err_comp.to_string().contains("compression"));

    // 2. Dry run
    let err_dry = sync
        .push(
            &[f.to_str().unwrap()],
            "/data/test.txt",
            &PushOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .await
        .expect_err("dry run should be rejected");
    assert!(err_dry.to_string().contains("dry-run"));

    // 3. Sync option
    let err_sync = sync
        .push(
            &[f.to_str().unwrap()],
            "/data/test.txt",
            &PushOptions {
                sync: true,
                ..Default::default()
            },
        )
        .await
        .expect_err("sync option should be rejected");
    assert!(err_sync.to_string().contains("sync"));

    mock.shutdown();
}
