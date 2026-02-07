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
use std::io::{Read, Write};
use sysdeps::AdbFd;
use zerocopy::IntoBytes;

pub struct SyncConnection {
    file: AdbFd,
    pub have_stat_v2: bool,
    pub have_ls_v2: bool,
    pub have_sendrecv_v2: bool,
}

impl SyncConnection {
    pub fn new(fd: AdbFd) -> Self {
        Self {
            file: fd,
            have_stat_v2: false, // Default to false, should be set based on features
            have_ls_v2: false,
            have_sendrecv_v2: false,
        }
    }

    pub fn send_request(&mut self, id: u32, path: &str) -> std::io::Result<()> {
        if path.len() > 1024 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "path too long"));
        }
        let req = SyncRequest {
            id,
            path_length: path.len() as u32,
        };
        write_exactly(&mut self.file, req.as_bytes())?;
        write_exactly(&mut self.file, path.as_bytes())
    }

    pub fn send_stat(&mut self, path: &str) -> std::io::Result<SyncStatV2> {
        let id = if self.have_stat_v2 { ID_STAT_V2 } else { ID_LSTAT_V1 };
        self.send_request(id, path)?;
        self.finish_stat(id)
    }

    pub fn send_lstat(&mut self, path: &str) -> std::io::Result<SyncStatV2> {
        let id = if self.have_stat_v2 { ID_LSTAT_V2 } else { ID_LSTAT_V1 };
        self.send_request(id, path)?;
        self.finish_stat(id)
    }

    fn finish_stat(&mut self, id: u32) -> std::io::Result<SyncStatV2> {
        if id == ID_STAT_V2 || id == ID_LSTAT_V2 {
            let mut msg = SyncStatV2 { id: 0, error: 0, dev: 0, ino: 0, mode: 0, nlink: 0, uid: 0, gid: 0, size: 0, atime: 0, mtime: 0, ctime: 0 };
            read_exactly(&mut self.file, msg.as_mut_bytes())?;
            if msg.error != 0 {
                return Err(std::io::Error::from_raw_os_error(msg.error as i32));
            }
            Ok(msg)
        } else {
            let mut msg = SyncStatV1 { id: 0, mode: 0, size: 0, mtime: 0 };
            read_exactly(&mut self.file, msg.as_mut_bytes())?;
            if msg.mode == 0 && msg.size == 0 && msg.mtime == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "not found"));
            }
            Ok(SyncStatV2 {
                id: ID_LSTAT_V2,
                error: 0,
                dev: 0,
                ino: 0,
                mode: msg.mode,
                nlink: 0,
                uid: 0,
                gid: 0,
                size: msg.size as u64,
                atime: 0,
                mtime: msg.mtime as i64,
                ctime: msg.mtime as i64,
            })
        }
    }

    pub fn send_ls<F>(&mut self, path: &str, mut callback: F) -> std::io::Result<()>
    where F: FnMut(u32, u64, i64, &str)
    {
        let id = if self.have_ls_v2 { ID_LIST_V2 } else { ID_LIST_V1 };
        self.send_request(id, path)?;

        loop {
            if self.have_ls_v2 {
                let mut dent = SyncDentV2 { id: 0, error: 0, dev: 0, ino: 0, mode: 0, nlink: 0, uid: 0, gid: 0, size: 0, atime: 0, mtime: 0, ctime: 0, namelen: 0 };
                read_exactly(&mut self.file, dent.as_mut_bytes())?;
                if dent.id == ID_DONE { break; }
                let mut name_buf = vec![0u8; dent.namelen as usize];
                read_exactly(&mut self.file, &mut name_buf)?;
                let name = String::from_utf8_lossy(&name_buf);
                callback(dent.mode, dent.size, dent.mtime, &name);
            } else {
                let mut dent = SyncDentV1 { id: 0, mode: 0, size: 0, mtime: 0, namelen: 0 };
                read_exactly(&mut self.file, dent.as_mut_bytes())?;
                if dent.id == ID_DONE { break; }
                let mut name_buf = vec![0u8; dent.namelen as usize];
                read_exactly(&mut self.file, &mut name_buf)?;
                let name = String::from_utf8_lossy(&name_buf);
                callback(dent.mode, dent.size as u64, dent.mtime as i64, &name);
            }
        }
        Ok(())
    }

    pub fn push(&mut self, lpath: &str, rpath: &str, mtime: u32, mode: u32) -> std::io::Result<()> {
        let mut lfile = std::fs::File::open(lpath)?;

        if self.have_sendrecv_v2 {
            let req = SyncRequest { id: ID_SEND_V2, path_length: rpath.len() as u32 };
            write_exactly(&mut self.file, req.as_bytes())?;
            write_exactly(&mut self.file, rpath.as_bytes())?;

            let setup = SyncSendV2 { id: ID_SEND_V2, mode, flags: 0 };
            write_exactly(&mut self.file, setup.as_bytes())?;
        } else {
            let path_and_mode = format!("{},{}", rpath, mode);
            self.send_request(ID_SEND_V1, &path_and_mode)?;
        }

        let mut buffer = vec![0u8; SYNC_DATA_MAX];
        loop {
            let n = lfile.read(&mut buffer)?;
            if n == 0 { break; }
            let msg = SyncData { id: ID_DATA, size: n as u32 };
            write_exactly(&mut self.file, msg.as_bytes())?;
            write_exactly(&mut self.file, &buffer[..n])?;
        }

        let done = SyncData { id: ID_DONE, size: mtime };
        write_exactly(&mut self.file, done.as_bytes())?;

        let mut status = SyncStatus { id: 0, msglen: 0 };
        read_exactly(&mut self.file, status.as_mut_bytes())?;
        let status_id = status.id;
        if status_id == ID_OKAY {
            Ok(())
        } else {
            let mut msg = vec![0u8; status.msglen as usize];
            read_exactly(&mut self.file, &mut msg)?;
            Err(std::io::Error::new(std::io::ErrorKind::Other, String::from_utf8_lossy(&msg)))
        }
    }

    pub fn pull(&mut self, rpath: &str, lpath: &str) -> std::io::Result<()> {
        if self.have_sendrecv_v2 {
            let req = SyncRequest { id: ID_RECV_V2, path_length: rpath.len() as u32 };
            write_exactly(&mut self.file, req.as_bytes())?;
            write_exactly(&mut self.file, rpath.as_bytes())?;

            let setup = SyncRecvV2 { id: ID_RECV_V2, flags: 0 };
            write_exactly(&mut self.file, setup.as_bytes())?;
        } else {
            self.send_request(ID_RECV_V1, rpath)?;
        }

        let mut lfile = std::fs::File::create(lpath)?;
        loop {
            let mut msg = SyncData { id: 0, size: 0 };
            read_exactly(&mut self.file, msg.as_mut_bytes())?;
            if msg.id == ID_DONE { break; }
            if msg.id == ID_FAIL {
                let mut err_msg = vec![0u8; msg.size as usize];
                read_exactly(&mut self.file, &mut err_msg)?;
                return Err(std::io::Error::new(std::io::ErrorKind::Other, String::from_utf8_lossy(&err_msg)));
            }
            let mut buffer = vec![0u8; msg.size as usize];
            read_exactly(&mut self.file, &mut buffer)?;
            lfile.write_all(&buffer)?;
        }
        Ok(())
    }

    pub fn quit(&mut self) -> std::io::Result<()> {
        self.send_request(ID_QUIT, "")
    }
}
