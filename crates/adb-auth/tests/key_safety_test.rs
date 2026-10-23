use std::env;
use std::fs;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

#[test]
fn test_1_generate_key_on_existing_file_fails_and_preserves_bytes() {
    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("existing_key");

    let original_bytes = b"ORIGINAL KEY CONTENT DO NOT OVERWRITE";
    fs::write(&key_path, original_bytes).unwrap();

    let result = adb_auth::generate_key(&key_path);
    assert!(
        result.is_err(),
        "generate_key on an existing file should return an error"
    );

    let current_bytes = fs::read(&key_path).unwrap();
    assert_eq!(
        current_bytes, original_bytes,
        "existing file bytes must remain unchanged"
    );
}

#[test]
fn test_2_concurrent_load_or_generate_creates_one_key_and_returns_same_key() {
    let temp_dir = TempDir::new().unwrap();
    let key_path = Arc::new(temp_dir.path().join("concurrent_adbkey"));

    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = Arc::clone(&key_path);
        handles.push(thread::spawn(move || {
            adb_auth::load_or_generate(&*path).expect("load_or_generate failed in thread")
        }));
    }

    let mut keys = Vec::new();
    for h in handles {
        keys.push(h.join().unwrap());
    }

    assert_eq!(keys.len(), 8);
    let fp0 = adb_auth::fingerprint(&keys[0]);
    for key in &keys[1..] {
        assert_eq!(
            adb_auth::fingerprint(key),
            fp0,
            "All threads must return the same key"
        );
    }
}

#[test]
fn test_3_no_home_env_returns_error_and_does_not_pollute_cwd() {
    // Record current working directory state
    let cwd = env::current_dir().unwrap();
    let entries_before: Vec<_> = fs::read_dir(&cwd)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    temp_env::with_vars(
        [
            ("HOME", None::<&str>),
            ("USERPROFILE", None::<&str>),
            ("LOGDIR", None::<&str>),
        ],
        || {
            let path_res = adb_auth::adb_get_android_dir_path();
            assert!(
                path_res.is_err(),
                "adb_get_android_dir_path should return an error when HOME and USERPROFILE are unset"
            );

            let key_path_res = adb_auth::get_user_key_path();
            assert!(
                key_path_res.is_err(),
                "get_user_key_path should return an error when HOME and USERPROFILE are unset"
            );
        },
    );

    let entries_after: Vec<_> = fs::read_dir(&cwd)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();

    assert_eq!(
        entries_before, entries_after,
        "Nothing should be created in the working directory when home is unset"
    );
}

#[test]
fn test_4_pkcs8_pem_format_and_reference_adb_compatibility() {
    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("pkcs8_key");

    adb_auth::generate_key(&key_path).expect("Failed to generate key");

    let pem_content = fs::read_to_string(&key_path).unwrap();
    assert!(
        pem_content.starts_with("-----BEGIN PRIVATE KEY-----"),
        "Written private key must start with -----BEGIN PRIVATE KEY-----, got: {}",
        pem_content.lines().next().unwrap_or("")
    );

    // Verify it loads in Rust
    let _rust_key =
        adb_auth::read_key_file(&key_path).expect("Rust should load generated PKCS#8 key");
    let rust_pubkey =
        adb_auth::adb_auth_pubkey(&key_path).expect("Rust adb_auth_pubkey should succeed");

    // If reference adb is available, test against it
    let reference_adb = match env::var("ADB_REFERENCE") {
        Ok(val) if !val.is_empty() => val,
        _ => {
            let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
            format!("{}/../../binaries/linux/adb", manifest_dir)
        }
    };

    if std::path::Path::new(&reference_adb).exists() {
        let output = Command::new(&reference_adb)
            .arg("pubkey")
            .arg(&key_path)
            .output()
            .expect("Failed to run reference adb pubkey");

        assert!(
            output.status.success(),
            "reference adb pubkey failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let ref_pubkey = String::from_utf8_lossy(&output.stdout);
        // Compare the base64 public key portion (first word) between Rust and reference ADB
        let rust_b64 = rust_pubkey.split_whitespace().next().unwrap_or("");
        let ref_b64 = ref_pubkey.split_whitespace().next().unwrap_or("");
        assert_eq!(
            rust_b64, ref_b64,
            "Reference ADB pubkey base64 string must match Rust pubkey base64 string"
        );
    }
}

#[test]
#[cfg(not(unix))]
fn test_5_unix_file_permissions_0600_and_preexisting_0644_unchanged() {
    eprintln!("Skipping test_5 on non-Unix: file mode bits (0600/0644) are Unix-specific");
}

#[test]
#[cfg(unix)]
fn test_5_unix_file_permissions_0600_and_preexisting_0644_unchanged() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let key_path = temp_dir.path().join("perm_key");

    adb_auth::generate_key(&key_path).expect("Failed to generate key");

    let meta = fs::metadata(&key_path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "Generated key file mode must be 0600 on Unix, got {:o}",
        mode
    );

    // Test that loading an existing key with 0644 permissions does NOT alter file permissions
    let existing_key_path = temp_dir.path().join("loose_key");
    adb_auth::generate_key(&existing_key_path).expect("Failed to generate key");

    let mut perms = fs::metadata(&existing_key_path).unwrap().permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&existing_key_path, perms).unwrap();

    // Load the key file
    let _loaded = adb_auth::read_key_file(&existing_key_path).expect("Read key file");
    let _loaded_or_gen = adb_auth::load_or_generate(&existing_key_path).expect("load_or_generate");

    let meta_after = fs::metadata(&existing_key_path).unwrap();
    let mode_after = meta_after.permissions().mode() & 0o777;
    assert_eq!(
        mode_after, 0o644,
        "Loading an existing key file with 0644 mode must not alter its permissions, got {:o}",
        mode_after
    );
}

#[test]
#[cfg(not(unix))]
fn test_6_android_dir_creation_permissions_0750() {
    eprintln!("Skipping test_6 on non-Unix: directory mode bits (0750) are Unix-specific");
}

#[test]
#[cfg(unix)]
fn test_6_android_dir_creation_permissions_0750() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let fake_home = temp_dir.path().join("home_user");
    fs::create_dir(&fake_home).unwrap();

    temp_env::with_vars(
        [
            ("HOME", Some(fake_home.to_str().unwrap())),
            ("USERPROFILE", Some(fake_home.to_str().unwrap())),
        ],
        || {
            let android_dir = adb_auth::ensure_android_dir().expect("ensure_android_dir failed");
            assert!(android_dir.exists(), ".android dir should exist");

            let meta = fs::metadata(&android_dir).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(
                mode, 0o750,
                ".android directory permission mode on Unix must be 0750, got {:o}",
                mode
            );
        },
    );
}
