use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};

/// Resolves the path to the compiled `adb-cli` binary (`adb`).
pub fn get_adb_cli_bin() -> PathBuf {
    env::var("ADB_CLIENT")
        .map(PathBuf::from)
        .expect("ADB_CLIENT environment variable must be set to the path of the adb binary. Please build adb-cli (e.g. `cargo build -p adb-cli`) and set ADB_CLIENT.")
}

/// Computes the SHA-256 digest of a binary file as a hex string.
fn compute_binary_sha256(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "Failed to read binary file {} for checksum: {e}",
            path.display()
        )
    });
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

#[test]
fn test_binary_checksum_validation() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let root = PathBuf::from(manifest_dir).join("../..");
    let checksums_path = root.join("binaries").join("CHECKSUMS.txt");
    assert!(
        checksums_path.exists(),
        "Checksum file missing: {}",
        checksums_path.display()
    );

    let content = std::fs::read_to_string(&checksums_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", checksums_path.display()));

    let mut expected_entries = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(
            parts.len(),
            2,
            "Invalid checksum line in {}: {}",
            checksums_path.display(),
            line
        );
        let expected_hash = parts[0].to_lowercase();
        let rel_path = parts[1];
        expected_entries.insert(rel_path.to_string(), expected_hash);
    }

    assert!(
        !expected_entries.is_empty(),
        "No entries found in binaries/CHECKSUMS.txt"
    );

    for (rel_path, expected_hash) in &expected_entries {
        let file_path = root.join(rel_path);
        assert!(
            file_path.exists(),
            "Binary file listed in CHECKSUMS.txt does not exist: {}",
            file_path.display()
        );
        let actual_hash = compute_binary_sha256(&file_path);
        assert_eq!(
            actual_hash,
            *expected_hash,
            "SHA-256 checksum mismatch for binary {}",
            file_path.display()
        );
    }

    let binaries_dir = root.join("binaries");
    let mut found_files = Vec::new();
    collect_files_recursive(&binaries_dir, &mut found_files);

    for file in found_files {
        if file.file_name().and_then(|n| n.to_str()) == Some(".DS_Store") {
            continue;
        }
        if file.file_name().and_then(|n| n.to_str()) == Some("CHECKSUMS.txt") {
            continue;
        }
        let rel_path = file
            .strip_prefix(&root)
            .expect("File under root")
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            expected_entries.contains_key(&rel_path),
            "Binary file {} is not recorded in binaries/CHECKSUMS.txt",
            rel_path
        );
    }
}

fn collect_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursive(&path, files);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
}
