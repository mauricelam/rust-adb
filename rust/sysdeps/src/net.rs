#![cfg(unix)]
use std::os::unix::io::AsRawFd;
use libc;

/// Sets TCP socket keepalive.
pub fn set_tcp_keepalive<T: AsRawFd>(socket: &T, interval_sec: i32) -> bool {
    let fd = socket.as_raw_fd();
    let enable: libc::c_int = if interval_sec > 0 { 1 } else { 0 };

    if unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_KEEPALIVE,
            &enable as *const _ as *const libc::c_void,
            std::mem::size_of_val(&enable) as libc::socklen_t,
        )
    } != 0 {
        return false;
    }

    if enable == 0 {
        return true;
    }

    #[cfg(target_os = "linux")]
    {
        if unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPIDLE,
                &interval_sec as *const _ as *const libc::c_void,
                std::mem::size_of_val(&interval_sec) as libc::socklen_t,
            )
        } != 0 {
            return false;
        }
        if unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPINTVL,
                &interval_sec as *const _ as *const libc::c_void,
                std::mem::size_of_val(&interval_sec) as libc::socklen_t,
            )
        } != 0 {
            return false;
        }
        let keepcnt: libc::c_int = 10;
        if unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPCNT,
                &keepcnt as *const _ as *const libc::c_void,
                std::mem::size_of_val(&keepcnt) as libc::socklen_t,
            )
        } != 0 {
            return false;
        }
    }

    #[cfg(target_os = "macos")]
    {
        // On macOS, TCP_KEEPALIVE is used instead of TCP_KEEPIDLE
        if unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPALIVE,
                &interval_sec as *const _ as *const libc::c_void,
                std::mem::size_of_val(&interval_sec) as libc::socklen_t,
            )
        } != 0 {
            return false;
        }
    }

    true
}

/// Disables TCP Nagle algorithm.
pub fn disable_tcp_nagle<T: AsRawFd>(socket: &T) {
    let fd = socket.as_raw_fd();
    let off: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &off as *const _ as *const libc::c_void,
            std::mem::size_of_val(&off) as libc::socklen_t,
        );
    }
}

/// Checks if the file descriptor is a terminal.
pub fn unix_isatty<T: AsRawFd>(fd: &T) -> bool {
    unsafe { libc::isatty(fd.as_raw_fd()) == 1 }
}

/// Peeks at the next message size in a socket.
pub fn network_peek<T: AsRawFd>(socket: &T) -> Option<isize> {
    let fd = socket.as_raw_fd();
    #[cfg(not(target_os = "macos"))]
    {
        let ret = unsafe { libc::recv(fd, std::ptr::null_mut(), 0, libc::MSG_PEEK | libc::MSG_TRUNC) };
        if ret == -1 {
            None
        } else {
            Some(ret as isize)
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut upper_bound_bytes: libc::c_int = 0;
        let mut optlen = std::mem::size_of_val(&upper_bound_bytes) as libc::socklen_t;
        // SO_NREAD is 0x1020 on macOS
        const SO_NREAD: libc::c_int = 0x1020;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                SO_NREAD,
                &mut upper_bound_bytes as *mut _ as *mut libc::c_void,
                &mut optlen,
            )
        } == -1 {
            None
        } else {
            Some(upper_bound_bytes as isize)
        }
    }
}
