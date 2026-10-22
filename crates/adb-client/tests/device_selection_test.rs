use adb_client::{
    AdbClient, ClientError, DeviceSelection, SelectionError, ServerAddress, SessionError,
    TransportSelector,
};
use compat::TestServer;
use std::collections::HashMap;
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

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

fn get_server_error_reason(err: &ClientError) -> &str {
    match err {
        ClientError::Session(SessionError::Fail(reason)) => reason.as_str(),
        ClientError::QueryFailed(reason) => reason.as_str(),
        _ => "",
    }
}

fn extract_server_fail_message(stderr: &str) -> &str {
    let trimmed = stderr.trim();
    let rest = trimmed
        .strip_prefix("adb: ")
        .or_else(|| trimmed.strip_prefix("adb.exe: "))
        .unwrap_or(trimmed);
    rest.strip_prefix("error: ").unwrap_or(rest)
}

fn run_client_isolated(server: &TestServer, args: &[&str]) -> compat::TestOutput {
    temp_env::with_var_unset("ANDROID_SERIAL", || server.run_client(args))
}

fn run_client_with_env(server: &TestServer, args: &[&str], serial: &str) -> compat::TestOutput {
    temp_env::with_var("ANDROID_SERIAL", Some(serial), || server.run_client(args))
}

