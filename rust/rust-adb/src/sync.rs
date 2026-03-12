use std::path::{Path};
use crate::adb_client::{adb_connect, adb_get_feature_set, format_host_command};
use adb_services::file_sync_client::SyncConnection;
use adb_transport::{FEATURE_STAT2, FEATURE_LS2, FEATURE_SENDRECV_V2, can_use_feature};

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, IntoRawSocket, FromRawHandle};

fn create_sync_connection() -> anyhow::Result<SyncConnection> {
    let service = format_host_command("sync:");
    let (fd, _) = adb_connect(&service, false)?;
    let features = adb_get_feature_set()?;

    #[cfg(unix)]
    let mut conn = SyncConnection::new(unsafe { std::os::unix::io::OwnedFd::from_raw_fd(fd.into_raw_fd()) });
    #[cfg(windows)]
    let mut conn = SyncConnection::new(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(fd.into_raw_socket() as _) });

    conn.have_stat_v2 = can_use_feature(&features, FEATURE_STAT2);
    conn.have_ls_v2 = can_use_feature(&features, FEATURE_LS2);
    conn.have_sendrecv_v2 = can_use_feature(&features, FEATURE_SENDRECV_V2);

    Ok(conn)
}

/// Pushes files to the device.
pub fn adb_push(srcs: &[String], dst: &str) -> anyhow::Result<()> {
    let mut sc = create_sync_connection()?;

    for src in srcs {
        let src_path = Path::new(src);
        if src_path.is_dir() {
            // Recursive push logic would go here.
            // For now, let's just implement single file push as a base.
            anyhow::bail!("directory push not yet fully implemented in CLI");
        } else {
            let rpath = if dst.ends_with('/') {
                format!("{}{}", dst, src_path.file_name().unwrap().to_string_lossy())
            } else {
                dst.to_string()
            };
            sc.push(src, &rpath, 0, 0o644)?;
        }
    }

    sc.quit()?;
    Ok(())
}

/// Pulls files from the device.
pub fn adb_pull(srcs: &[String], dst: &str) -> anyhow::Result<()> {
    let mut sc = create_sync_connection()?;

    for src in srcs {
        let dst_path = Path::new(dst);
        let lpath = if dst_path.is_dir() {
            dst_path.join(Path::new(src).file_name().unwrap())
        } else {
            dst_path.to_path_buf()
        };
        sc.pull(src, &lpath.to_string_lossy())?;
    }

    sc.quit()?;
    Ok(())
}

/// Syncs local files to the device.
pub fn adb_sync(_partition: Option<&str>) -> anyhow::Result<()> {
    let _sc = create_sync_connection()?;
    // Logic for sync based on $ANDROID_PRODUCT_OUT
    anyhow::bail!("sync not yet fully implemented in CLI");
}
