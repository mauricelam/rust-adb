/*
 * Copyright (C) 2007 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use adb_io::{read_exactly, write_exactly};
use adb_protocol::file_sync_protocol::*;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::OwnedFd;
use std::path::Path;
use zerocopy::IntoBytes;

pub fn file_sync_service(fd: OwnedFd) {
    let mut file = File::from(fd);
    let mut buffer = vec![0u8; SYNC_DATA_MAX];

    while handle_sync_command(&mut file, &mut buffer) {}
}

fn handle_sync_command(file: &mut File, buffer: &mut [u8]) -> bool {
    let mut request = SyncRequest {
        id: 0,
        path_length: 0,
    };
    if read_exactly(file, request.as_mut_bytes()).is_err() {
        return false;
    }

    let id = request.id;
    let path_length = request.path_length as usize;
    if path_length > 1024 {
        let _ = send_sync_fail(file, "path too long");
        return false;
    }

    let mut path_buf = vec![0u8; path_length];
    if read_exactly(file, &mut path_buf).is_err() {
        return false;
    }
    let path = String::from_utf8_lossy(&path_buf).into_owned();

    match id {
        ID_LSTAT_V1 => {
            let _ = do_lstat_v1(file, &path);
        }
        ID_LSTAT_V2 | ID_STAT_V2 => {
            let _ = do_stat_v2(file, id, &path);
        }
        ID_LIST_V1 => {
            let _ = do_list_v1(file, &path);
        }
        ID_LIST_V2 => {
            let _ = do_list_v2(file, &path);
        }
        ID_SEND_V1 => {
            return do_send_v1(file, &path, buffer);
        }
        ID_SEND_V2 => {
            return do_send_v2(file, &path, buffer);
        }
        ID_RECV_V1 => {
            return do_recv_v1(file, &path, buffer);
        }
        ID_RECV_V2 => {
            return do_recv_v2(file, &path, buffer);
        }
        ID_QUIT => {
            return false;
        }
        _ => {
            let _ = send_sync_fail(file, &format!("unknown command {:08x}", id));
            return false;
        }
    }

    true
}

fn send_sync_fail(file: &mut File, reason: &str) -> std::io::Result<()> {
    let msg = SyncData {
        id: ID_FAIL,
        size: reason.len() as u32,
    };
    write_exactly(file, msg.as_bytes())?;
    write_exactly(file, reason.as_bytes())
}

fn do_lstat_v1(file: &mut File, path: &str) -> std::io::Result<()> {
    let mut msg = SyncStatV1 {
        id: ID_LSTAT_V1,
        mode: 0,
        size: 0,
        mtime: 0,
    };

    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        msg.mode = metadata.mode();
        msg.size = metadata.size() as u32;
        msg.mtime = metadata.mtime() as u32;
    }

    write_exactly(file, msg.as_bytes())
}

fn do_stat_v2(file: &mut File, id: u32, path: &str) -> std::io::Result<()> {
    let mut msg = SyncStatV2 {
        id,
        error: 0,
        dev: 0,
        ino: 0,
        mode: 0,
        nlink: 0,
        uid: 0,
        gid: 0,
        size: 0,
        atime: 0,
        mtime: 0,
        ctime: 0,
    };

    let result = if id == ID_STAT_V2 {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    };

    match result {
        Ok(st) => {
            msg.dev = st.dev();
            msg.ino = st.ino();
            msg.mode = st.mode();
            msg.nlink = st.nlink() as u32;
            msg.uid = st.uid();
            msg.gid = st.gid();
            msg.size = st.size();
            msg.atime = st.atime();
            msg.mtime = st.mtime();
            msg.ctime = st.ctime();
        }
        Err(e) => {
            msg.error = e.raw_os_error().unwrap_or(libc::EINVAL) as u32;
        }
    }

    write_exactly(file, msg.as_bytes())
}

fn do_list(file: &mut File, v2: bool, path: &str) -> std::io::Result<()> {
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries {
            if let Ok(entry) = entry {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let name_bytes = name_str.as_bytes();
                let metadata = entry.metadata();

                if v2 {
                    let mut msg = SyncDentV2 {
                        id: ID_DENT_V2,
                        error: 0,
                        dev: 0,
                        ino: 0,
                        mode: 0,
                        nlink: 0,
                        uid: 0,
                        gid: 0,
                        size: 0,
                        atime: 0,
                        mtime: 0,
                        ctime: 0,
                        namelen: name_bytes.len() as u32,
                    };
                    if let Ok(st) = metadata {
                        msg.dev = st.dev();
                        msg.ino = st.ino();
                        msg.mode = st.mode();
                        msg.nlink = st.nlink() as u32;
                        msg.uid = st.uid();
                        msg.gid = st.gid();
                        msg.size = st.size();
                        msg.atime = st.atime();
                        msg.mtime = st.mtime();
                        msg.ctime = st.ctime();
                    } else {
                        msg.error = libc::EACCES as u32;
                    }
                    write_exactly(file, msg.as_bytes())?;
                    write_exactly(file, name_bytes)?;
                } else {
                    let mut msg = SyncDentV1 {
                        id: ID_DENT_V1,
                        mode: 0,
                        size: 0,
                        mtime: 0,
                        namelen: name_bytes.len() as u32,
                    };
                    if let Ok(st) = metadata {
                        msg.mode = st.mode();
                        msg.size = st.size() as u32;
                        msg.mtime = st.mtime() as u32;
                        write_exactly(file, msg.as_bytes())?;
                        write_exactly(file, name_bytes)?;
                    }
                }
            }
        }
    }

    if v2 {
        let done = SyncDentV2 {
            id: ID_DONE,
            error: 0,
            dev: 0,
            ino: 0,
            mode: 0,
            nlink: 0,
            uid: 0,
            gid: 0,
            size: 0,
            atime: 0,
            mtime: 0,
            ctime: 0,
            namelen: 0,
        };
        write_exactly(file, done.as_bytes())
    } else {
        let done = SyncDentV1 {
            id: ID_DONE,
            mode: 0,
            size: 0,
            mtime: 0,
            namelen: 0,
        };
        write_exactly(file, done.as_bytes())
    }
}

fn do_list_v1(file: &mut File, path: &str) -> std::io::Result<()> {
    do_list(file, false, path)
}

fn do_list_v2(file: &mut File, path: &str) -> std::io::Result<()> {
    do_list(file, true, path)
}

fn do_send_v1(file: &mut File, spec: &str, buffer: &mut [u8]) -> bool {
    // spec is "/path,mode"
    let comma = match spec.rfind(',') {
        Some(idx) => idx,
        None => {
            let _ = send_sync_fail(file, "missing , in ID_SEND_V1");
            return false;
        }
    };

    let path = &spec[..comma];
    let mode_str = &spec[comma + 1..];
    let mode = match u32::from_str_radix(mode_str, 10) {
        Ok(m) => m,
        Err(_) => {
            let _ = send_sync_fail(file, "bad mode");
            return false;
        }
    };

    send_impl(file, path, mode, CompressionType::None, false, buffer)
}

fn do_send_v2(file: &mut File, path: &str, buffer: &mut [u8]) -> bool {
    let mut setup = SyncSendV2 {
        id: 0,
        mode: 0,
        flags: 0,
    };
    if read_exactly(file, setup.as_mut_bytes()).is_err() {
        return false;
    }

    let mut dry_run = false;
    let mut compression = CompressionType::None;

    if setup.flags & SYNC_FLAG_BROTLI != 0 {
        compression = CompressionType::Brotli;
    } else if setup.flags & SYNC_FLAG_LZ4 != 0 {
        compression = CompressionType::LZ4;
    } else if setup.flags & SYNC_FLAG_ZSTD != 0 {
        compression = CompressionType::Zstd;
    }

    if setup.flags & SYNC_FLAG_DRY_RUN != 0 {
        dry_run = true;
    }

    send_impl(file, path, setup.mode, compression, dry_run, buffer)
}

#[allow(unused_assignments)]
fn send_impl(file: &mut File, path: &str, mode: u32, compression: CompressionType, dry_run: bool, buffer: &mut [u8]) -> bool {
    if compression != CompressionType::None {
        let _ = send_sync_fail(file, "compression not supported");
        return false;
    }

    let dest_path = Path::new(path);
    if let Some(parent) = dest_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut dest_file = if dry_run {
        None
    } else {
        match OpenOptions::new().write(true).create(true).truncate(true).open(path) {
            Ok(f) => {
                let _ = f.set_permissions(std::fs::Permissions::from_mode(mode & 0o777));
                Some(f)
            }
            Err(e) => {
                let _ = send_sync_fail(file, &format!("failed to open destination: {}", e));
                return false;
            }
        }
    };

    let mut timestamp = 0;
    loop {
        let mut msg = SyncData { id: 0, size: 0 };
        if read_exactly(file, msg.as_mut_bytes()).is_err() {
            return false;
        }

        match msg.id {
            ID_DATA => {
                let size = msg.size as usize;
                if size > buffer.len() {
                    let _ = send_sync_fail(file, "packet too large");
                    return false;
                }
                if read_exactly(file, &mut buffer[..size]).is_err() {
                    return false;
                }
                if let Some(ref mut f) = dest_file {
                    if f.write_all(&buffer[..size]).is_err() {
                        let _ = send_sync_fail(file, "failed to write to destination");
                        return false;
                    }
                }
            }
            ID_DONE => {
                timestamp = msg.size;
                break;
            }
            _ => {
                let _ = send_sync_fail(file, "invalid data message");
                return false;
            }
        }
    }

    if let Some(f) = dest_file {
        drop(f);
        // Set timestamp
        let times = [
            libc::timeval {
                tv_sec: timestamp as libc::time_t,
                tv_usec: 0,
            },
            libc::timeval {
                tv_sec: timestamp as libc::time_t,
                tv_usec: 0,
            },
        ];
        unsafe {
            if let Ok(path_c) = std::ffi::CString::new(path) {
                libc::utimes(path_c.as_ptr(), times.as_ptr());
            }
        }
    }

    let okay = SyncStatus { id: ID_OKAY, msglen: 0 };
    write_exactly(file, okay.as_bytes()).is_ok()
}

fn do_recv_v1(file: &mut File, path: &str, buffer: &mut [u8]) -> bool {
    recv_impl(file, path, CompressionType::None, buffer)
}

fn do_recv_v2(file: &mut File, path: &str, buffer: &mut [u8]) -> bool {
    let mut setup = SyncRecvV2 { id: 0, flags: 0 };
    if read_exactly(file, setup.as_mut_bytes()).is_err() {
        return false;
    }

    let mut compression = CompressionType::None;
    if setup.flags & SYNC_FLAG_BROTLI != 0 {
        compression = CompressionType::Brotli;
    } else if setup.flags & SYNC_FLAG_LZ4 != 0 {
        compression = CompressionType::LZ4;
    } else if setup.flags & SYNC_FLAG_ZSTD != 0 {
        compression = CompressionType::Zstd;
    }

    recv_impl(file, path, compression, buffer)
}

fn recv_impl(file: &mut File, path: &str, compression: CompressionType, buffer: &mut [u8]) -> bool {
    if compression != CompressionType::None {
        let _ = send_sync_fail(file, "compression not supported");
        return false;
    }

    let mut src_file = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            let _ = send_sync_fail(file, &format!("failed to open source: {}", e));
            return false;
        }
    };

    loop {
        match src_file.read(buffer) {
            Ok(0) => break,
            Ok(n) => {
                let msg = SyncData {
                    id: ID_DATA,
                    size: n as u32,
                };
                if write_exactly(file, msg.as_bytes()).is_err() ||
                   write_exactly(file, &buffer[..n]).is_err() {
                    return false;
                }
            }
            Err(e) => {
                let _ = send_sync_fail(file, &format!("failed to read from source: {}", e));
                return false;
            }
        }
    }

    let done = SyncData { id: ID_DONE, size: 0 };
    write_exactly(file, done.as_bytes()).is_ok()
}
