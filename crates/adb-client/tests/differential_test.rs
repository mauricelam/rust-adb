use adb_client::{
    check_server_version, kill_server, DefaultSessionFactory, LaunchError, ServerAddress,
    ServerAddressOptions, ServerLauncher,
};
use compat::TestServer;
use std::env;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Command;
use std::time::Duration;
use tokio::net::TcpStream;

fn get_reference_adb_path() -> PathBuf {
    if let Ok(path) = env::var("ADB_REFERENCE") {
        return PathBuf::from(path);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to locate repo root");

    #[cfg(target_os = "windows")]
    let binary = repo_root.join("binaries/windows/adb.exe");
    #[cfg(target_os = "macos")]
    let binary = repo_root.join("binaries/mac/adb");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let binary = repo_root.join("binaries/linux/adb");

    binary
}

fn extract_error_message(output_stderr: &str) -> &str {
    let trimmed = output_stderr.trim();
    if let Some((_prefix, rest)) = trimmed.split_once(": ") {
        rest
    } else {
        trimmed
    }
}

#[test]
fn test_differential_invalid_port() {
    let ref_adb = get_reference_adb_path();
    assert!(
        ref_adb.exists(),
        "Reference ADB binary not found at {:?}",
        ref_adb
    );

    let output = Command::new(&ref_adb)
        .args(["-P", "99999", "devices"])
        .output()
        .expect("Failed to execute reference adb");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg_after_prefix = extract_error_message(&stderr);

    let options = ServerAddressOptions {
        port: Some("99999".to_string()),
        ..Default::default()
    };
    let err = adb_client::resolve_server_address(&options, &Default::default()).unwrap_err();

    assert_eq!(msg_after_prefix, err.to_string());
    assert_eq!(
        err,
        adb_client::AddressError::InvalidPort("99999".to_string())
    );
}

#[test]
fn test_differential_incompatible_options() {
    let ref_adb = get_reference_adb_path();
    assert!(
        ref_adb.exists(),
        "Reference ADB binary not found at {:?}",
        ref_adb
    );

    let output = Command::new(&ref_adb)
        .args(["-L", "tcp:1", "-H", "x", "devices"])
        .output()
        .expect("Failed to execute reference adb");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg_after_prefix = extract_error_message(&stderr);

    let options = ServerAddressOptions {
        socket: Some("tcp:1".to_string()),
        host: Some("x".to_string()),
        ..Default::default()
    };
    let err = adb_client::resolve_server_address(&options, &Default::default()).unwrap_err();

    assert_eq!(msg_after_prefix, err.to_string());
    assert_eq!(err, adb_client::AddressError::IncompatibleOptions);
}

struct NoOpLauncher;

impl ServerLauncher for NoOpLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a str,
        _one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_differential_check_version_and_kill_server() {
    let server = TestServer::start();
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let session_factory = DefaultSessionFactory;
    let launcher = NoOpLauncher;
    let mut events = Vec::new();

    // 1. check_server_version against reference server returns matching version (41)
    let res = check_server_version(&addr, &session_factory, &launcher, &mut events).await;
    assert!(res.is_ok(), "check_server_version failed: {:?}", res);
    assert!(
        events.is_empty(),
        "expected no events for matching version, got {:?}",
        events
    );

    // 2. kill_server against reference server
    let mut kill_events = Vec::new();
    let kill_res = kill_server(&addr, &session_factory, &mut kill_events).await;
    assert!(kill_res.is_ok(), "kill_server failed: {:?}", kill_res);

    // 3. Verify server shut down and port stops accepting connections
    tokio::time::sleep(Duration::from_millis(200)).await;
    let conn_res = TcpStream::connect(("127.0.0.1", server.port)).await;
    assert!(
        conn_res.is_err(),
        "reference server should have shut down after kill_server, but connection succeeded"
    );
}
