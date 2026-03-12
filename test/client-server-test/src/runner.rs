use std::process::{Command, Output};

fn get_adb_path() -> String {
    if let Ok(path) = std::env::var("ADB_BINARY") {
        return path;
    }
    let path = {
        #[cfg(target_os = "linux")]
        {
            "../../binaries/linux/adb"
        }
        #[cfg(target_os = "macos")]
        {
            "../../binaries/mac/adb"
        }
        #[cfg(target_os = "windows")]
        {
            "../../binaries/win/adb.exe"
        }
    };
    path.to_string()
}

pub fn run_adb_command(port: u16, args: &[&str]) -> std::io::Result<Output> {
    Command::new(get_adb_path())
        .env("ADB_NOSERVER", "1")
        .args(["-P", &port.to_string()])
        .args(args)
        .output()
}

pub fn spawn_adb_command(port: u16, args: &[&str]) -> std::io::Result<std::process::Child> {
    Command::new(get_adb_path())
        .env("ADB_NOSERVER", "1")
        .args(["-P", &port.to_string()])
        .args(args)
        .spawn()
}
