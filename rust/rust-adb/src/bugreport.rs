use crate::adb_client::{adb_connect, format_host_command};
use adb_protocol::shell_protocol::{ShellId, ShellProtocol};
use adb_services::file_sync_client::SyncConnection;
use adb_socket_spec::NativeOwnedHandle;
use anyhow::{anyhow, Result};
use std::io::{self, Read, Write};
use std::path::Path;
/// Represents the bugreport command.
/// Ported from `bugreport.cpp`.
pub struct Bugreport {
    /// Whether to show progress updates.
    pub show_progress: bool,
}

impl Bugreport {
    /// Creates a new `Bugreport` instance.
    pub fn new() -> Self {
        Self {
            show_progress: true,
        }
    }

    /// Executes the bugreport command.
    pub fn do_it(&mut self, args: &[String]) -> Result<()> {
        let mut br_real = BugreportReal {};
        self.do_it_internal(&mut br_real, args)
    }

    /// Internal implementation of the bugreport command.
    pub fn do_it_internal<T: BugreportInternal>(&mut self, br: &mut T, args: &[String]) -> Result<()> {
        if args.len() > 1 {
            return Err(anyhow!("usage: adb bugreport [[PATH] | [--stream]]"));
        }

        let (bugz_stdout, bugz_stderr) = br.send_shell_command("bugreportz -v", false)?;
        let bugz_version = bugz_stderr.trim();
        let bugz_output = bugz_stdout.trim();

        if bugz_version.is_empty() {
            if args.is_empty() {
                eprintln!(
                    "Failed to get bugreportz version, which is only available on devices                     running Android 7.0 or later.\nTrying a plain-text bug report instead."
                );
                return br.send_legacy_bugreport();
            }

            return Err(anyhow!(
                "Failed to get bugreportz version: 'bugreportz -v' returned '{}'.\n                If the device does not run Android 7.0 or above, try this instead:\n                \tadb bugreport > bugreport.txt",
                bugz_output
            ));
        }

        let mut version_parts = bugz_version.split('.');
        let major: i32 = version_parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor: i32 = version_parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        let mut dest_file = String::new();
        let mut dest_dir = String::new();

        if args.is_empty() {
            dest_dir = std::env::current_dir()?.to_string_lossy().into_owned();
        } else if args[0] == "--stream" {
            if major == 1 && minor < 2 {
                return Err(anyhow!("Failed to stream bugreport: bugreportz does not support stream."));
            } else {
                return br.send_shell_command_to_stdout("bugreportz -s", false);
            }
        } else {
            let path = Path::new(&args[0]);
            if path.is_dir() {
                dest_dir = args[0].clone();
            } else {
                dest_file = args[0].clone();
            }
        }

        if dest_file.is_empty() {
            dest_file = "bugreport.zip".to_string();
        } else if !dest_file.to_lowercase().ends_with(".zip") {
            dest_file.push_str(".zip");
        }

        let mut show_progress = true;
        let mut bugz_command = "bugreportz -p".to_string();
        if bugz_version == "1.0" {
            eprintln!(
                "Bugreport is in progress and it could take minutes to complete.\n                Please be patient and do not cancel or disconnect your device                 until it completes."
            );
            show_progress = false;
            bugz_command = "bugreportz".to_string();
        }

        self.show_progress = show_progress;

        let mut reader = br.open_shell_stream(&bugz_command, false)?;
        let mut line = String::new();
        let mut src_file = String::new();
        let mut final_dest_file = dest_file.to_string();
        let mut last_progress = -1;

        let mut sp = ShellProtocol::new();
        while sp.read(&mut reader)? {
            if sp.id == ShellId::Stdout {
                for &c in &sp.data {
                    if c == b'\n' {
                        if line.starts_with("BEGIN:") {
                            src_file = line["BEGIN:".len()..].to_string();
                            if !dest_dir.is_empty() {
                                final_dest_file = Path::new(&src_file).file_name().unwrap().to_string_lossy().into_owned();
                            }
                        } else if line.starts_with("OK:") {
                            src_file = line["OK:".len()..].to_string();
                            if !dest_dir.is_empty() {
                                final_dest_file = Path::new(&src_file).file_name().unwrap().to_string_lossy().into_owned();
                            }
                        } else if line.starts_with("FAIL:") {
                            return Err(anyhow!("adb: device failed to take a zipped bugreport: {}", &line["FAIL:".len()..]));
                        } else if self.show_progress && line.starts_with("PROGRESS:") {
                            let parts: Vec<&str> = line["PROGRESS:".len()..].split('/').collect();
                            if parts.len() == 2 {
                                if let (Ok(progress), Ok(total)) = (parts[0].parse::<i32>(), parts[1].parse::<i32>()) {
                                    let percentage = progress * 100 / total;
                                    if percentage >= 0 && (percentage == 0 || percentage > last_progress) {
                                        br.update_progress(&format!("generating {}", final_dest_file), percentage);
                                        last_progress = percentage;
                                    }
                                }
                            }
                        }
                        line.clear();
                    } else {
                        line.push(c as char);
                    }
                }
            } else if sp.id == ShellId::Stderr {
                io::stderr().write_all(&sp.data)?;
            }
        }

        if src_file.is_empty() {
            return Err(anyhow!("bugreportz did not return a 'OK:' or 'FAIL:' line"));
        }

        let destination = if dest_dir.is_empty() {
            final_dest_file.clone()
        } else {
            Path::new(&dest_dir).join(&final_dest_file).to_string_lossy().into_owned()
        };

        br.do_sync_pull(&src_file, &destination, &format!("pulling {}", final_dest_file))?;
        println!("Bug report copied to {}", destination);

        Ok(())
    }
}

