// Copyright (C) 2023 The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! sysdeps is a cross-platform crate for system-dependent functions.

use std::env;
use std::fs;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::Path;

/// Converts a host errno to a wire protocol errno.
pub fn errno_to_wire(error: i32) -> i32 {
    match error {
        libc::EACCES => 13,
        libc::EEXIST => 17,
        libc::EFAULT => 14,
        libc::EFBIG => 27,
        libc::EINTR => 4,
        libc::EINVAL => 22,
        libc::EIO => 5,
        libc::EISDIR => 21,
        libc::ELOOP => 40,
        libc::EMFILE => 24,
        libc::ENAMETOOLONG => 36,
        libc::ENFILE => 23,
        libc::ENOENT => 2,
        libc::ENOMEM => 12,
        libc::ENOSPC => 28,
        libc::ENOTDIR => 20,
        libc::EOVERFLOW => 75,
        libc::EPERM => 1,
        libc::EROFS => 30,
        libc::ETXTBSY => 26,
        _ => {
            // TODO: Log this.
            5 // EIO
        }
    }
}

/// Converts a wire protocol errno to a host errno.
pub fn errno_from_wire(error: i32) -> i32 {
    match error {
        13 => libc::EACCES,
        17 => libc::EEXIST,
        14 => libc::EFAULT,
        27 => libc::EFBIG,
        4 => libc::EINTR,
        22 => libc::EINVAL,
        5 => libc::EIO,
        21 => libc::EISDIR,
        40 => libc::ELOOP,
        24 => libc::EMFILE,
        36 => libc::ENAMETOOLONG,
        23 => libc::ENFILE,
        2 => libc::ENOENT,
        12 => libc::ENOMEM,
        28 => libc::ENOSPC,
        20 => libc::ENOTDIR,
        75 => libc::EOVERFLOW,
        1 => libc::EPERM,
        30 => libc::EROFS,
        26 => libc::ETXTBSY,
        _ => {
            // TODO: Log this.
            libc::EIO
        }
    }
}

/// Attempts to retrieve the environment variable value for |var|. Returns None
/// if unset.
pub fn get_environment_variable(var: &str) -> Option<String> {
    env::var(var).ok()
}

/// Gets the host name of the system. Returns empty string on failure.
pub fn get_hostname() -> String {
    hostname::get().map_or_else(|_| "".to_string(), |s| s.to_string_lossy().into_owned())
}

/// Gets the current login user. Returns empty string on failure.
pub fn get_login_name() -> String {
    users::get_current_username()
        .map_or_else(|| "".to_string(), |s| s.to_string_lossy().into_owned())
}

/// Performs a stat on a path, but does not follow symlinks.
pub fn lstat(path: &Path) -> io::Result<fs::Metadata> {
    fs::symlink_metadata(path)
}

/// Performs a stat on a path.
pub fn stat(path: &Path) -> io::Result<fs::Metadata> {
    fs::metadata(path)
}

/// Creates a TCP client connected to a loopback address.
pub fn network_loopback_client(port: u16) -> io::Result<TcpStream> {
    TcpStream::connect(("127.0.0.1", port))
}

/// Creates a TCP server bound to a loopback address.
pub fn network_loopback_server(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn test_errno_to_wire() {
        assert_eq!(errno_to_wire(libc::EACCES), 13);
        assert_eq!(errno_to_wire(libc::ENOENT), 2);
        assert_eq!(errno_to_wire(libc::EINVAL), 22);
        // Test an unknown errno.
        assert_eq!(errno_to_wire(12345), 5);
    }

    #[test]
    fn test_errno_from_wire() {
        assert_eq!(errno_from_wire(13), libc::EACCES);
        assert_eq!(errno_from_wire(2), libc::ENOENT);
        assert_eq!(errno_from_wire(22), libc::EINVAL);
        // Test an unknown errno.
        assert_eq!(errno_from_wire(12345), libc::EIO);
    }

    #[test]
    fn test_stat() {
        let dir = tempdir().unwrap();
        let file = NamedTempFile::new().unwrap();

        // Test existing directory.
        let st = stat(dir.path()).unwrap();
        assert!(st.is_dir());
        assert!(!st.is_file());

        // Test existing directory with trailing slash.
        let dir_path_with_slash = format!("{}/", dir.path().to_str().unwrap());
        let st = stat(Path::new(&dir_path_with_slash)).unwrap();
        assert!(st.is_dir());

        let nonexistent_path = dir.path().join("nonexistent");

        // Test nonexistent path.
        let err = stat(&nonexistent_path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        // Test file.
        let st = stat(file.path()).unwrap();
        assert!(st.is_file());
        assert!(!st.is_dir());

        // Test file with trailing slash.
        let file_path_with_slash = format!("{}/", file.path().to_str().unwrap());
        let err = stat(Path::new(&file_path_with_slash)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotADirectory);

        #[cfg(windows)]
        {
            // Test existing directory with trailing backslash.
            let dir_path_with_slash = format!("{}\\", dir.path().to_str().unwrap());
            let st = stat(Path::new(&dir_path_with_slash)).unwrap();
            assert!(st.is_dir());

            // Test file with trailing backslash.
            let file_path_with_slash = format!("{}\\", file.path().to_str().unwrap());
            let err = stat(Path::new(&file_path_with_slash)).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotADirectory);
        }
    }

    #[test]
    fn test_get_environment_variable() {
        let key = "TEST_ENV_VAR_THAT_DOES_NOT_EXIST";
        let val = "test_value";
        assert_eq!(get_environment_variable(key), None);
        env::set_var(key, val);
        assert_eq!(get_environment_variable(key), Some(val.to_string()));
        env::remove_var(key);
    }
}
