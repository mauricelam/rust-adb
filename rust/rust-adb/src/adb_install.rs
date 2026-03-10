use crate::adb_client::{adb_connect, adb_query, format_host_command};
use adb_transport::{string_to_feature_set, FEATURE_CMD, FeatureSet};
use adb_utils::escape_arg;
use anyhow::{anyhow, Result};
use std::io::{self, Read, Write};
use std::path::Path;

/// Retrieves the set of features supported by the connected device.
pub fn get_device_features() -> Result<FeatureSet> {
    let query = format_host_command("features");
    let features_str = adb_query(&query)?;
    Ok(string_to_feature_set(&features_str))
}

/// Determines if the device supports modern streamed installation (via `cmd`).
pub fn best_install_mode(features: &FeatureSet) -> bool {
    features.iter().any(|f| f == FEATURE_CMD)
}

/// Uninstalls an application from the device.
pub fn uninstall_app(args: &[String]) -> Result<()> {
    let features = get_device_features()?;
    let use_cmd = best_install_mode(&features);

    let mut cmd = if use_cmd {
        "shell:cmd package uninstall".to_string()
    } else {
        "shell:pm uninstall".to_string()
    };

    for arg in args {
        cmd.push(' ');
        cmd.push_str(&escape_arg(arg));
    }

    let service = format_host_command(&cmd);
    let (fd, _) = adb_connect(&service, false)?;

    let mut stream = stream_from_fd(fd);
    io::copy(&mut stream, &mut io::stdout())?;

    Ok(())
}

/// Installs an application APK or APEX using the streamed install protocol.
pub fn install_app_streamed(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Err(anyhow!("install requires an apk argument"));
    }

    let file_path = &args[args.len() - 1];
    let path = Path::new(file_path);
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if extension != "apk" && extension != "apex" {
        return Err(anyhow!("filename doesn't end .apk or .apex: {}", file_path));
    }

    let metadata = std::fs::metadata(path)?;
    let size = metadata.len();

    let features = get_device_features()?;
    let use_cmd = best_install_mode(&features);

    let mut cmd = if use_cmd {
        "exec:cmd package install".to_string()
    } else {
        "exec:pm install".to_string()
    };

    // Add arguments except the last one (the file path)
    for arg in &args[..args.len() - 1] {
        cmd.push(' ');
        cmd.push_str(&escape_arg(arg));
    }

    cmd.push_str(&format!(" -S {}", size));

    if extension == "apex" {
        cmd.push_str(" --apex");
    }

    let service = format_host_command(&cmd);
    let (fd, _) = adb_connect(&service, false)?;
    let mut stream = stream_from_fd(fd);

    let mut file = std::fs::File::open(path)?;
    io::copy(&mut file, &mut stream)?;

    // We should probably shutdown the write side here if possible, but TcpStream doesn't easily expose it when wrapped.
    // However, adb_status might work if the server sends it after receiving all data.

    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 { break; }
        io::stdout().write_all(&buf[..n])?;
    }

    Ok(())
}

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};
use adb_socket_spec::NativeOwnedHandle;

fn stream_from_fd(fd: NativeOwnedHandle) -> Box<dyn ReadWriteSend> {
    #[cfg(unix)]
    {
        Box::new(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
    }
    #[cfg(windows)]
    {
        Box::new(unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) })
    }
}

trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWriteSend for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_features(has_cmd: bool) -> FeatureSet {
        if has_cmd {
            vec!["cmd".to_string(), "shell_v2".to_string()]
        } else {
            vec!["shell_v2".to_string()]
        }
    }

    fn generate_uninstall_cmd(args: &[String], features: &FeatureSet) -> String {
        let use_cmd = best_install_mode(features);
        let mut cmd = if use_cmd {
            "shell:cmd package uninstall".to_string()
        } else {
            "shell:pm uninstall".to_string()
        };
        for arg in args {
            cmd.push(' ');
            cmd.push_str(&escape_arg(arg));
        }
        cmd
    }

    fn generate_install_cmd(args: &[String], size: u64, is_apex: bool, features: &FeatureSet) -> String {
        let use_cmd = best_install_mode(features);
        let mut cmd = if use_cmd {
            "exec:cmd package install".to_string()
        } else {
            "exec:pm install".to_string()
        };
        for arg in &args[..args.len() - 1] {
            cmd.push(' ');
            cmd.push_str(&escape_arg(arg));
        }
        cmd.push_str(&format!(" -S {}", size));
        if is_apex {
            cmd.push_str(" --apex");
        }
        cmd
    }

    #[test]
    fn test_uninstall_command_generation() {
        let args = vec!["com.example.app".to_string()];

        let cmd_modern = generate_uninstall_cmd(&args, &mock_features(true));
        assert_eq!(cmd_modern, "shell:cmd package uninstall 'com.example.app'");

        let cmd_legacy = generate_uninstall_cmd(&args, &mock_features(false));
        assert_eq!(cmd_legacy, "shell:pm uninstall 'com.example.app'");
    }

    #[test]
    fn test_install_command_generation() {
        let args = vec!["-r".to_string(), "test.apk".to_string()];
        let size = 12345;

        let cmd_modern = generate_install_cmd(&args, size, false, &mock_features(true));
        assert_eq!(cmd_modern, "exec:cmd package install '-r' -S 12345");

        let cmd_legacy = generate_install_cmd(&args, size, false, &mock_features(false));
        assert_eq!(cmd_legacy, "exec:pm install '-r' -S 12345");

        let cmd_apex = generate_install_cmd(&args, size, true, &mock_features(true));
        assert_eq!(cmd_apex, "exec:cmd package install '-r' -S 12345 --apex");
    }

    #[test]
    fn test_best_install_mode() {
        let features_with_cmd = vec!["cmd".to_string(), "shell_v2".to_string()];
        assert!(best_install_mode(&features_with_cmd));

        let features_without_cmd = vec!["shell_v2".to_string()];
        assert!(!best_install_mode(&features_without_cmd));
    }
}