/// Internal trait for bugreport operations, allowing for mocking in tests.
pub trait BugreportInternal {
    /// Sends a shell command and returns the output (stdout, stderr).
    fn send_shell_command(&mut self, command: &str, disable_shell_protocol: bool) -> Result<(String, String)>;
    /// Sends a shell command directly to stdout.
    fn send_shell_command_to_stdout(&mut self, command: &str, disable_shell_protocol: bool) -> Result<()>;
    /// Opens a shell stream for reading.
    fn open_shell_stream(&mut self, command: &str, disable_shell_protocol: bool) -> Result<Box<dyn Read + Send>>;
    /// Pulls a file from the device using the sync protocol.
    fn do_sync_pull(&mut self, src: &str, dst: &str, name: &str) -> Result<bool>;
    /// Updates the progress of the bugreport.
    fn update_progress(&mut self, message: &str, percentage: i32);
    /// Sends a legacy plain-text bugreport.
    fn send_legacy_bugreport(&mut self) -> Result<()>;
}

struct BugreportReal;

impl BugreportInternal for BugreportReal {
    fn send_shell_command(&mut self, command: &str, _disable_shell_protocol: bool) -> Result<(String, String)> {
        let service = format_host_command(&format!("shell,v2,raw:{}", command));
        let (fd, _) = adb_connect(&service, false)?;
        let mut stream = stream_from_fd(fd);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut sp = ShellProtocol::new();
        while sp.read(&mut stream)? {
            match sp.id {
                ShellId::Stdout => stdout.extend_from_slice(&sp.data),
                ShellId::Stderr => stderr.extend_from_slice(&sp.data),
                _ => {}
            }
        }
        Ok((String::from_utf8_lossy(&stdout).into_owned(), String::from_utf8_lossy(&stderr).into_owned()))
    }

    fn send_shell_command_to_stdout(&mut self, command: &str, _disable_shell_protocol: bool) -> Result<()> {
        let service = format_host_command(&format!("shell,v2,raw:{}", command));
        let (fd, _) = adb_connect(&service, false)?;
        let mut stream = stream_from_fd(fd);

        let mut sp = ShellProtocol::new();
        while sp.read(&mut stream)? {
            match sp.id {
                ShellId::Stdout => io::stdout().write_all(&sp.data)?,
                ShellId::Stderr => io::stderr().write_all(&sp.data)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn open_shell_stream(&mut self, command: &str, _disable_shell_protocol: bool) -> Result<Box<dyn Read + Send>> {
        let service = format_host_command(&format!("shell,v2,raw:{}", command));
        let (fd, _) = adb_connect(&service, false)?;
        Ok(Box::new(stream_from_fd(fd)))
    }

    fn do_sync_pull(&mut self, src: &str, dst: &str, _name: &str) -> Result<bool> {
        let (fd, _) = adb_connect("sync:", false)?;
        let mut sync = SyncConnection::new(fd);
        sync.pull(src, dst)?;
        Ok(true)
    }

    fn update_progress(&mut self, message: &str, percentage: i32) {
        eprint!("\r[{:>3}%] {}", percentage, message);
        io::stderr().flush().unwrap();
        if percentage == 100 {
            eprintln!();
        }
    }

    fn send_legacy_bugreport(&mut self) -> Result<()> {
        let service = format_host_command("shell:bugreport");
        let (fd, _) = adb_connect(&service, false)?;
        let mut stream = stream_from_fd(fd);
        io::copy(&mut stream, &mut io::stdout())?;
        Ok(())
    }
}

trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWriteSend for T {}

fn stream_from_fd(fd: NativeOwnedHandle) -> Box<dyn ReadWriteSend> {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        Box::new(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::FromRawSocket;
        Box::new(unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) })
    }
}

#[cfg(unix)]
use std::os::unix::io::IntoRawFd;
#[cfg(windows)]
use std::os::windows::io::IntoRawSocket;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct BugreportMock {
        shell_commands: HashMap<String, (i32, String, String)>,
        shell_streams: HashMap<String, Vec<(ShellId, String)>>,
        sync_pulls: Vec<(String, String, String)>,
        progress_updates: Vec<(String, i32)>,
        stdout_capture: Arc<Mutex<Vec<u8>>>,
        legacy_called: bool,
    }

