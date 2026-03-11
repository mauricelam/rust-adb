use crate::adb_client::{adb_connect, adb_query};
use adb_io::{read_protocol_string, read_orderly_shutdown};
use adb_transport::{can_use_feature, FEATURE_DEVRAW};
use adb_utils::forward_targets_are_valid;
use anyhow::{anyhow, Result};

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

/// Checks if the destination for a forward/reverse command is supported by the device.
/// Ported from `forward_dest_is_featured` in `commandline.cpp`.
pub fn forward_dest_is_featured(dest: &str) -> Result<()> {
    if dest.starts_with("dev-raw:") {
        let features = crate::adb_install::get_device_features()?;
        if !can_use_feature(&features, FEATURE_DEVRAW) {
            return Err(anyhow!("dev-raw is not supported by the device"));
        }
    }
    Ok(())
}

/// Command type for forward/reverse operations.
#[derive(Debug, PartialEq)]
pub enum ForwardCommand {
    /// List all forward/reverse socket connections.
    List,
    /// Remove all forward/reverse socket connections.
    RemoveAll,
    /// Remove a specific forward/reverse socket connection.
    Remove(String),
    /// Create a new forward/reverse socket connection.
    Forward(String, String, bool), // local, remote, no_rebind
}

/// Parses the command line arguments for forward/reverse commands.
pub fn parse_forward_reverse_args(args: &[String], reverse: bool) -> Result<ForwardCommand> {
    let mut args_iter = args.iter();
    let first_arg = args_iter.next().ok_or_else(|| anyhow!("{} requires an argument", if reverse { "reverse" } else { "forward" }))?;

    if first_arg == "--list" {
        if args_iter.next().is_some() {
            return Err(anyhow!("--list doesn't take any arguments"));
        }
        Ok(ForwardCommand::List)
    } else if first_arg == "--remove-all" {
        if args_iter.next().is_some() {
            return Err(anyhow!("--remove-all doesn't take any arguments"));
        }
        Ok(ForwardCommand::RemoveAll)
    } else if first_arg == "--remove" {
        let local = args_iter.next().ok_or_else(|| anyhow!("--remove requires an argument"))?;
        if args_iter.next().is_some() {
            return Err(anyhow!("--remove takes only one argument"));
        }
        Ok(ForwardCommand::Remove(local.clone()))
    } else if first_arg == "--no-rebind" {
        let local = args_iter.next().ok_or_else(|| anyhow!("--no-rebind takes two arguments"))?;
        let remote = args_iter.next().ok_or_else(|| anyhow!("--no-rebind takes two arguments"))?;
        if args_iter.next().is_some() {
            return Err(anyhow!("--no-rebind takes only two arguments"));
        }
        forward_targets_are_valid(local, remote).map_err(|e| anyhow!("{}", e))?;
        forward_dest_is_featured(remote)?;
        Ok(ForwardCommand::Forward(local.clone(), remote.clone(), true))
    } else {
        let local = first_arg;
        let remote = args_iter.next().ok_or_else(|| anyhow!("{} takes two arguments", if reverse { "reverse" } else { "forward" }))?;
        if args_iter.next().is_some() {
            return Err(anyhow!("too many arguments"));
        }
        forward_targets_are_valid(local, remote).map_err(|e| anyhow!("{}", e))?;
        forward_dest_is_featured(remote)?;
        Ok(ForwardCommand::Forward(local.clone(), remote.clone(), false))
    }
}

/// Implements the forward and reverse commands.
/// Ported from the forward/reverse logic in `adb_commandline` in `commandline.cpp`.
pub fn do_forward_reverse(args: &[String], reverse: bool) -> Result<()> {
    let host_prefix = if reverse { "reverse:" } else { "host:" };
    let command = parse_forward_reverse_args(args, reverse)?;

    let cmd = match command {
        ForwardCommand::List => {
            let result = adb_query(&format!("{}list-forward", host_prefix))?;
            print!("{}", result);
            return Ok(());
        }
        ForwardCommand::RemoveAll => "killforward-all".to_string(),
        ForwardCommand::Remove(local) => format!("killforward:{}", local),
        ForwardCommand::Forward(local, remote, no_rebind) => {
            if no_rebind {
                format!("forward:norebind:{};{}", local, remote)
            } else {
                format!("forward:{};{}", local, remote)
            }
        }
    };

    let service = format!("{}{}", host_prefix, cmd);
    let (fd, _) = adb_connect(&service, true)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    // Server or device may optionally return a resolved TCP port number.
    if let Ok(resolved_port) = read_protocol_string(&mut stream) {
        if !resolved_port.is_empty() {
            println!("{}", resolved_port);
        }
    }

    let _ = read_orderly_shutdown(&mut stream);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adb_utils::forward_targets_are_valid;

    #[test]
    fn test_forward_targets_validation() {
        assert!(forward_targets_are_valid("tcp:8000", "tcp:9000").is_ok());
        assert!(forward_targets_are_valid("tcp:0", "tcp:9000").is_ok());
        assert!(forward_targets_are_valid("tcp:-1", "tcp:9000").is_err());
        assert!(forward_targets_are_valid("tcp:8000", "tcp:0").is_err());
        assert!(forward_targets_are_valid("tcp:8000", "tcp:-1").is_err());
    }

    #[test]
    fn test_parse_forward_reverse_args() {
        assert_eq!(parse_forward_reverse_args(&["--list".to_string()], false).unwrap(), ForwardCommand::List);
        assert_eq!(parse_forward_reverse_args(&["--remove-all".to_string()], false).unwrap(), ForwardCommand::RemoveAll);
        assert_eq!(parse_forward_reverse_args(&["--remove".to_string(), "tcp:8000".to_string()], false).unwrap(), ForwardCommand::Remove("tcp:8000".to_string()));
        assert_eq!(parse_forward_reverse_args(&["tcp:8000".to_string(), "tcp:9000".to_string()], false).unwrap(), ForwardCommand::Forward("tcp:8000".to_string(), "tcp:9000".to_string(), false));
        assert_eq!(parse_forward_reverse_args(&["--no-rebind".to_string(), "tcp:8000".to_string(), "tcp:9000".to_string()], false).unwrap(), ForwardCommand::Forward("tcp:8000".to_string(), "tcp:9000".to_string(), true));
    }

    #[test]
    fn test_parse_forward_reverse_args_invalid() {
        assert!(parse_forward_reverse_args(&[], false).is_err());
        assert!(parse_forward_reverse_args(&["--list".to_string(), "extra".to_string()], false).is_err());
        assert!(parse_forward_reverse_args(&["--remove-all".to_string(), "extra".to_string()], false).is_err());
        assert!(parse_forward_reverse_args(&["--remove".to_string()], false).is_err());
        assert!(parse_forward_reverse_args(&["tcp:8000".to_string()], false).is_err());
        assert!(parse_forward_reverse_args(&["tcp:8000".to_string(), "tcp:9000".to_string(), "extra".to_string()], false).is_err());
    }
}
