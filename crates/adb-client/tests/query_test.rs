use adb_client::{
    AdbClient, ClientError, DefaultSessionFactory, ServerAddress, ServerLauncher, TransportSelector,
};
use compat::TestServer;
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

struct DummyLauncher;

impl ServerLauncher for DummyLauncher {
    fn launch_server<'a>(
        &'a self,
        _addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send + 'a>> {
        Box::pin(async move {
            Err(ClientError::KillFailed(
                "DummyLauncher cannot start daemon".to_string(),
            ))
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_query_no_device() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::new(addr);
    let session_factory = DefaultSessionFactory;
    let launcher = DummyLauncher;

    // Helper to normalize CRLF for cross-platform process output comparison
    let norm = |s: &str| s.replace("\r\n", "\n");

    // 1. adb devices (no device)
    let devices_out = client
        .devices(false, &session_factory, &launcher)
        .await
        .expect("devices query failed");
    let ref_devices_out = server.run_client(&["devices"]);
    assert!(ref_devices_out.success());
    assert_eq!(norm(&devices_out), norm(&ref_devices_out.stdout));
    assert_eq!(devices_out, "List of devices attached\n\n");

    // 2. adb devices -l (no device)
    let devices_l_out = client
        .devices(true, &session_factory, &launcher)
        .await
        .expect("devices -l query failed");
    let ref_devices_l_out = server.run_client(&["devices", "-l"]);
    assert!(ref_devices_l_out.success());
    assert_eq!(norm(&devices_l_out), norm(&ref_devices_l_out.stdout));
    assert_eq!(devices_l_out, "List of devices attached\n\n");

    // 3. adb devices with invalid arguments
    let err_arg = client
        .devices_with_args(&["-x"], &session_factory, &launcher)
        .await
        .expect_err("invalid args should fail");
    assert_eq!(err_arg.to_string(), "adb devices [-l]");

    fn extract_error_message(output_stderr: &str) -> &str {
        let trimmed = output_stderr.trim();
        if let Some((_prefix, rest)) = trimmed.split_once(": ") {
            rest
        } else {
            trimmed
        }
    }

    let ref_bad_arg = server.run_client(&["devices", "-x"]);
    assert_eq!(ref_bad_arg.exit_code, Some(1));
    // Ref client outputs "adb: adb devices [-l]" (or "adb.exe: adb devices [-l]" on Windows) on stderr when error_exit("adb devices [-l]") is called.
    assert_eq!(
        extract_error_message(&ref_bad_arg.stderr),
        err_arg.to_string()
    );

    // 4. adb get-state (no device)
    let err_state = client
        .get_state(&TransportSelector::Any, &session_factory)
        .await
        .expect_err("get-state without device should fail");
    assert_eq!(err_state.to_string(), "error: no devices/emulators found");

    let ref_state = server.run_client(&["get-state"]);
    assert_eq!(ref_state.exit_code, Some(1));
    assert_eq!(ref_state.stderr.trim(), "error: no devices/emulators found");

    // 5. adb get-serialno (no device)
    let err_serial = client
        .get_serialno(&TransportSelector::Any, &session_factory)
        .await
        .expect_err("get-serialno without device should fail");
    assert_eq!(err_serial.to_string(), "error: no devices/emulators found");

    let ref_serial = server.run_client(&["get-serialno"]);
    assert_eq!(ref_serial.exit_code, Some(1));
    assert_eq!(
        ref_serial.stderr.trim(),
        "error: no devices/emulators found"
    );

    // 6. adb get-devpath (no device)
    let err_devpath = client
        .get_devpath(&TransportSelector::Any, &session_factory)
        .await
        .expect_err("get-devpath without device should fail");
    assert_eq!(err_devpath.to_string(), "error: no devices/emulators found");

    let ref_devpath = server.run_client(&["get-devpath"]);
    assert_eq!(ref_devpath.exit_code, Some(1));
    assert_eq!(
        ref_devpath.stderr.trim(),
        "error: no devices/emulators found"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_query_mock_device() {
    let mock = MockAdbdProcess::start();
    let server = TestServer::start();

    // Connect mock adbd via reference client `adb connect 127.0.0.1:<port>`
    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(
        connect_out.success(),
        "adb connect failed: {}",
        connect_out.stderr
    );

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::new(addr);
    let session_factory = DefaultSessionFactory;
    let launcher = DummyLauncher;

    let norm = |s: &str| s.replace("\r\n", "\n");

    // 1. adb devices with connected mock adbd
    let devices_out = client
        .devices(false, &session_factory, &launcher)
        .await
        .expect("devices query failed");
    let ref_devices = server.run_client(&["devices"]);
    assert!(ref_devices.success());
    assert_eq!(norm(&devices_out), norm(&ref_devices.stdout));
    assert!(devices_out.contains(&format!("{}\tdevice\n", mock.addr)));

    // 2. adb devices -l with connected mock adbd
    let devices_l_out = client
        .devices(true, &session_factory, &launcher)
        .await
        .expect("devices -l query failed");
    let ref_devices_l = server.run_client(&["devices", "-l"]);
    assert!(ref_devices_l.success());

    // Masked fields comparison:
    // - transport_id:<id>: Assigned dynamically and sequentially by the ADB server for each connected device.
    // - product / model / device properties: Fixed or determined by mock adbd banner properties.
    fn mask_dynamic_fields(input: &str) -> String {
        input
            .replace("\r\n", "\n")
            .lines()
            .map(|line| {
                if let Some(idx) = line.find("transport_id:") {
                    let (prefix, rest) = line.split_at(idx);
                    let end_idx = rest.find(' ').unwrap_or(rest.len());
                    format!("{prefix}transport_id:MASKED{}", &rest[end_idx..])
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + if input.ends_with('\n') { "\n" } else { "" }
    }

    assert_eq!(
        mask_dynamic_fields(&devices_l_out),
        mask_dynamic_fields(&ref_devices_l.stdout)
    );

    // 3. adb get-state with connected mock adbd
    let state_out = client
        .get_state(
            &TransportSelector::Serial(mock.addr.clone()),
            &session_factory,
        )
        .await
        .expect("get-state failed");
    let ref_state = server.run_client(&["-s", &mock.addr, "get-state"]);
    assert!(ref_state.success());
    assert_eq!(norm(&state_out), norm(&ref_state.stdout));
    assert_eq!(state_out, "device\n");

    // 4. adb get-serialno with connected mock adbd
    let serial_out = client
        .get_serialno(
            &TransportSelector::Serial(mock.addr.clone()),
            &session_factory,
        )
        .await
        .expect("get-serialno failed");
    let ref_serial = server.run_client(&["-s", &mock.addr, "get-serialno"]);
    assert!(ref_serial.success());
    assert_eq!(norm(&serial_out), norm(&ref_serial.stdout));
    assert_eq!(serial_out, format!("{}\n", mock.addr));

    // 5. adb get-devpath with connected mock adbd
    let devpath_out = client
        .get_devpath(
            &TransportSelector::Serial(mock.addr.clone()),
            &session_factory,
        )
        .await
        .expect("get-devpath failed");
    let ref_devpath = server.run_client(&["-s", &mock.addr, "get-devpath"]);
    assert!(ref_devpath.success());
    assert_eq!(norm(&devpath_out), norm(&ref_devpath.stdout));
    assert_eq!(devpath_out, "unknown\n");
}
