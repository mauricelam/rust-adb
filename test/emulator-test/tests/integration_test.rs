use std::process::{Command, Child, Stdio, Output};
use std::env;
use std::time::{Duration, Instant};
use std::thread;
use std::io::Read;

struct EmulatorGuard {
    child: Option<Child>,
}

impl EmulatorGuard {
    fn new() -> Self {
        // Ensure ADB server is running
        let adb_path = get_adb_path();
        println!("Using adb path: {:?}", adb_path);
        let _ = Command::new(&adb_path).arg("start-server").status();

        if is_emulator_reachable() {
            println!("Emulator already reachable.");
            return Self { child: None };
        }

        println!("Starting emulator...");
        let android_home = env::var("ANDROID_HOME").expect("ANDROID_HOME not set");
        let avd_name = env::var("RS_ADB_AVD_NAME").unwrap_or_else(|_| "test".to_string());

        let mut emulator_path = std::path::PathBuf::from(android_home);
        emulator_path.push("emulator");
        emulator_path.push("emulator");

        let child = Command::new(emulator_path)
            .args(&["-avd", &avd_name, "-no-window", "-gpu", "libguestgl", "-no-audio"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Failed to start emulator");

        Self { child: Some(child) }
    }
}

impl Drop for EmulatorGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            println!("Terminating emulator process...");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn get_adb_path() -> std::path::PathBuf {
    if let Ok(adb_bin) = env::var("ADB_BINARY") {
        return std::path::PathBuf::from(adb_bin);
    }
    if let Ok(home) = env::var("ANDROID_HOME") {
        let mut p = std::path::PathBuf::from(home);
        p.push("platform-tools");
        p.push("adb");
        if p.exists() {
            return p;
        }
    }
    std::path::PathBuf::from("adb")
}

fn run_adb(args: &[&str]) -> Output {
    Command::new(get_adb_path())
        .args(args)
        .output()
        .expect("Failed to execute adb")
}

fn is_emulator_reachable() -> bool {
    let output = run_adb(&["devices"]);
    let devices = String::from_utf8_lossy(&output.stdout);
    devices.contains("emulator-5554") && devices.contains("\tdevice")
}

fn wait_for_boot() {
    let timeout = Duration::from_secs(300); // 5 minutes for CI
    let start = Instant::now();

    while start.elapsed() < timeout {
        if is_emulator_reachable() {
            println!("Emulator is ready!");
            return;
        }
        thread::sleep(Duration::from_secs(5));
    }
    panic!("Timeout waiting for emulator to boot after 5 minutes");
}

#[test]
fn test_adb_integration() {
    // Setup: Check for emulator or start it
    let _guard = EmulatorGuard::new();

    // Readiness: Wait for boot
    wait_for_boot();

    // Test 1: Device Enumeration
    {
        println!("Running Test 1: Device Enumeration");
        let output = run_adb(&["devices"]);
        let devices = String::from_utf8_lossy(&output.stdout);
        assert!(devices.contains("emulator-5554"), "Device list should contain emulator-5554. Output: {}", devices);
        assert!(devices.contains("\tdevice"), "Device status should be 'device'. Output: {}", devices);
    }

    // Test 2: Shell Protocol Verification
    {
        println!("Running Test 2: Shell Protocol Verification");
        // Execute adb shell echo hi
        let output = run_adb(&["-s", "emulator-5554", "shell", "echo", "hi"]);
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Handle potential cross-platform line ending issues and assert it's exactly "hi"
        assert_eq!(stdout.trim(), "hi", "Shell output should be 'hi'");
    }
}
