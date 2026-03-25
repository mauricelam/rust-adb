use std::process::{Command, Output};
use anyhow::{Result, anyhow};
use std::env;
use std::path::PathBuf;

pub struct AdbTestHarness {
    pub rust_adb_path: PathBuf,
    pub original_adb_path: PathBuf,
    pub serial: String,
}

impl AdbTestHarness {
    pub fn new(serial: &str) -> Result<Self> {
        let root = env::current_dir()?;
        // Since tests run from test/emulator-test, we need to go up two levels to reach the root
        let rust_adb_path = root.join("../../rust/target/debug/rust-adb");
        let original_adb_path = root.join("../../binaries/linux/adb");

        Ok(Self {
            rust_adb_path,
            original_adb_path,
            serial: serial.to_string(),
        })
    }

    pub fn run_parity(&self, args: &[&str]) -> Result<()> {
        let rust_output = self.run_adb(&self.rust_adb_path, args)?;
        let original_output = self.run_adb(&self.original_adb_path, args)?;

        if rust_output.status != original_output.status {
            return Err(anyhow!(
                "Status mismatch: rust={:?}, original={:?}\nArgs: {:?}\nRust Stdout: {}\nRust Stderr: {}\nOrig Stdout: {}\nOrig Stderr: {}",
                rust_output.status,
                original_output.status,
                args,
                String::from_utf8_lossy(&rust_output.stdout),
                String::from_utf8_lossy(&rust_output.stderr),
                String::from_utf8_lossy(&original_output.stdout),
                String::from_utf8_lossy(&original_output.stderr),
            ));
        }

        // Output normalization: Some commands include time-varying output (like bugreportz)
        // For general commands, we expect identical stdout/stderr.
        if rust_output.stdout != original_output.stdout {
            return Err(anyhow!(
                "Stdout mismatch!\nArgs: {:?}\nRust: {}\nOrig: {}",
                args,
                String::from_utf8_lossy(&rust_output.stdout),
                String::from_utf8_lossy(&original_output.stdout)
            ));
        }

        Ok(())
    }

    fn run_adb(&self, adb_path: &PathBuf, args: &[&str]) -> Result<Output> {
        let output = Command::new(adb_path)
            .args(["-s", &self.serial])
            .args(args)
            .output()?;
        Ok(output)
    }

    pub fn run_rust(&self, args: &[&str]) -> Result<Output> {
        self.run_adb(&self.rust_adb_path, args)
    }

    pub fn run_original(&self, args: &[&str]) -> Result<Output> {
        self.run_adb(&self.original_adb_path, args)
    }
}