    impl BugreportMock {
        fn new() -> Self {
            Self {
                shell_commands: HashMap::new(),
                shell_streams: HashMap::new(),
                sync_pulls: Vec::new(),
                progress_updates: Vec::new(),
                stdout_capture: Arc::new(Mutex::new(Vec::new())),
                legacy_called: false,
            }
        }

        fn expect_shell_command(&mut self, command: &str, status: i32, stdout: &str, stderr: &str) {
            self.shell_commands.insert(command.to_string(), (status, stdout.to_string(), stderr.to_string()));
        }

        fn expect_shell_stream(&mut self, command: &str, output: Vec<(ShellId, String)>) {
            self.shell_streams.insert(command.to_string(), output);
        }
    }

    impl BugreportInternal for BugreportMock {
        fn send_shell_command(&mut self, command: &str, _disable_shell_protocol: bool) -> Result<(String, String)> {
            if let Some((status, stdout, stderr)) = self.shell_commands.get(command) {
                if *status == 0 {
                    Ok((stdout.clone(), stderr.clone()))
                } else {
                    Err(anyhow!("command failed with status {}", status))
                }
            } else {
                Err(anyhow!("unexpected shell command: {}", command))
            }
        }

        fn send_shell_command_to_stdout(&mut self, command: &str, _disable_shell_protocol: bool) -> Result<()> {
             if let Some((status, stdout, _)) = self.shell_commands.get(command) {
                if *status == 0 {
                    self.stdout_capture.lock().unwrap().extend_from_slice(stdout.as_bytes());
                    Ok(())
                } else {
                    Err(anyhow!("command failed with status {}", status))
                }
            } else {
                Err(anyhow!("unexpected shell command to stdout: {}", command))
            }
        }

        fn open_shell_stream(&mut self, command: &str, _disable_shell_protocol: bool) -> Result<Box<dyn Read + Send>> {
            if let Some(output) = self.shell_streams.get(command) {
                let mut data = Vec::new();
                for (id, s) in output {
                    ShellProtocol::write_packet(&mut data, *id, s.as_bytes()).unwrap();
                }
                Ok(Box::new(io::Cursor::new(data)))
            } else {
                Err(anyhow!("unexpected shell stream: {}", command))
            }
        }

        fn do_sync_pull(&mut self, src: &str, dst: &str, name: &str) -> Result<bool> {
            self.sync_pulls.push((src.to_string(), dst.to_string(), name.to_string()));
            Ok(true)
        }

        fn update_progress(&mut self, message: &str, percentage: i32) {
            self.progress_updates.push((message.to_string(), percentage));
        }

        fn send_legacy_bugreport(&mut self) -> Result<()> {
            self.legacy_called = true;
            self.stdout_capture.lock().unwrap().extend_from_slice(b"Reported the bug was.");
            Ok(())
        }
    }

