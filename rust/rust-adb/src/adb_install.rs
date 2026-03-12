use std::io::{Read, self};
use crate::adb_client::{adb_connect, adb_get_feature_set, format_host_command};
use crate::incremental_server::IncrementalServer;
use adb_transport::{FEATURE_CMD, can_use_feature};
use adb_utils::escape_arg;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket};

/// Installs a single package.
pub fn adb_install(pkg: &str, args: &[String]) -> anyhow::Result<()> {
    let mut install_args = args.to_vec();
    let mut is_incremental = false;

    install_args.retain(|arg| {
        if arg == "--incremental" {
            is_incremental = true;
            false
        } else {
            true
        }
    });

    let features = adb_get_feature_set()?;
    let use_cmd = can_use_feature(&features, FEATURE_CMD);

    if is_incremental {
        let mut cmd = if use_cmd {
            format!("exec:cmd package install-incremental")
        } else {
            format!("exec:pm install-incremental")
        };
        for arg in &install_args {
            cmd.push_str(&format!(" {}", escape_arg(arg)));
        }

        let service = format_host_command(&cmd);
        let (fd, _) = adb_connect(&service, false)?;

        let mut server = IncrementalServer::new(fd, &[pkg.to_string()])?;
        return server.serve();
    }

    let file = std::fs::File::open(pkg)?;
    let size = file.metadata()?.len();

    let mut cmd = if use_cmd {
        format!("exec:cmd package install -S {}", size)
    } else {
        format!("exec:pm install -S {}", size)
    };

    for arg in &install_args {
        cmd.push_str(&format!(" {}", escape_arg(arg)));
    }

    let service = format_host_command(&cmd);
    let (fd, _) = adb_connect(&service, false)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut file = file;
    io::copy(&mut file, &mut stream)?;

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    print!("{}", buf);

    Ok(())
}

/// Uninstalls a package.
pub fn adb_uninstall(pkg: &str, keep_data: bool) -> anyhow::Result<()> {
    let features = adb_get_feature_set()?;
    let use_cmd = can_use_feature(&features, FEATURE_CMD);

    let cmd = if use_cmd {
        format!("shell:cmd package uninstall {} {}", if keep_data { "-k" } else { "" }, escape_arg(pkg))
    } else {
        format!("shell:pm uninstall {} {}", if keep_data { "-k" } else { "" }, escape_arg(pkg))
    };

    let service = format_host_command(&cmd);
    let (fd, _) = adb_connect(&service, false)?;

    #[cfg(unix)]
    let mut stream = unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) };
    #[cfg(windows)]
    let mut stream = unsafe { std::net::TcpStream::from_raw_socket(fd.into_raw_socket() as _) };

    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;
    print!("{}", buf);

    Ok(())
}
