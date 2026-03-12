use std::io::{Read, Write, self};
use crate::adb_client::{adb_connect, format_host_command};
use adb_utils::escape_arg;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

/// Displays device log.
pub fn adb_logcat(args: &[String], longcat: bool) -> anyhow::Result<()> {
    let log_tags = std::env::var("ANDROID_LOG_TAGS").unwrap_or_default();
    let quoted = escape_arg(&log_tags);

    let mut cmd = format!("export ANDROID_LOG_TAGS={}; exec logcat", quoted);

    if longcat {
        cmd.push_str(" -v long");
    }

    for arg in args {
        cmd.push_str(&format!(" {}", escape_arg(arg)));
    }

    let service = format_host_command(&format!("shell:{}", cmd));
    let (fd, _) = adb_connect(&service, false)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut stdout = io::stdout();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 { break; }
        stdout.write_all(&buf[..n])?;
        stdout.flush()?;
    }

    Ok(())
}
