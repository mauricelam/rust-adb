/*
 * Copyright (C) 2023 The Android Open Source Project
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

use adb_services::file_sync_client::SyncConnection;
use adb_services::file_sync_service::file_sync_service;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::OwnedFd;
use std::os::unix::net::UnixStream;
use tempfile::tempdir;

#[test]
fn test_file_sync_basic() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let s2_fd = OwnedFd::from(s2);

    // Start sync service in a thread
    std::thread::spawn(move || {
        file_sync_service(s2_fd.into());
    });

    let mut conn = SyncConnection::new(OwnedFd::from(s1));
    let tmp = tempdir().unwrap();
    let local_file = tmp.path().join("local.txt");
    let remote_file = tmp.path().join("remote.txt");
    let pulled_file = tmp.path().join("pulled.txt");

    // Write some data to local file
    let data = b"hello adb sync";
    {
        let mut f = File::create(&local_file).unwrap();
        f.write_all(data).unwrap();
    }

    // Push
    conn.push(
        local_file.to_str().unwrap(),
        remote_file.to_str().unwrap(),
        12345678,
        0o644,
    )
    .unwrap();

    // Verify remote file exists and has correct content
    let mut remote_data = Vec::new();
    {
        let mut f = File::open(&remote_file).unwrap();
        f.read_to_end(&mut remote_data).unwrap();
    }
    assert_eq!(remote_data, data);

    // Stat
    let st = conn.send_stat(remote_file.to_str().unwrap()).unwrap();
    let size = st.size;
    assert_eq!(size, data.len() as u64);

    // List
    let mut found = false;
    conn.send_ls(tmp.path().to_str().unwrap(), |_mode, size, _mtime, name| {
        if name == "remote.txt" {
            found = true;
            assert_eq!(size, data.len() as u64);
        }
    })
    .unwrap();
    assert!(found);

    // Pull
    conn.pull(remote_file.to_str().unwrap(), pulled_file.to_str().unwrap())
        .unwrap();

    // Verify pulled file has correct content
    let mut pulled_data = Vec::new();
    {
        let mut f = File::open(&pulled_file).unwrap();
        f.read_to_end(&mut pulled_data).unwrap();
    }
    assert_eq!(pulled_data, data);

    conn.quit().unwrap();
}

#[test]
fn test_file_sync_v2() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let s2_fd = OwnedFd::from(s2);

    // Start sync service in a thread
    std::thread::spawn(move || {
        file_sync_service(s2_fd.into());
    });

    let mut conn = SyncConnection::new(OwnedFd::from(s1));
    conn.have_stat_v2 = true;
    conn.have_ls_v2 = true;
    conn.have_sendrecv_v2 = true;

    let tmp = tempdir().unwrap();
    let local_file = tmp.path().join("local_v2.txt");
    let remote_file = tmp.path().join("remote_v2.txt");

    // Write some data to local file
    let data = b"hello adb sync v2";
    {
        let mut f = File::create(&local_file).unwrap();
        f.write_all(data).unwrap();
    }

    // Push
    conn.push(
        local_file.to_str().unwrap(),
        remote_file.to_str().unwrap(),
        12345678,
        0o644,
    )
    .unwrap();

    // Stat V2
    let st = conn.send_stat(remote_file.to_str().unwrap()).unwrap();
    let size = st.size;
    assert_eq!(size, data.len() as u64);

    // List V2
    let mut found = false;
    conn.send_ls(tmp.path().to_str().unwrap(), |_mode, size, _mtime, name| {
        if name == "remote_v2.txt" {
            found = true;
            assert_eq!(size, data.len() as u64);
        }
    })
    .unwrap();
    assert!(found);

    conn.quit().unwrap();
}
