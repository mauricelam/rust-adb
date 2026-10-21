use adb_client::{resolve_server_address, AddressError, ServerAddressOptions};
use std::env;
use std::path::PathBuf;
use std::process::Command;

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
    let err = resolve_server_address(&options, &Default::default()).unwrap_err();

    assert_eq!(msg_after_prefix, err.to_string());
    assert_eq!(err, AddressError::InvalidPort("99999".to_string()));
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
    let err = resolve_server_address(&options, &Default::default()).unwrap_err();

    assert_eq!(msg_after_prefix, err.to_string());
    assert_eq!(err, AddressError::IncompatibleOptions);
}
