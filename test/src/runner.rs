use std::process::{Command, Output};

pub fn run_adb_command(port: u16, args: &[&str]) -> std::io::Result<Output> {
    // Path to the adb binary, relative to the workspace root
    #[cfg(target_os = "linux")]
    let adb_path = "../binaries/linux/adb";
    #[cfg(target_os = "macos")]
    let adb_path = "../binaries/mac/adb";

    Command::new(adb_path)
        .args(args)
        .env("ADB_SERVER_PORT", port.to_string())
        .output()
}
