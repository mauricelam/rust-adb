use rust_adb::{adb_query, adb_connect};
use std::process::{Command, Child, Stdio};
use std::env;
use std::time::{Duration, Instant};
use std::thread;
use std::io::Read;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

struct EmulatorGuard {
    child: Option<Child>,
}

impl EmulatorGuard {
    fn new() -> Self {
        let android_home = env::var("ANDROID_HOME");

        // Ensure ADB server is running
        let adb_path = if let Ok(ref home) = android_home {
            let mut p = std::path::PathBuf::from(home);
            p.push("platform-tools");
            p.push("adb");
            p
        } else {
            std::path::PathBuf::from("adb")
        };
        let _ = Command::new(adb_path).arg("start-server").status();

        if is_emulator_reachable() {
            println!("Emulator already reachable.");
            return Self { child: None };
        }

        println!("Starting emulator...");
        let android_home = env::var("ANDROID_HOME").expect("ANDROID_HOME not set");
        let avd_name = env::var("RS_ADB_AVD_NAME").unwrap_or_else(|_| "test".to_string());

        // Use $ANDROID_HOME/emulator/emulator
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

fn is_emulator_reachable() -> bool {
    // Poll adb devices until status is device
    match adb_query("host:devices") {
        Ok(devices) => {
            devices.contains("emulator-5554") && devices.contains("\tdevice")
        }
        Err(_) => false,
    }
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
        let devices = adb_query("host:devices").expect("Failed to query devices");
        assert!(devices.contains("emulator-5554"), "Device list should contain emulator-5554. Output: {}", devices);
        assert!(devices.contains("\tdevice"), "Device status should be 'device'. Output: {}", devices);
    }

    // Test 2: Shell Protocol Verification
    {
        println!("Running Test 2: Shell Protocol Verification");
        // Execute adb shell echo hi
        let (handle, _) = adb_connect("shell:echo hi", false).expect("Failed to connect to shell");

        #[cfg(unix)]
        let mut stream = unsafe { std::fs::File::from_raw_fd(handle.into_raw_fd()) };
        #[cfg(windows)]
        let mut stream = unsafe { std::net::TcpStream::from_raw_socket(handle.into_raw_socket() as _) };

        let mut output = String::new();
        stream.read_to_string(&mut output).expect("Failed to read from shell stream");

        // Handle potential cross-platform line ending issues and assert it's exactly "hi"
        assert_eq!(output.trim(), "hi", "Shell output should be 'hi'");
    }
}
