#![cfg(unix)]
use libc;

/// Wrapper around `libc::pollfd`.
pub type AdbPollFd = libc::pollfd;

/// Wrapper around `libc::poll`.
pub fn adb_poll(fds: &mut [AdbPollFd], timeout: i32) -> i32 {
    unsafe {
        libc::poll(
            fds.as_mut_ptr(),
            fds.len() as libc::nfds_t,
            timeout,
        )
    }
}
