use crate::adb_client::{adb_connect, adb_query, adb_status, AdbClientError};
use adb_io::read_protocol_string;
use adb_utils::forward_targets_are_valid;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

/// Represents a forward or reverse command.
pub enum ForwardCommand {
    /// List all redirections.
    List,
    /// Remove all redirections.
    RemoveAll,
    /// Remove a specific redirection.
    Remove(String),
    /// Add a new redirection.
    Add(String, String, bool),
}

/// Manages port forwarding or reverse redirections.
pub fn adb_forward_command(command: ForwardCommand, reverse: bool) -> anyhow::Result<()> {
    let host_prefix = if reverse { "reverse:" } else { "host:" };

    match command {
        ForwardCommand::List => {
            let query = format!("{}list-forward", host_prefix);
            let result = adb_query(&query)?;
            print!("{}", result);
            Ok(())
        }
        ForwardCommand::RemoveAll => {
            let service = format!("{}killforward-all", host_prefix);
            let (fd, _) = adb_connect(&service, true)?;
            #[cfg(unix)]
            let stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
            #[cfg(windows)]
            let stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };
            adb_status(stream)?;
            Ok(())
        }
        ForwardCommand::Remove(local) => {
            let service = format!("{}killforward:{}", host_prefix, local);
            let (fd, _) = adb_connect(&service, true)?;
            #[cfg(unix)]
            let stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
            #[cfg(windows)]
            let stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };
            adb_status(stream)?;
            Ok(())
        }
        ForwardCommand::Add(local, remote, no_rebind) => {
            forward_targets_are_valid(&local, &remote).map_err(|e| AdbClientError::SocketSpec(e))?;

            let mut service = if reverse {
                "reverse:".to_string()
            } else {
                "host:".to_string()
            };

            if no_rebind {
                service.push_str("forward:norebind:");
            } else {
                service.push_str("forward:");
            }
            service.push_str(&format!("{};{}", local, remote));

            let (fd, _) = adb_connect(&service, true)?;
            #[cfg(unix)]
            let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
            #[cfg(windows)]
            let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

            if let Ok(resolved_port) = read_protocol_string(&mut stream) {
                if !resolved_port.is_empty() {
                    println!("{}", resolved_port);
                }
            }

            Ok(())
        }
    }
}