    #[test]
    fn test_invalid_number_args() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        let args = vec!["to".to_string(), "principal".to_string()];
        assert!(br.do_it_internal(&mut mock, &args).is_err());
    }

    #[test]
    fn test_no_arguments_pre_n_device() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        mock.expect_shell_command("bugreportz -v", 0, "Dude, where is my bugreportz?", "");
        let bugreport = "Reported the bug was.";

        assert!(br.do_it_internal(&mut mock, &[]).is_ok());
        assert!(mock.legacy_called);
        assert_eq!(String::from_utf8(mock.stdout_capture.lock().unwrap().clone()).unwrap(), bugreport);
    }

    #[test]
    fn test_no_arguments_n_device() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        mock.expect_shell_command("bugreportz -v", 0, "", "1.0");
        mock.expect_shell_stream("bugreportz", vec![(ShellId::Stdout, "OK:/device/da_bugreport.zip\n".to_string())]);

        assert!(br.do_it_internal(&mut mock, &[]).is_ok());
        assert_eq!(mock.sync_pulls.len(), 1);
        assert_eq!(mock.sync_pulls[0].0, "/device/da_bugreport.zip");
        assert!(mock.sync_pulls[0].1.ends_with("da_bugreport.zip"));
    }

    #[test]
    fn test_no_arguments_post_n_device() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        mock.expect_shell_command("bugreportz -v", 0, "", "1.1");
        mock.expect_shell_stream("bugreportz -p", vec![
            (ShellId::Stdout, "BEGIN:/device/da_bugreport.zip\n".to_string()),
            (ShellId::Stdout, "PROGRESS:50/100\n".to_string()),
            (ShellId::Stdout, "OK:/device/da_bugreport.zip\n".to_string()),
        ]);

        assert!(br.do_it_internal(&mut mock, &[]).is_ok());
        assert_eq!(mock.sync_pulls.len(), 1);
        assert_eq!(mock.sync_pulls[0].0, "/device/da_bugreport.zip");
        assert!(mock.sync_pulls[0].1.ends_with("da_bugreport.zip"));
        assert!(mock.progress_updates.iter().any(|p| p.1 == 50));
    }

    #[test]
    fn test_ok_n_device() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        mock.expect_shell_command("bugreportz -v", 0, "", "1.0");
        mock.expect_shell_stream("bugreportz", vec![(ShellId::Stdout, "OK:/device/bugreport.zip\n".to_string())]);

        assert!(br.do_it_internal(&mut mock, &["file.zip".to_string()]).is_ok());
        assert_eq!(mock.sync_pulls.len(), 1);
        assert_eq!(mock.sync_pulls[0].0, "/device/bugreport.zip");
        assert_eq!(mock.sync_pulls[0].1, "file.zip");
    }

    #[test]
    fn test_ok_progress() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        mock.expect_shell_command("bugreportz -v", 0, "", "1.1");
        mock.expect_shell_stream("bugreportz -p", vec![
            (ShellId::Stdout, "BEGIN:/device/bugreport___NOT.zip\n".to_string()),
            (ShellId::Stdout, "PROGRESS:1/100\n".to_string()),
            (ShellId::Stdout, "\nDUDE:SWEET\n\nBLA\n\nBLA\nBLA\n\n".to_string()),
            (ShellId::Stdout, "PROGRESS:10/100\nPROGRESS:50/100\n".to_string()),
            (ShellId::Stdout, "PROGRESS:99/100\n".to_string()),
            (ShellId::Stdout, "OK:/device/bugreport.zip\n".to_string()),
        ]);

        assert!(br.do_it_internal(&mut mock, &["file.zip".to_string()]).is_ok());
        assert_eq!(mock.sync_pulls.len(), 1);
        assert_eq!(mock.sync_pulls[0].0, "/device/bugreport.zip");
        assert_eq!(mock.sync_pulls[0].1, "file.zip");
        let percentages: Vec<i32> = mock.progress_updates.iter().map(|p| p.1).collect();
        assert!(percentages.contains(&1));
        assert!(percentages.contains(&10));
        assert!(percentages.contains(&50));
        assert!(percentages.contains(&99));
    }

     #[test]
    fn test_ok_progress_always_forward() {
        let mut br = Bugreport::new();
        let mut mock = BugreportMock::new();
        mock.expect_shell_command("bugreportz -v", 0, "", "1.1");
        mock.expect_shell_stream("bugreportz -p", vec![
            (ShellId::Stdout, "BEGIN:/device/bugreport.zip\n".to_string()),
            (ShellId::Stdout, "PROGRESS:1/100\n".to_string()),
            (ShellId::Stdout, "PROGRESS:50/100\n".to_string()),
            (ShellId::Stdout, "PROGRESS:25/100\n".to_string()),
            (ShellId::Stdout, "PROGRESS:75/100\n".to_string()),
            (ShellId::Stdout, "PROGRESS:75/100\n".to_string()),
            (ShellId::Stdout, "PROGRESS:700/1000\n".to_string()),
            (ShellId::Stdout, "OK:/device/bugreport.zip\n".to_string()),
        ]);

        assert!(br.do_it_internal(&mut mock, &["file.zip".to_string()]).is_ok());
        let percentages: Vec<i32> = mock.progress_updates.iter().map(|p| p.1).collect();
        assert_eq!(percentages, vec![1, 50, 75]);
    }
}
