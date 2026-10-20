use adb_auth::{fingerprint, KeyStore};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn get_reference_adb() -> Option<PathBuf> {
    let reference_adb = match env::var("ADB_REFERENCE") {
        Ok(val) if !val.is_empty() => PathBuf::from(val),
        _ => {
            let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
            PathBuf::from(format!("{}/../../binaries/linux/adb", manifest_dir))
        }
    };

    if reference_adb.exists() {
        Some(reference_adb)
    } else {
        None
    }
}

fn generate_ref_key(ref_adb: &Path, path: &Path) {
    let status = Command::new(ref_adb)
        .arg("keygen")
        .arg(path)
        .status()
        .expect("Failed to run adb keygen");
    assert!(status.success(), "adb keygen failed for {:?}", path);
}

#[test]
fn test_vendor_key_file_entry() {
    let ref_adb = match get_reference_adb() {
        Some(path) => path,
        None => return,
    };

    let user_dir = TempDir::new().unwrap();
    let vendor_dir = TempDir::new().unwrap();
    let vendor_key = vendor_dir.path().join("vendor_key");
    generate_ref_key(&ref_adb, &vendor_key);

    let store = KeyStore::load(Some(user_dir.path()), Some(vendor_key.to_str().unwrap()));

    let keys = store.private_keys();
    let vendor_key_rsa = adb_auth::read_key_file(&vendor_key).unwrap();
    let vendor_fp = fingerprint(&vendor_key_rsa);

    assert!(
        keys.iter().any(|k| fingerprint(k) == vendor_fp),
        "KeyStore should contain vendor key"
    );
}

#[test]
fn test_vendor_key_directory_entry() {
    let ref_adb = match get_reference_adb() {
        Some(path) => path,
        None => return,
    };

    let user_dir = TempDir::new().unwrap();
    let vendor_dir = TempDir::new().unwrap();

    let key1 = vendor_dir.path().join("key1.adb_key");
    let key2 = vendor_dir.path().join("key2.adb_key");
    let other = vendor_dir.path().join("other.txt");

    generate_ref_key(&ref_adb, &key1);
    generate_ref_key(&ref_adb, &key2);
    fs::write(&other, "not a key content").unwrap();

    let store = KeyStore::load(
        Some(user_dir.path()),
        Some(vendor_dir.path().to_str().unwrap()),
    );

    let keys = store.private_keys();
    let fp1 = fingerprint(&adb_auth::read_key_file(&key1).unwrap());
    let fp2 = fingerprint(&adb_auth::read_key_file(&key2).unwrap());

    assert!(
        keys.iter().any(|k| fingerprint(k) == fp1),
        "KeyStore should load key1.adb_key"
    );
    assert!(
        keys.iter().any(|k| fingerprint(k) == fp2),
        "KeyStore should load key2.adb_key"
    );
}

#[test]
fn test_vendor_keys_duplicate_key_deduplication() {
    let ref_adb = match get_reference_adb() {
        Some(path) => path,
        None => return,
    };

    let user_dir = TempDir::new().unwrap();
    let vendor_dir = TempDir::new().unwrap();

    let key1 = vendor_dir.path().join("key1.adb_key");
    let key2 = vendor_dir.path().join("key2.adb_key");

    generate_ref_key(&ref_adb, &key1);
    // Copy key1 to key2 so they have identical key material/fingerprint
    fs::copy(&key1, &key2).unwrap();

    let store = KeyStore::load(
        Some(user_dir.path()),
        Some(vendor_dir.path().to_str().unwrap()),
    );

    let fp1 = fingerprint(&adb_auth::read_key_file(&key1).unwrap());
    let match_count = store
        .private_keys()
        .iter()
        .filter(|k| fingerprint(k) == fp1)
        .count();

    assert_eq!(
        match_count, 1,
        "Duplicate key should only be loaded once (deduplicated by fingerprint)"
    );
}

#[test]
fn test_vendor_keys_missing_path() {
    let user_dir = TempDir::new().unwrap();
    let missing_path = user_dir.path().join("does_not_exist_vendor_keys_dir");

    let store = KeyStore::load(Some(user_dir.path()), Some(missing_path.to_str().unwrap()));

    // Should not panic, user key should still be generated/loaded
    assert!(store.user_key().is_some());
}

