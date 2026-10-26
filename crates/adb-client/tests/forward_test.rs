use adb_client::{AdbClient, ServerAddress, TransportSelector};
use compat::TestServer;
use std::env;
use std::io::{BufRead, BufReader};
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
    let status = Command::new("cargo")
        .args(["build", "-p", "mock-adbd"])
        .status()
        .expect("Failed to build mock-adbd binary");
    assert!(status.success(), "Failed to build mock-adbd");
    bin_path
}

struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start() -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen").arg("127.0.0.1:0");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_forward_management_parity() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    // Connect mock adbd device to reference server
    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success(), "connect failed: {:?}", connect_out);

    let client = AdbClient::new(ServerAddress {
        host: Some("127.0.0.1".to_string()),
        port: server.port,
    });
    let selector = TransportSelector::Serial(mock.addr.clone());

    let norm = |s: &str| s.replace("\r\n", "\n");

    // 1. List with nothing forwarded
    let list_empty = client
        .forward_list(&TransportSelector::Any)
        .await
        .result
        .unwrap();
    let ref_list_empty = server.run_client(&["forward", "--list"]);
    assert_eq!(norm(&list_empty), norm(&ref_list_empty.stdout));

    // 2. Add tcp:0 tcp:1234
    let add_res = client
        .forward_add(&selector, "tcp:0", "tcp:1234", false)
        .await
        .result
        .unwrap();
    let allocated_port: u16 = add_res.trim().parse().expect("Expected valid port number");
    assert!(allocated_port > 0);

    let local_spec = format!("tcp:{allocated_port}");
    let list_after_add = client
        .forward_list(&TransportSelector::Any)
        .await
        .result
        .unwrap();
    let ref_list_after_add = server.run_client(&["forward", "--list"]);
    let expected_rule = format!("{} {local_spec} tcp:1234\n", mock.addr);
    assert_eq!(norm(&list_after_add), norm(&ref_list_after_add.stdout));
    assert!(list_after_add.contains(&expected_rule));

    // 3. Add same local port twice (rebind allowed)
    let rebind_res = client
        .forward_add(&selector, &local_spec, "tcp:5678", false)
        .await
        .result
        .unwrap();
    assert_eq!(rebind_res, "");
    let list_after_rebind = client
        .forward_list(&TransportSelector::Any)
        .await
        .result
        .unwrap();
    let expected_rebind_rule = format!("{} {local_spec} tcp:5678\n", mock.addr);
    assert!(list_after_rebind.contains(&expected_rebind_rule));

    // 4. Add same local port with --no-rebind (failure text from server)
    let no_rebind_err = client
        .forward_add(&selector, &local_spec, "tcp:9999", true)
        .await
        .result
        .unwrap_err();
    let ref_no_rebind = server.run_client(&[
        "-s",
        &mock.addr,
        "forward",
        "--no-rebind",
        &local_spec,
        "tcp:9999",
    ]);
    assert!(!ref_no_rebind.success());
    assert!(no_rebind_err
        .to_string()
        .contains("cannot rebind existing socket"));
    assert!(ref_no_rebind
        .stderr
        .contains("cannot rebind existing socket"));

    // 5. Remove one rule
    let remove_res = client
        .forward_remove(&selector, &local_spec)
        .await
        .result
        .unwrap();
    assert_eq!(remove_res, "");
    let list_after_remove = client
        .forward_list(&TransportSelector::Any)
        .await
        .result
        .unwrap();
    assert!(!list_after_remove.contains(&local_spec));

    // 6. Remove unknown rule
    let remove_unknown_err = client
        .forward_remove(&selector, "tcp:9999")
        .await
        .result
        .unwrap_err();
    let ref_remove_unknown =
        server.run_client(&["-s", &mock.addr, "forward", "--remove", "tcp:9999"]);
    assert!(!ref_remove_unknown.success());
    assert!(remove_unknown_err
        .to_string()
        .contains("listener 'tcp:9999' not found"));
    assert!(ref_remove_unknown
        .stderr
        .contains("listener 'tcp:9999' not found"));

    // 7. Remove all rules
    let port1 = client
        .forward_add(&selector, "tcp:0", "tcp:1111", false)
        .await
        .result
        .unwrap()
        .trim()
        .to_string();
    let port2 = client
        .forward_add(&selector, "tcp:0", "tcp:2222", false)
        .await
        .result
        .unwrap()
        .trim()
        .to_string();
    assert!(!port1.is_empty() && !port2.is_empty());

    let remove_all_res = client.forward_remove_all(&selector).await.result.unwrap();
    assert_eq!(remove_all_res, "");
    let list_after_remove_all = client
        .forward_list(&TransportSelector::Any)
        .await
        .result
        .unwrap();
    let ref_list_after_remove_all = server.run_client(&["forward", "--list"]);
    assert_eq!(
        norm(&list_after_remove_all),
        norm(&ref_list_after_remove_all.stdout)
    );
    assert_eq!(list_after_remove_all.trim(), "");

    // 8. Target validation failures (forward_targets_are_valid)
    let val_src_err = client
        .forward_add(&selector, "tcp:-1", "tcp:1234", false)
        .await
        .result
        .unwrap_err();
    let ref_val_src = server.run_client(&["forward", "tcp:-1", "tcp:1234"]);
    assert!(!ref_val_src.success());
    assert_eq!(val_src_err.to_string(), "Invalid source port: '-1'");
    assert!(ref_val_src.stderr.contains("Invalid source port: '-1'"));

    let val_dst_err = client
        .forward_add(&selector, "tcp:1234", "tcp:abc", false)
        .await
        .result
        .unwrap_err();
    let ref_val_dst = server.run_client(&["forward", "tcp:1234", "tcp:abc"]);
    assert!(!ref_val_dst.success());
    assert_eq!(
        val_dst_err.to_string(),
        "Invalid destination port: 'abc'"
    );
    assert!(ref_val_dst
        .stderr
        .contains("Invalid destination port: 'abc'"));

    let val_dst_zero_err = client
        .forward_add(&selector, "tcp:1234", "tcp:0", false)
        .await
        .result
        .unwrap_err();
    let ref_val_dst_zero = server.run_client(&["forward", "tcp:1234", "tcp:0"]);
    assert!(!ref_val_dst_zero.success());
    assert_eq!(
        val_dst_zero_err.to_string(),
        "Invalid destination port: '0'"
    );
    assert!(ref_val_dst_zero
        .stderr
        .contains("Invalid destination port: '0'"));

    // 9. dev-raw destination against device without dev-raw feature
    let dev_raw_err = client
        .forward_add(&selector, "tcp:1234", "dev-raw:foo", false)
        .await
        .result
        .unwrap_err();
    let ref_dev_raw = server.run_client(&["-s", &mock.addr, "forward", "tcp:1234", "dev-raw:foo"]);
    assert!(!ref_dev_raw.success());
    assert_eq!(
        dev_raw_err.to_string(),
        "dev-raw is not supported by the device"
    );
    assert!(ref_dev_raw
        .stderr
        .contains("dev-raw is not supported by the device"));

    // 10. Argument count / usage errors (forward_with_args)
    let usage_cases: &[(&[&str], &str)] = &[
        (&[], "forward requires an argument"),
        (&["--list", "extra"], "--list doesn't take any arguments"),
        (
            &["--remove-all", "extra"],
            "--remove-all doesn't take any arguments",
        ),
        (&["--remove"], "--remove requires an argument"),
        (
            &["--no-rebind", "tcp:1234"],
            "--no-rebind takes two arguments",
        ),
        (&["tcp:1234"], "forward takes two arguments"),
    ];

    for (args, expected_msg) in usage_cases {
        let err = adb_cli::parse_and_exec_forward(&client, &selector, args)
            .await
            .result
            .unwrap_err();
        let ref_out = server.run_client(&[&["forward"], *args].concat());
        assert!(!ref_out.success());
        assert_eq!(err.to_string(), *expected_msg);
        assert!(ref_out.stderr.contains(expected_msg));
    }
}
