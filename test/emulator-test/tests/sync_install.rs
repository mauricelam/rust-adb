use emulator_test::harness::AdbTestHarness;
use anyhow::Result;
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_sync_push_pull() -> Result<()> {
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    let dir = tempdir()?;
    let file_path = dir.path().join("test.txt");
    let mut file = File::create(&file_path)?;
    file.write_all(b"Hello ADB!")?;

    let remote_path = "/data/local/tmp/test.txt";

    // Test push
    harness.run_parity(&["push", file_path.to_str().unwrap(), remote_path])?;

    // Test pull
    let pull_path = dir.path().join("pulled.txt");
    harness.run_parity(&["pull", remote_path, pull_path.to_str().unwrap()])?;

    Ok(())
}

#[test]
fn test_recursive_sync() -> Result<()> {
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    let dir = tempdir()?;
    let subdir = dir.path().join("subdir");
    std::fs::create_dir(&subdir)?;
    File::create(subdir.join("f1.txt"))?.write_all(b"1")?;
    File::create(subdir.join("f2.txt"))?.write_all(b"2")?;

    let remote_dir = "/data/local/tmp/sync_test";
    harness.run_rust(&["shell", "rm", "-rf", remote_dir])?;

    // Test recursive push
    harness.run_parity(&["push", dir.path().to_str().unwrap(), remote_dir])?;

    Ok(())
}
