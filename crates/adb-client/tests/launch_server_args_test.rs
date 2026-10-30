use adb_client::{ExternalBinaryLauncher, ServerAddress};
use std::env;
use std::fs;
use std::path::PathBuf;

fn get_reference_adb_path() -> PathBuf {
    if let Ok(path) = env::var("ADB_SERVER_BIN") {
        return PathBuf::from(path);
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_launch_server_arguments() {
    let temp_dir = tempfile::tempdir().unwrap();
    let log_path = temp_dir.path().join("args.txt");
    let ref_adb = get_reference_adb_path();

    #[cfg(unix)]
    let script_path = temp_dir.path().join("launcher.sh");
    #[cfg(windows)]
    let script_path = temp_dir.path().join("launcher.bat");

    #[cfg(unix)]
    {
        let script_content = format!(
            "#!/bin/sh\necho \"$@\" >> \"{}\"\nexec \"{}\" \"$@\"\n",
            log_path.display(),
            ref_adb.display()
        );
        fs::write(&script_path, script_content).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(windows)]
    {
        let script_content = format!(
            "@echo off\r\necho %* >> \"{}\"\r\n\"{}\" %*\r\n",
            log_path.display(),
            ref_adb.display()
        );
        fs::write(&script_path, script_content).unwrap();
    }

    let port = 5997;
    let addr = ServerAddress { host: None, port };
    let session_factory = adb_client::DefaultSessionFactory::new();
    let launcher = ExternalBinaryLauncher::new(&script_path);

    let mut kill_events = Vec::new();
    let _ = adb_client::kill_server(&addr, &session_factory, &mut kill_events).await;

    let mut events = Vec::new();
    let _res = adb_client::start_server_with_options(
        &addr,
        true,
        Some("serial123"),
        &launcher,
        &mut events,
    )
    .await;

    let _ = adb_client::kill_server(&addr, &session_factory, &mut kill_events).await;

    if log_path.exists() {
        let logged = fs::read_to_string(&log_path).unwrap();
        assert!(logged.contains("-a"), "Logged args missing -a: {}", logged);
        assert!(
            logged.contains("--one-device serial123"),
            "Logged args missing --one-device: {}",
            logged
        );
    }
}
