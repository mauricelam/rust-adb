#[cfg(unix)]
use libc;

#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::*;

/// Wrapper around `libc::pollfd` or equivalent on Windows.
/// Ported from `adb_pollfd` in `sysdeps.h`.
#[cfg(unix)]
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct AdbPollFd {
    /// The file descriptor to monitor.
    pub fd: i32,
    /// The events to monitor.
    pub events: i16,
    /// The events that occurred.
    pub revents: i16,
}

#[cfg(unix)]
/// Data is available to read.
pub const POLLIN: i16 = libc::POLLIN;
/// Ready to write data.
pub const POLLOUT: i16 = libc::POLLOUT;
/// Error condition.
pub const POLLERR: i16 = libc::POLLERR;
/// Hang up.
pub const POLLHUP: i16 = libc::POLLHUP;

#[cfg(windows)]
#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct AdbPollFd {
    /// The socket handle to monitor.
    pub fd: usize,
    /// The events to monitor.
    pub events: i16,
    /// The events that occurred.
    pub revents: i16,
}

#[cfg(windows)]
/// Data is available to read.
pub const POLLIN: i16 = 0x0001;
#[cfg(windows)]
/// Ready to write data.
pub const POLLOUT: i16 = 0x0004;
#[cfg(windows)]
/// Error condition.
pub const POLLERR: i16 = 0x0008;
#[cfg(windows)]
/// Hang up.
pub const POLLHUP: i16 = 0x0010;

/// Wrapper around `libc::poll` or `WSAPoll`.
/// Ported from `adb_poll` in `sysdeps.h`.
pub fn adb_poll(fds: &mut [AdbPollFd], timeout: i32) -> i32 {
    #[cfg(unix)]
    {
        // SAFETY: poll is a standard libc function.
        unsafe {
            libc::poll(
                fds.as_mut_ptr() as *mut libc::pollfd,
                fds.len() as libc::nfds_t,
                timeout,
            )
        }
    }
    #[cfg(windows)]
    {
        // Optimization: use a stack-allocated array for small numbers of file descriptors
        // to avoid frequent heap allocations in the event loop.
        const STACK_LIMIT: usize = 16;
        let mut wsapollfds_stack = [WSAPOLLFD { fd: 0, events: 0, revents: 0 }; STACK_LIMIT];
        let mut wsapollfds_vec;

        let wsapollfds = if fds.len() <= STACK_LIMIT {
            for (i, f) in fds.iter().enumerate() {
                wsapollfds_stack[i] = WSAPOLLFD {
                    fd: f.fd,
                    events: f.events,
                    revents: f.revents,
                };
            }
            &mut wsapollfds_stack[..fds.len()]
        } else {
            wsapollfds_vec = fds
                .iter()
                .map(|f| WSAPOLLFD {
                    fd: f.fd,
                    events: f.events,
                    revents: f.revents,
                })
                .collect::<Vec<_>>();
            &mut wsapollfds_vec[..]
        };

        let ret = unsafe {
            WSAPoll(
                wsapollfds.as_mut_ptr(),
                wsapollfds.len() as u32,
                timeout,
            )
        };

        if ret >= 0 {
            for (f, w) in fds.iter_mut().zip(wsapollfds.iter()) {
                f.revents = w.revents;
            }
        }

        ret
    }
}
