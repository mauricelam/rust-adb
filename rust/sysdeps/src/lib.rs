pub mod errno;
pub mod env;
pub mod net;
pub mod poll;

#[cfg(unix)]
pub use std::os::unix::io::AsRawFd;

/// Sets the blocking mode of a file descriptor.
///
/// Corresponds to the C++ function `set_file_block_mode` in `original/adb_utils.cpp`.
#[cfg(unix)]
pub fn set_file_block_mode<T: AsRawFd>(fd: &T, block: bool) -> bool {
    let fd = fd.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags == -1 {
        return false;
    }
    let new_flags = if block {
        flags & !libc::O_NONBLOCK
    } else {
        flags | libc::O_NONBLOCK
    };
    unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) != -1 }
}

#[cfg(windows)]
pub fn set_file_block_mode<T>(fd: &T, _block: bool) -> bool {
    // Windows implementation would go here (using ioctlsocket for sockets).
    // For now, we only support Unix.
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::tempdir;

    #[test]
    #[cfg(unix)]
    fn test_set_file_block_mode() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        File::create(&file_path).unwrap();
        let file = File::open(&file_path).unwrap();

        assert!(set_file_block_mode(&file, false));
        // Verify it's non-blocking
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL, 0) };
        assert!(flags != -1);
        assert_ne!(flags & libc::O_NONBLOCK, 0);

        assert!(set_file_block_mode(&file, true));
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL, 0) };
        assert!(flags != -1);
        assert_eq!(flags & libc::O_NONBLOCK, 0);
    }
}
