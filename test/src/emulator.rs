use anyhow::{anyhow, Result};
use cmd_lib::{run_fun, CmdChildren};
use std::thread;
use std::time::Duration;

pub struct EmulatorGuard {
    children: CmdChildren,
    port: u16,
}

impl EmulatorGuard {
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for EmulatorGuard {
    fn drop(&mut self) {
        let _ = self.children.kill();
        let _ = self.children.wait();
    }
}

pub fn start_and_get_port() -> Result<EmulatorGuard> {
    // Start the emulator in the background.
    let children = cmd_lib::spawn!(emulator -avd test_avd -no-window -read-only)?;

    // Wait for the emulator to boot, and get its port.
    for _ in 0..60 {
        if let Ok(output) = run_fun!(adb devices) {
            for line in output.lines() {
                if line.starts_with("emulator-") {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() == 2 && parts[1] == "device" {
                        let device_name = parts[0];
                        let port_str = device_name.replace("emulator-", "");
                        if let Ok(port) = port_str.parse::<u16>() {
                            return Ok(EmulatorGuard { children, port });
                        }
                    }
                }
            }
        }
        thread::sleep(Duration::from_secs(1));
    }

    Err(anyhow!("Could not find booted emulator"))
}
