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
use std::os::unix::io::RawFd;

/// Converts a host errno to a wire protocol errno.
pub fn errno_to_wire(error: i32) -> i32 {
    match error {
        libc::EPERM => 1,
        libc::ENOENT => 2,
        libc::EINTR => 4,
        libc::EIO => 5,
        libc::ENXIO => 6,
        libc::ENOMEM => 12,
        libc::EACCES => 13,
        libc::EFAULT => 14,
        libc::EEXIST => 17,
        libc::ENOTDIR => 20,
        libc::EISDIR => 21,
        libc::EINVAL => 22,
        libc::ENFILE => 23,
        libc::EMFILE => 24,
        libc::ETXTBSY => 26,
        libc::EFBIG => 27,
        libc::ENOSPC => 28,
        libc::EROFS => 30,
        libc::EPIPE => 32,
        libc::ENAMETOOLONG => 36,
        libc::ELOOP => 40,
        libc::EOVERFLOW => 75,
        _ => 5, // EIO
    }
}

/// Converts a wire protocol errno to a host errno.
pub fn errno_from_wire(error: i32) -> i32 {
    match error {
        1 => libc::EPERM,
        2 => libc::ENOENT,
        4 => libc::EINTR,
        5 => libc::EIO,
        6 => libc::ENXIO,
        12 => libc::ENOMEM,
        13 => libc::EACCES,
        14 => libc::EFAULT,
        17 => libc::EEXIST,
        20 => libc::ENOTDIR,
        21 => libc::EISDIR,
        22 => libc::EINVAL,
        23 => libc::ENFILE,
        24 => libc::EMFILE,
        26 => libc::ETXTBSY,
        27 => libc::EFBIG,
        28 => libc::ENOSPC,
        30 => libc::EROFS,
        32 => libc::EPIPE,
        36 => libc::ENAMETOOLONG,
        40 => libc::ELOOP,
        75 => libc::EOVERFLOW,
        _ => libc::EIO,
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

/// adb_read wrapper.
pub fn adb_read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    let res = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if res == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(res as usize)
    }
}

/// adb_write wrapper.
pub fn adb_write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    let res = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
    if res == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(res as usize)
    }
}

/// adb_close wrapper.
pub fn adb_close(fd: RawFd) -> io::Result<()> {
    let res = unsafe { libc::close(fd) };
    if res == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// adb_mkdir wrapper.
pub fn adb_mkdir(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.mode(mode);
    builder.create(path)
}

/// adb_unlink wrapper.
pub fn adb_unlink(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

/// adb_rename wrapper.
pub fn adb_rename(old: &Path, new: &Path) -> io::Result<()> {
    fs::rename(old, new)
}

/// adb_socketpair wrapper.
pub fn adb_socketpair() -> io::Result<(RawFd, RawFd)> {
    let mut fds = [0; 2];
    let res = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    if res == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok((fds[0], fds[1]))
    }
}

/// adb_thread_setname wrapper.
pub fn adb_thread_setname(name: &str) -> io::Result<()> {
    let c_name = std::ffi::CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let res = unsafe { libc::pthread_setname_np(libc::pthread_self(), c_name.as_ptr()) };
    if res != 0 {
        Err(io::Error::from_raw_os_error(res))
    } else {
        Ok(())
    }
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
        assert_eq!(errno_to_wire(libc::EPIPE), 32);
        assert_eq!(errno_to_wire(libc::ENXIO), 6);
        // Test an unknown errno.
        assert_eq!(errno_to_wire(12345), 5);
    }

    #[test]
    fn test_errno_from_wire() {
        assert_eq!(errno_from_wire(13), libc::EACCES);
        assert_eq!(errno_from_wire(2), libc::ENOENT);
        assert_eq!(errno_from_wire(22), libc::EINVAL);
        assert_eq!(errno_from_wire(32), libc::EPIPE);
        assert_eq!(errno_from_wire(6), libc::ENXIO);
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

    #[test]
    fn test_adb_socketpair() {
        let (fd1, fd2) = adb_socketpair().unwrap();
        let msg = b"hello";
        adb_write(fd1, msg).unwrap();
        let mut buf = [0; 5];
        adb_read(fd2, &mut buf).unwrap();
        assert_eq!(&buf, msg);
        adb_close(fd1).unwrap();
        adb_close(fd2).unwrap();
    }

    #[test]
    fn test_adb_thread_setname() {
        adb_thread_setname("test_thread").unwrap();
    }
}