#[test]
fn test_vendor_keys_separator_with_two_entries() {
    let ref_adb = match get_reference_adb() {
        Some(path) => path,
        None => return,
    };

    let user_dir = TempDir::new().unwrap();
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    let key1 = dir1.path().join("key1.adb_key");
    let key2 = dir2.path().join("key2.adb_key");

    generate_ref_key(&ref_adb, &key1);
    generate_ref_key(&ref_adb, &key2);

    let sep = if cfg!(windows) { ";" } else { ":" };
    let vendor_env = format!("{}{}{}", dir1.path().display(), sep, dir2.path().display());

    let store = KeyStore::load(Some(user_dir.path()), Some(&vendor_env));

    let fp1 = fingerprint(&adb_auth::read_key_file(&key1).unwrap());
    let fp2 = fingerprint(&adb_auth::read_key_file(&key2).unwrap());

    let keys = store.private_keys();
    assert!(keys.iter().any(|k| fingerprint(k) == fp1));
    assert!(keys.iter().any(|k| fingerprint(k) == fp2));
}

#[test]
fn test_vendor_keys_ordering_by_fingerprint() {
    let ref_adb = match get_reference_adb() {
        Some(path) => path,
        None => return,
    };

    let user_dir = TempDir::new().unwrap();
    let vendor_dir = TempDir::new().unwrap();

    for i in 1..=5 {
        let key_path = vendor_dir.path().join(format!("key{}.adb_key", i));
        generate_ref_key(&ref_adb, &key_path);
    }

    let store = KeyStore::load(
        Some(user_dir.path()),
        Some(vendor_dir.path().to_str().unwrap()),
    );

    let keys = store.private_keys();
    assert!(keys.len() >= 5);

    // Verify keys are strictly sorted in ascending fingerprint order
    for window in keys.windows(2) {
        let fp1 = fingerprint(&window[0]);
        let fp2 = fingerprint(&window[1]);
        assert!(
            fp1 < fp2,
            "Private keys must be ordered by fingerprint in ascending byte order"
        );
    }
}

#[test]
fn test_oracle_against_reference_adb() {
    let ref_adb = match get_reference_adb() {
        Some(path) => path,
        None => return,
    };

    let test_dir = TempDir::new().unwrap();
    let user_home = test_dir.path().join("user_home");
    let vendor_dir = test_dir.path().join("vendor_dir");

    fs::create_dir_all(user_home.join(".android")).unwrap();
    fs::create_dir_all(&vendor_dir).unwrap();

    let key1 = vendor_dir.join("a_key.adb_key");
    let key2 = vendor_dir.join("b_key.adb_key");
    let key2_dup = vendor_dir.join("c_dup_key.adb_key");
    let non_key = vendor_dir.join("ignored.txt");

    generate_ref_key(&ref_adb, &key1);
    generate_ref_key(&ref_adb, &key2);
    fs::copy(&key2, &key2_dup).unwrap();
    fs::write(&non_key, "ignored data").unwrap();

    // Spawn reference ADB server in nodaemon mode with ADB_TRACE=auth
    let mut child = Command::new(&ref_adb)
        .arg("server")
        .arg("nodaemon")
        .env("HOME", &user_home)
        .env("USERPROFILE", &user_home)
        .env("ADB_TRACE", "auth")
        .env("ADB_VENDOR_KEYS", &vendor_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start reference adb server");

    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = child.kill();
    let output = child
        .wait_with_output()
        .expect("Failed to wait on adb server");

    let combined_output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // Parse fingerprints logged as loaded by reference ADB
    let mut ref_loaded_fingerprints = Vec::new();
    for line in combined_output.lines() {
        if line.contains("loaded new key") {
            if let Some(pos) = line.find("with fingerprint ") {
                let fp_hex = line[pos + "with fingerprint ".len()..].trim();
                ref_loaded_fingerprints.push(fp_hex.to_uppercase());
            }
        }
    }

    assert!(
        !ref_loaded_fingerprints.is_empty(),
        "Reference ADB should log loaded key fingerprints"
    );

    // Run Rust KeyStore on same paths
    let store = KeyStore::load(
        Some(&user_home.join(".android")),
        Some(vendor_dir.to_str().unwrap()),
    );

    let rust_keys = store.private_keys();
    let rust_fingerprints: Vec<String> = rust_keys
        .iter()
        .map(|k| {
            fingerprint(k)
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect()
        })
        .collect();

    // Verify deduplicated loaded fingerprints set matches reference ADB
    let mut ref_sorted = ref_loaded_fingerprints.clone();
    ref_sorted.sort();
    ref_sorted.dedup();

    assert_eq!(
        rust_fingerprints, ref_sorted,
        "Rust KeyStore loaded fingerprints and ordering must match reference ADB"
    );
}
