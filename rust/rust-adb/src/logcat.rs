use crate::adb_client::{adb_connect, format_host_command};
use adb_protocol::shell_protocol::{ShellId, ShellProtocol};
use adb_socket_spec::NativeOwnedHandle;
use adb_utils::escape_arg;
use anyhow::Result;
use std::io::{self, Read, Write};

/// Represents the logcat command.
pub struct Logcat;

impl Logcat {
    /// Executes the logcat command.
    pub fn do_it(args: &[String], is_longcat: bool) -> Result<()> {
        let mut real = LogcatReal {};
        Self::do_it_internal(&mut real, args, is_longcat)
    }

    /// Internal implementation of the logcat command.
    pub fn do_it_internal<T: LogcatInternal>(
        internal: &mut T,
        args: &[String],
        is_longcat: bool,
    ) -> Result<()> {
        let log_tags = std::env::var("ANDROID_LOG_TAGS").unwrap_or_default();
        let quoted = escape_arg(&log_tags);

        let mut cmd = format!("export ANDROID_LOG_TAGS={}; exec logcat", quoted);

        if is_longcat {
            cmd.push_str(" -v long");
        }

        for arg in args {
            cmd.push_str(" ");
            cmd.push_str(&escape_arg(arg));
        }

        internal.run_logcat_command(&cmd)
    }
}

/// Internal trait for logcat operations, allowing for mocking in tests.
pub trait LogcatInternal {
    /// Runs the constructed logcat command on the device.
    fn run_logcat_command(&mut self, command: &str) -> Result<()>;
}

struct LogcatReal;

impl LogcatInternal for LogcatReal {
    fn run_logcat_command(&mut self, command: &str) -> Result<()> {
        let service = format_host_command(&format!("shell,v2,raw:{}", command));
        let (fd, _) = adb_connect(&service, false)?;
        let mut stream = stream_from_fd(fd);

        let mut sp = ShellProtocol::new();
        while sp.read(&mut stream)? {
            match sp.id {
                ShellId::Stdout => {
                    io::stdout().write_all(&sp.data)?;
                    io::stdout().flush()?;
                }
                ShellId::Stderr => {
                    io::stderr().write_all(&sp.data)?;
                    io::stderr().flush()?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWriteSend for T {}

fn stream_from_fd(fd: NativeOwnedHandle) -> Box<dyn ReadWriteSend> {
    #[cfg(unix)]
    {
        use std::os::unix::io::{FromRawFd, IntoRawFd};
        Box::new(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::{FromRawSocket, IntoRawSocket};
        Box::new(unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) })
    }
}

#[cfg(windows)]
use std::os::windows::io::IntoRawSocket;

#[cfg(test)]
mod tests {
    use super::*;

    struct LogcatMock {
        last_command: String,
    }

    impl LogcatMock {
        fn new() -> Self {
            Self {
                last_command: String::new(),
            }
        }
    }

    impl LogcatInternal for LogcatMock {
        fn run_logcat_command(&mut self, command: &str) -> Result<()> {
            self.last_command = command.to_string();
            Ok(())
        }
    }

    #[test]
    fn test_logcat_no_args() {
        let mut mock = LogcatMock::new();
        std::env::remove_var("ANDROID_LOG_TAGS");
        Logcat::do_it_internal(&mut mock, &[], false).unwrap();
        assert_eq!(mock.last_command, "export ANDROID_LOG_TAGS=''; exec logcat");
    }

    #[test]
    fn test_logcat_with_tags() {
        let mut mock = LogcatMock::new();
        std::env::set_var("ANDROID_LOG_TAGS", "ActivityManager:I *:S");
        Logcat::do_it_internal(&mut mock, &[], false).unwrap();
        assert_eq!(
            mock.last_command,
            "export ANDROID_LOG_TAGS='ActivityManager:I *:S'; exec logcat"
        );
    }

    #[test]
    fn test_logcat_with_args() {
        let mut mock = LogcatMock::new();
        std::env::remove_var("ANDROID_LOG_TAGS");
        Logcat::do_it_internal(&mut mock, &["-b".to_string(), "radio".to_string()], false).unwrap();
        assert_eq!(
            mock.last_command,
            "export ANDROID_LOG_TAGS=''; exec logcat '-b' 'radio'"
        );
    }

    #[test]
    fn test_longcat() {
        let mut mock = LogcatMock::new();
        std::env::remove_var("ANDROID_LOG_TAGS");
        Logcat::do_it_internal(&mut mock, &[], true).unwrap();
        assert_eq!(
            mock.last_command,
            "export ANDROID_LOG_TAGS=''; exec logcat -v long"
        );
    }

    #[test]
    fn test_logcat_complex_escape() {
        let mut mock = LogcatMock::new();
        std::env::set_var("ANDROID_LOG_TAGS", "a'b");
        Logcat::do_it_internal(&mut mock, &["-m".to_string(), "10'0".to_string()], false).unwrap();
        assert_eq!(
            mock.last_command,
            "export ANDROID_LOG_TAGS='a'''b'; exec logcat '-m' '10'''0'"
        );
    }
}
