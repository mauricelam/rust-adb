#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
#[cfg(unix)]
use libc;

#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket, OwnedSocket};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::*;
#[cfg(windows)]
use std::sync::Once;

#[cfg(windows)]
static WSA_STARTUP: Once = Once::new();

#[cfg(windows)]
pub fn ensure_wsa_startup() {
    WSA_STARTUP.call_once(|| {
        let mut data = unsafe { std::mem::zeroed() };
        unsafe { WSAStartup(0x0202, &mut data) };
    });
}

/// Sets TCP socket keepalive.
#[cfg(unix)]
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

#[cfg(windows)]
pub fn set_tcp_keepalive<T: AsRawSocket>(socket: &T, interval_sec: i32) -> bool {
    ensure_wsa_startup();
    let s = socket.as_raw_socket();
    let enable: i32 = if interval_sec > 0 { 1 } else { 0 };
    unsafe {
        setsockopt(
            s,
            SOL_SOCKET,
            SO_KEEPALIVE,
            &enable as *const _ as *const _,
            std::mem::size_of_val(&enable) as i32,
        ) == 0
    }
}

/// Disables TCP Nagle algorithm.
#[cfg(unix)]
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

#[cfg(windows)]
pub fn disable_tcp_nagle<T: AsRawSocket>(socket: &T) {
    ensure_wsa_startup();
    let s = socket.as_raw_socket();
    let off: i32 = 1;
    unsafe {
        setsockopt(
            s,
            IPPROTO_TCP as _,
            TCP_NODELAY as _,
            &off as *const _ as *const _,
            std::mem::size_of_val(&off) as i32,
        );
    }
}

/// Checks if the file descriptor is a terminal.
#[cfg(unix)]
pub fn unix_isatty<T: AsRawFd>(fd: &T) -> bool {
    unsafe { libc::isatty(fd.as_raw_fd()) == 1 }
}

#[cfg(windows)]
pub fn unix_isatty<T: AsRawSocket>(_fd: &T) -> bool {
    false
}

/// Peeks at the next message size in a socket.
#[cfg(unix)]
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

#[cfg(windows)]
pub fn network_peek<T: AsRawSocket>(socket: &T) -> Option<isize> {
    ensure_wsa_startup();
    let s = socket.as_raw_socket();
    let mut available: i32 = 0;
    unsafe {
        if ioctlsocket(s, FIONREAD, &mut available) == 0 {
            Some(available as isize)
        } else {
            None
        }
    }
}

use crate::AdbFd;

pub fn adb_socketpair() -> std::io::Result<(AdbFd, AdbFd)> {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let (s1, s2) = UnixStream::pair()?;
        Ok((AdbFd::from(OwnedFd::from(s1)), AdbFd::from(OwnedFd::from(s2))))
    }
    #[cfg(windows)]
    {
        ensure_wsa_startup();
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let s1 = std::net::TcpStream::connect(addr)?;
        let (s2, _) = listener.accept()?;
        s1.set_nodelay(true)?;
        s2.set_nodelay(true)?;
        use std::os::windows::io::IntoRawSocket;
        let s1_owned = unsafe { OwnedSocket::from_raw_socket(s1.into_raw_socket()) };
        let s2_owned = unsafe { OwnedSocket::from_raw_socket(s2.into_raw_socket()) };
        Ok((AdbFd::from(s1_owned), AdbFd::from(s2_owned)))
    }
}
