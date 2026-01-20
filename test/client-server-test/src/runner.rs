use std::process::{Command, Output};

const ADB_PATH: &str = {
    #[cfg(target_os = "linux")]
    {
        "../../binaries/linux/adb"
    }
    #[cfg(target_os = "macos")]
    {
        "../../binaries/mac/adb"
    }
};

pub fn run_adb_command(port: u16, args: &[&str]) -> std::io::Result<Output> {
    Command::new(ADB_PATH)
        .args(["-P", &port.to_string()])
        .args(args)
        .output()
}

pub fn spawn_adb_command(port: u16, args: &[&str]) -> std::io::Result<std::process::Child> {
    Command::new(ADB_PATH)
        .args(["-P", &port.to_string()])
        .args(args)
        .spawn()
}
