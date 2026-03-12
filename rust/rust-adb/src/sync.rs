use std::path::{Path, PathBuf};
use crate::adb_client::{adb_connect, adb_get_feature_set, format_host_command};
use adb_services::file_sync_client::SyncConnection;
use adb_transport::{FEATURE_STAT2, FEATURE_LS2, FEATURE_SENDRECV_V2, can_use_feature};
use std::fs;

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

struct CopyInfo {
    lpath: PathBuf,
    rpath: String,
    mtime: u32,
    mode: u32,
    size: u64,
}

fn local_build_list(lpath: &Path, rpath: &str, file_list: &mut Vec<CopyInfo>) -> anyhow::Result<()> {
    for entry in fs::read_dir(lpath)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str == "." || name_str == ".." {
            continue;
        }

        let mut new_rpath = rpath.to_string();
        if !new_rpath.ends_with('/') {
            new_rpath.push('/');
        }
        new_rpath.push_str(&name_str);

        if metadata.is_dir() {
            local_build_list(&entry.path(), &new_rpath, file_list)?;
        } else {
            file_list.push(CopyInfo {
                lpath: entry.path(),
                rpath: new_rpath,
                mtime: metadata.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs() as u32,
                mode: 0o644, // Default mode for files
                size: metadata.len(),
            });
        }
    }
    Ok(())
}

fn remote_build_list(sc: &mut SyncConnection, rpath: &str, lpath: &Path, file_list: &mut Vec<CopyInfo>) -> anyhow::Result<()> {
    let mut entries = Vec::new();
    sc.send_ls(rpath, |mode, size, mtime, name| {
        if name != "." && name != ".." {
            entries.push((mode, size, mtime, name.to_string()));
        }
    })?;

    for (mode, size, mtime, name) in entries {
        let mut new_rpath = rpath.to_string();
        if !new_rpath.ends_with('/') {
            new_rpath.push('/');
        }
        new_rpath.push_str(&name);

        let new_lpath = lpath.join(&name);

        if (mode & 0o170000) == 0o040000 { // S_IFDIR
            remote_build_list(sc, &new_rpath, &new_lpath, file_list)?;
        } else {
            file_list.push(CopyInfo {
                lpath: new_lpath,
                rpath: new_rpath,
                mtime: mtime as u32,
                mode,
                size,
            });
        }
    }
    Ok(())
}

/// Pushes files to the device.
pub fn adb_push(srcs: &[String], dst: &str) -> anyhow::Result<()> {
    let mut sc = create_sync_connection()?;

    for src in srcs {
        let src_path = Path::new(src);
        if src_path.is_dir() {
            let mut file_list = Vec::new();
            let mut dst_dir = dst.to_string();
            if !dst_dir.ends_with('/') {
                dst_dir.push('/');
            }
            dst_dir.push_str(&src_path.file_name().unwrap().to_string_lossy());
            local_build_list(src_path, &dst_dir, &mut file_list)?;

            for info in file_list {
                sc.push(&info.lpath.to_string_lossy(), &info.rpath, info.mtime, info.mode)?;
            }
        } else {
            let rpath = if dst.ends_with('/') {
                format!("{}{}", dst, src_path.file_name().unwrap().to_string_lossy())
            } else {
                dst.to_string()
            };
            let metadata = fs::metadata(src_path)?;
            let mtime = metadata.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs() as u32;
            sc.push(src, &rpath, mtime, 0o644)?;
        }
    }

    sc.quit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    fn test_local_build_list() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let file1 = dir.path().join("file1.txt");
        let subdir = dir.path().join("subdir");
        let file2 = subdir.join("file2.txt");

        File::create(&file1)?;
        fs::create_dir(&subdir)?;
        File::create(&file2)?;

        let mut file_list = Vec::new();
        local_build_list(dir.path(), "/data", &mut file_list)?;

        assert_eq!(file_list.len(), 2);
        let paths: Vec<String> = file_list.iter().map(|f| f.rpath.clone()).collect();
        assert!(paths.contains(&"/data/file1.txt".to_string()));
        assert!(paths.contains(&"/data/subdir/file2.txt".to_string()));

        Ok(())
    }
}

/// Pulls files from the device.
pub fn adb_pull(srcs: &[String], dst: &str) -> anyhow::Result<()> {
    let mut sc = create_sync_connection()?;

    for src in srcs {
        let dst_path = Path::new(dst);
        let stat = sc.send_lstat(src)?;
        if (stat.mode & 0o170000) == 0o040000 { // S_IFDIR
            let mut file_list = Vec::new();
            let lpath = if dst_path.is_dir() {
                dst_path.join(Path::new(src).file_name().unwrap())
            } else {
                dst_path.to_path_buf()
            };
            remote_build_list(&mut sc, src, &lpath, &mut file_list)?;

            for info in file_list {
                if let Some(parent) = info.lpath.parent() {
                    fs::create_dir_all(parent)?;
                }
                sc.pull(&info.rpath, &info.lpath.to_string_lossy())?;
            }
        } else {
            let lpath = if dst_path.is_dir() {
                dst_path.join(Path::new(src).file_name().unwrap())
            } else {
                dst_path.to_path_buf()
            };
            sc.pull(src, &lpath.to_string_lossy())?;
        }
    }

    sc.quit()?;
    Ok(())
}

/// Syncs local files to the device.
pub fn adb_sync(partition: Option<&str>) -> anyhow::Result<()> {
    let product_out = std::env::var("ANDROID_PRODUCT_OUT")
        .map_err(|_| anyhow::anyhow!("ANDROID_PRODUCT_OUT not set"))?;
    let product_out_path = Path::new(&product_out);

    if !product_out_path.exists() {
        anyhow::bail!("ANDROID_PRODUCT_OUT directory does not exist: {}", product_out);
    }

    let partitions = if let Some(p) = partition {
        vec![p]
    } else {
        vec!["system", "vendor", "oem", "data", "product", "system_ext"]
    };

    let mut sc = create_sync_connection()?;

    for p in partitions {
        let lpath = product_out_path.join(p);
        if !lpath.exists() {
            continue;
        }

        let rpath = format!("/{}", p);
        let mut file_list = Vec::new();
        local_build_list(&lpath, &rpath, &mut file_list)?;

        for info in file_list {
            // Check if file needs to be synced (simple mtime/size check)
            let mut needs_sync = true;
            if let Ok(rstat) = sc.send_lstat(&info.rpath) {
                if rstat.size == info.size && rstat.mtime == info.mtime as i64 {
                    needs_sync = false;
                }
            }

            if needs_sync {
                sc.push(&info.lpath.to_string_lossy(), &info.rpath, info.mtime, info.mode)?;
            }
        }
    }

    sc.quit()?;
    Ok(())
}
