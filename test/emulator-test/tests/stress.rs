use emulator_test::harness::AdbTestHarness;
use anyhow::Result;
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;
use std::thread;

#[test]
fn test_concurrent_shells() -> Result<()> {
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    let mut handles = vec![];
    for i in 0..10 {
        let serial_clone = serial.clone();
        let handle = thread::spawn(move || {
            let h = AdbTestHarness::new(&serial_clone).unwrap();
            let out = h.run_rust(&["shell", &format!("echo {}; sleep 0.1", i)])?;
            if !out.status.success() {
                eprintln!("Shell {} failed: {}", i, String::from_utf8_lossy(&out.stderr));
            }
            Ok(out.status.success())
        });
        handles.push(handle);
    }

    for handle in handles {
        let res: Result<bool> = handle.join().unwrap();
        assert!(res?);
    }

    Ok(())
}

#[test]
fn test_large_data_transfer() -> Result<()> {
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    let dir = tempdir()?;
    let file_path = dir.path().join("large_file.bin");
    let mut file = File::create(&file_path)?;

    // 100MB of data
    let data = vec![0u8; 100 * 1024 * 1024];
    file.write_all(&data)?;

    let remote_path = "/data/local/tmp/large_file.bin";

    // Push large file
    harness.run_rust(&["push", file_path.to_str().unwrap(), remote_path])?;

    // Pull large file back
    let pull_path = dir.path().join("large_pulled.bin");
    harness.run_rust(&["pull", remote_path, pull_path.to_str().unwrap()])?;

    Ok(())
}