#[test]
fn test_device_selection_table() {
    struct TestCase {
        name: &'static str,
        args: Vec<&'static str>,
        env: HashMap<&'static str, &'static str>,
        expected_res: Result<(DeviceSelection, Vec<&'static str>), SelectionError>,
    }

    let no_env: HashMap<&'static str, &'static str> = HashMap::new();
    let mut env_serial = HashMap::new();
    env_serial.insert("ANDROID_SERIAL", "env_device_123");

    let cases = vec![
        TestCase {
            name: "attached serial -s123",
            args: vec!["-s123", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: Some("123".into()),
                    transport_id: None,
                    usb: false,
                    local: false,
                    selector: TransportSelector::Serial("123".into()),
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "spaced serial -s 123",
            args: vec!["-s", "123", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: Some("123".into()),
                    transport_id: None,
                    usb: false,
                    local: false,
                    selector: TransportSelector::Serial("123".into()),
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "invalid option -sabc",
            args: vec!["-sabc", "get-state"],
            env: no_env.clone(),
            expected_res: Err(SelectionError::SRequiresArgument),
        },
        TestCase {
            name: "missing arg -s",
            args: vec!["-s"],
            env: no_env.clone(),
            expected_res: Err(SelectionError::SRequiresArgument),
        },
        TestCase {
            name: "attached transport-id -t123",
            args: vec!["-t123", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: None,
                    transport_id: Some(123),
                    usb: false,
                    local: false,
                    selector: TransportSelector::Id(123),
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "spaced transport-id -t 123",
            args: vec!["-t", "123", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: None,
                    transport_id: Some(123),
                    usb: false,
                    local: false,
                    selector: TransportSelector::Id(123),
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "invalid option -tabc",
            args: vec!["-tabc", "get-state"],
            env: no_env.clone(),
            expected_res: Err(SelectionError::TRequiresArgument),
        },
        TestCase {
            name: "non numeric transport-id -t abc",
            args: vec!["-t", "abc", "get-state"],
            env: no_env.clone(),
            expected_res: Err(SelectionError::InvalidTransportId),
        },
        TestCase {
            name: "missing arg -t",
            args: vec!["-t"],
            env: no_env.clone(),
            expected_res: Err(SelectionError::TRequiresArgument),
        },
        TestCase {
            name: "usb device -d",
            args: vec!["-d", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: None,
                    transport_id: None,
                    usb: true,
                    local: false,
                    selector: TransportSelector::Usb,
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "local device -e",
            args: vec!["-e", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: None,
                    transport_id: None,
                    usb: false,
                    local: true,
                    selector: TransportSelector::Local,
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "-d with -e",
            args: vec!["-d", "-e", "get-state"],
            env: no_env.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: None,
                    transport_id: None,
                    usb: false,
                    local: true,
                    selector: TransportSelector::Local,
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "ANDROID_SERIAL fallback",
            args: vec!["get-state"],
            env: env_serial.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: Some("env_device_123".into()),
                    transport_id: None,
                    usb: false,
                    local: false,
                    selector: TransportSelector::Serial("env_device_123".into()),
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "ANDROID_SERIAL fallback with -t id",
            args: vec!["-t", "456", "get-state"],
            env: env_serial.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: Some("env_device_123".into()),
                    transport_id: Some(456),
                    usb: false,
                    local: false,
                    selector: TransportSelector::Id(456),
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "ANDROID_SERIAL ignored with -d",
            args: vec!["-d", "get-state"],
            env: env_serial.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: None,
                    transport_id: None,
                    usb: true,
                    local: false,
                    selector: TransportSelector::Usb,
                },
                vec!["get-state"],
            )),
        },
        TestCase {
            name: "ANDROID_SERIAL overridden with -s",
            args: vec!["-s", "override_serial", "get-state"],
            env: env_serial.clone(),
            expected_res: Ok((
                DeviceSelection {
                    serial: Some("override_serial".into()),
                    transport_id: None,
                    usb: false,
                    local: false,
                    selector: TransportSelector::Serial("override_serial".into()),
                },
                vec!["get-state"],
            )),
        },
    ];

    for case in cases {
        let env_fn = |key: &str| case.env.get(key).map(|s| s.to_string());
        let res = DeviceSelection::from_args_and_env(&case.args, env_fn);
        match (&res, &case.expected_res) {
            (Ok((sel, rem)), Ok((exp_sel, exp_rem))) => {
                assert_eq!(sel, exp_sel, "Case failed: {}", case.name);
                let rem_str: Vec<&str> = rem.iter().map(|s| s.as_str()).collect();
                assert_eq!(
                    &rem_str, exp_rem,
                    "Case remaining args failed: {}",
                    case.name
                );
            }
            (Err(err), Err(exp_err)) => {
                assert_eq!(err, exp_err, "Case failed: {}", case.name);
            }
            _ => panic!("Case failed: {}. Got {:?}", case.name, res),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_differential_no_device() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };
    let client = AdbClient::new(addr);

    // 1. host-features when no device connected
    let host_feats_out = client
        .host_features()
        .await
        .result
        .expect("host_features failed");
    let ref_host_feats = run_client_isolated(&server, &["host-features"]);
    assert!(ref_host_feats.success());
    assert_eq!(norm(&host_feats_out), norm(&ref_host_feats.stdout));

    // 2. Query operations when no device connected
    let selectors = vec![
        ("default", TransportSelector::Any, vec!["get-state"]),
        ("usb", TransportSelector::Usb, vec!["-d", "get-state"]),
        ("local", TransportSelector::Local, vec!["-e", "get-state"]),
        (
            "serial",
            TransportSelector::Serial("dummy_serial".into()),
            vec!["-s", "dummy_serial", "get-state"],
        ),
        (
            "id",
            TransportSelector::Id(9999),
            vec!["-t", "9999", "get-state"],
        ),
    ];

    for (name, selector, ref_args) in selectors {
        // get-state
        let err_state = client
            .get_state(&selector)
            .await
            .result
            .expect_err(&format!("get-state should fail for {name}"));
        let ref_state = run_client_isolated(&server, &ref_args);
        assert_eq!(ref_state.exit_code, Some(1));
        assert_eq!(
            get_server_error_reason(&err_state),
            extract_server_fail_message(&ref_state.stderr),
            "get-state error mismatch for {name}"
        );

        // get-serialno
        let mut serial_ref_args = ref_args.clone();
        serial_ref_args.pop();
        serial_ref_args.push("get-serialno");
        let err_serial = client
            .get_serialno(&selector)
            .await
            .result
            .expect_err(&format!("get-serialno should fail for {name}"));
        let ref_serial = run_client_isolated(&server, &serial_ref_args);
        assert_eq!(ref_serial.exit_code, Some(1));
        assert_eq!(
            get_server_error_reason(&err_serial),
            extract_server_fail_message(&ref_serial.stderr),
            "get-serialno error mismatch for {name}"
        );

        // transport-id
        let mut tid_ref_args = ref_args.clone();
        tid_ref_args.pop();
        tid_ref_args.push("transport-id");
        let err_tid = client
            .transport_id(&selector)
            .await
            .result
            .expect_err(&format!("transport-id should fail for {name}"));
        let ref_tid = run_client_isolated(&server, &tid_ref_args);
        assert_eq!(ref_tid.exit_code, Some(1));
        assert_eq!(
            get_server_error_reason(&err_tid),
            extract_server_fail_message(&ref_tid.stderr),
            "transport-id error mismatch for {name}"
        );

        // features
        let mut feat_ref_args = ref_args.clone();
        feat_ref_args.pop();
        feat_ref_args.push("features");
        let err_feat = client
            .features(&selector)
            .await
            .result
            .expect_err(&format!("features should fail for {name}"));
        let ref_feat = run_client_isolated(&server, &feat_ref_args);
        assert_eq!(ref_feat.exit_code, Some(1));
        assert_eq!(
            get_server_error_reason(&err_feat),
            extract_server_fail_message(&ref_feat.stderr),
            "features error mismatch for {name}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_differential_one_mock_device() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    let connect_out = run_client_isolated(&server, &["connect", &mock.addr]);
    assert!(connect_out.success(), "adb connect failed");

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };
    let client = AdbClient::new(addr);

    // 1. host-features
    let host_feats = client
        .host_features()
        .await
        .result
        .expect("host_features failed");
    let ref_host_feats = run_client_isolated(&server, &["host-features"]);
    assert!(ref_host_feats.success());
    assert_eq!(norm(&host_feats), norm(&ref_host_feats.stdout));

    // 2. Query device with serial selector
    let selector = TransportSelector::Serial(mock.addr.clone());

    let state = client
        .get_state(&selector)
        .await
        .result
        .expect("get-state failed");
    let ref_state = run_client_isolated(&server, &["-s", &mock.addr, "get-state"]);
    assert!(ref_state.success());
    assert_eq!(norm(&state), norm(&ref_state.stdout));

    let serialno = client
        .get_serialno(&selector)
        .await
        .result
        .expect("get-serialno failed");
    let ref_serialno = run_client_isolated(&server, &["-s", &mock.addr, "get-serialno"]);
    assert!(ref_serialno.success());
    assert_eq!(norm(&serialno), norm(&ref_serialno.stdout));

    let feats = client
        .features(&selector)
        .await
        .result
        .expect("features failed");
    let ref_feats = run_client_isolated(&server, &["-s", &mock.addr, "features"]);
    assert!(ref_feats.success());
    assert_eq!(norm(&feats), norm(&ref_feats.stdout));

    // 3. transport-id comparison
    let tid_str = client
        .transport_id(&selector)
        .await
        .result
        .expect("transport_id failed");
    let ref_tid = run_client_isolated(&server, &["-s", &mock.addr, "transport-id"]);
    assert!(ref_tid.success());
    assert_eq!(norm(&tid_str), norm(&ref_tid.stdout));

    let tid_val: u64 = tid_str
        .trim()
        .parse()
        .expect("transport-id must parse as u64");
    assert!(tid_val > 0);

    // 4. Query using TransportSelector::Id(tid_val)
    let id_selector = TransportSelector::Id(tid_val);
    let state_by_id = client
        .get_state(&id_selector)
        .await
        .result
        .expect("get-state by id failed");
    let ref_state_by_id = run_client_isolated(&server, &["-t", &tid_val.to_string(), "get-state"]);
    assert!(ref_state_by_id.success());
    assert_eq!(norm(&state_by_id), norm(&ref_state_by_id.stdout));

    // 5. Query using ANDROID_SERIAL environment variable
    let mut env = HashMap::new();
    env.insert("ANDROID_SERIAL".to_string(), mock.addr.clone());

    let (selection, rem) =
        DeviceSelection::from_args_and_env(vec!["get-state"], |k| env.get(k).cloned()).unwrap();
    assert_eq!(rem, vec!["get-state"]);
    assert_eq!(
        selection.selector,
        TransportSelector::Serial(mock.addr.clone())
    );

    let ref_env_state = run_client_with_env(&server, &["get-state"], &mock.addr);
    assert!(ref_env_state.success());
    let client_env_state = client.get_state(&selection.selector).await.result.unwrap();
    assert_eq!(norm(&client_env_state), norm(&ref_env_state.stdout));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_differential_two_mock_devices() {
    let mock1 = MockAdbdProcess::start();
    let mock2 = MockAdbdProcess::start();
    let server = TestServer::start();

    let conn1 = run_client_isolated(&server, &["connect", &mock1.addr]);
    assert!(conn1.success());
    let conn2 = run_client_isolated(&server, &["connect", &mock2.addr]);
    assert!(conn2.success());

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };
    let client = AdbClient::new(addr);

    // 1. Unambiguous host query host-features succeeds
    let host_feats = client
        .host_features()
        .await
        .result
        .expect("host_features failed");
    let ref_host_feats = run_client_isolated(&server, &["host-features"]);
    assert!(ref_host_feats.success());
    assert_eq!(norm(&host_feats), norm(&ref_host_feats.stdout));

    // 2. Ambiguous default selection without option fails with "more than one device/emulator"
    let err_state = client
        .get_state(&TransportSelector::Any)
        .await
        .result
        .expect_err("ambiguous get-state should fail");
    let ref_err_state = run_client_isolated(&server, &["get-state"]);
    assert_eq!(ref_err_state.exit_code, Some(1));
    assert_eq!(
        get_server_error_reason(&err_state),
        extract_server_fail_message(&ref_err_state.stderr)
    );

    // 3. Target mock1 and mock2 specifically by serial
    let sel1 = TransportSelector::Serial(mock1.addr.clone());
    let sel2 = TransportSelector::Serial(mock2.addr.clone());

    let serial1 = client.get_serialno(&sel1).await.result.unwrap();
    let ref_serial1 = run_client_isolated(&server, &["-s", &mock1.addr, "get-serialno"]);
    assert_eq!(norm(&serial1), norm(&ref_serial1.stdout));

    let serial2 = client.get_serialno(&sel2).await.result.unwrap();
    let ref_serial2 = run_client_isolated(&server, &["-s", &mock2.addr, "get-serialno"]);
    assert_eq!(norm(&serial2), norm(&ref_serial2.stdout));

    // 4. Transport IDs for both devices
    let tid1_str = client.transport_id(&sel1).await.result.unwrap();
    let tid2_str = client.transport_id(&sel2).await.result.unwrap();

    let ref_tid1 = run_client_isolated(&server, &["-s", &mock1.addr, "transport-id"]);
    let ref_tid2 = run_client_isolated(&server, &["-s", &mock2.addr, "transport-id"]);

    assert_eq!(norm(&tid1_str), norm(&ref_tid1.stdout));
    assert_eq!(norm(&tid2_str), norm(&ref_tid2.stdout));

    let tid1: u64 = tid1_str.trim().parse().unwrap();
    let tid2: u64 = tid2_str.trim().parse().unwrap();
    assert_ne!(tid1, tid2);

    // 5. Target mock1 and mock2 specifically by transport ID (-t)
    let id_sel1 = TransportSelector::Id(tid1);
    let id_sel2 = TransportSelector::Id(tid2);

    let serial_by_id1 = client.get_serialno(&id_sel1).await.result.unwrap();
    let ref_serial_by_id1 =
        run_client_isolated(&server, &["-t", &tid1.to_string(), "get-serialno"]);
    assert_eq!(norm(&serial_by_id1), norm(&ref_serial_by_id1.stdout));
    assert_eq!(serial_by_id1.trim(), mock1.addr);

    let serial_by_id2 = client.get_serialno(&id_sel2).await.result.unwrap();
    let ref_serial_by_id2 =
        run_client_isolated(&server, &["-t", &tid2.to_string(), "get-serialno"]);
    assert_eq!(norm(&serial_by_id2), norm(&ref_serial_by_id2.stdout));
    assert_eq!(serial_by_id2.trim(), mock2.addr);
}
