use std::process::{Command, Child, Stdio};
use std::time::{Duration, Instant};
use std::io::{BufRead, BufReader};
use anyhow::{Result, anyhow};
use std::env;

pub mod harness;

pub struct EmulatorGuard {
    process: Child,
    adb_path: String,
    port: u16,
}

impl EmulatorGuard {
    pub fn new(avd_name: &str, adb_path: &str, port: u16) -> Result<Self> {
        let emulator_path = env::var("ANDROID_HOME")
            .map(|home| format!("{}/emulator/emulator", home))
            .unwrap_or_else(|_| "emulator".to_string());

        println!("Starting emulator for AVD: {}", avd_name);
        let process = Command::new(emulator_path)
            .args(["-avd", avd_name, "-no-window", "-no-audio", "-port", &port.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut guard = Self {
            process,
            adb_path: adb_path.to_string(),
            port,
        };

        guard.wait_for_boot()?;
        Ok(guard)
    }

    fn wait_for_boot(&mut self) -> Result<()> {
        let timeout = Duration::from_secs(300); // 5 minutes
        let start = Instant::now();
        let serial = format!("emulator-{}", self.port);

        println!("Waiting for emulator {} to boot...", serial);

        while start.elapsed() < timeout {
            let output = Command::new(&self.adb_path)
                .args(["-s", &serial, "shell", "getprop", "sys.boot_completed"])
                .output();

            if let Ok(out) = output {
                let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if status == "1" {
                    println!("Emulator {} is ready.", serial);
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        }

        Err(anyhow!("Timeout waiting for emulator boot"))
    }
}

impl Drop for EmulatorGuard {
    fn drop(&mut self) {
        let serial = format!("emulator-{}", self.port);
        println!("Shutting down emulator {}...", serial);
        let _ = Command::new(&self.adb_path)
            .args(["-s", &serial, "emu", "kill"])
            .status();

        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
