//! adb-socket-spec crate
//! Ported from original/socket_spec.cpp, original/socket_spec.h, and original/sysdeps/posix/network.cpp.

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket, OwnedSocket, RawSocket};

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

/// Ported from original/adb.h: #define DEFAULT_ADB_LOCAL_TRANSPORT_PORT 5555
pub const DEFAULT_ADB_LOCAL_TRANSPORT_PORT: i32 = 5555;

/// Ported from original/socket_spec.cpp: bool gListenAll = false;
pub static G_LISTEN_ALL: AtomicBool = AtomicBool::new(false);

#[derive(Error, Debug)]
pub enum SocketSpecError {
    #[error("{0}")]
    Error(String),
}

/// Ported from original/adb.h or similar constants
const ANDROID_SOCKET_NAMESPACE_ABSTRACT: i32 = 0;
const ANDROID_SOCKET_NAMESPACE_RESERVED: i32 = 1;
const ANDROID_SOCKET_NAMESPACE_FILESYSTEM: i32 = 2;

/// Ported from original/socket_spec.cpp: toggled via ADB_HOST macro
const IS_HOST: bool = cfg!(feature = "host");

#[cfg(target_os = "linux")]
const AF_VSOCK: libc::c_int = 40;
#[cfg(target_os = "linux")]
const VMADDR_CID_ANY: libc::c_uint = 0xFFFFFFFF;
#[cfg(target_os = "linux")]
const VMADDR_PORT_ANY: libc::c_uint = 0xFFFFFFFF;

#[cfg(target_os = "linux")]
const SOCK_CLOEXEC: libc::c_int = libc::SOCK_CLOEXEC;
#[cfg(not(target_os = "linux"))]
const SOCK_CLOEXEC: libc::c_int = 0;

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Default, Copy, Clone)]
struct sockaddr_vm {
    svm_family: libc::sa_family_t,
    svm_reserved1: libc::c_ushort,
    svm_port: libc::c_uint,
    svm_cid: libc::c_uint,
    svm_zero: [u8; 4],
}

/// Ported from original/sysdeps.h: static inline void close_on_exec(borrowed_fd fd)
#[cfg(unix)]
fn close_on_exec(fd: RawFd) {
    // SAFETY: Calling fcntl with valid FD to set FD_CLOEXEC.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags >= 0 && (flags & libc::FD_CLOEXEC) == 0 {
            libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
        }
    }
}

/// Ported from original/sysdeps_unix.cpp: bool set_tcp_keepalive(borrowed_fd fd, int interval_sec)
#[cfg(unix)]
fn set_tcp_keepalive(fd: RawFd, interval_sec: i32) -> Result<(), String> {
    let enable: libc::c_int = if interval_sec > 0 { 1 } else { 0 };
    // SAFETY: Setting SO_KEEPALIVE on a valid socket.
    unsafe {
        if libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_KEEPALIVE,
            &enable as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        ) != 0
        {
            return Err(format!(
                "setsockopt(SO_KEEPALIVE) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    if enable == 0 {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    const TCP_KEEPIDLE: libc::c_int = libc::TCP_KEEPIDLE;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const TCP_KEEPIDLE: libc::c_int = libc::TCP_KEEPALIVE;

    // SAFETY: Platform-specific setsockopt calls on valid socket.
    unsafe {
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
        if libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            TCP_KEEPIDLE,
            &interval_sec as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        ) != 0
        {}

        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "ios"))]
        {
            if libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPINTVL,
                &interval_sec as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            ) != 0
            {}

            let keepcnt: libc::c_int = 10;
            if libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                libc::TCP_KEEPCNT,
                &keepcnt as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            ) != 0
            {}
        }
    }

    Ok(())
}

/// Ported from original/sysdeps.h: static inline void disable_tcp_nagle(borrowed_fd fd)
#[cfg(unix)]
fn disable_tcp_nagle(fd: RawFd) -> Result<(), String> {
    let on: libc::c_int = 1;
    // SAFETY: Setting TCP_NODELAY on a valid socket.
    unsafe {
        if libc::setsockopt(
            fd,
            libc::IPPROTO_TCP,
            libc::TCP_NODELAY,
            &on as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        ) != 0
        {
            return Err(format!(
                "setsockopt(TCP_NODELAY) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

/// Ported from original/sysdeps.h: int adb_socket_get_local_port(borrowed_fd fd)
#[cfg(unix)]
fn adb_socket_get_local_port(fd: RawFd) -> i32 {
    // SAFETY: Calling getsockname with valid FD and buffer.
    unsafe {
        let mut addr: libc::sockaddr_storage = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        if libc::getsockname(fd, &mut addr as *mut _ as *mut libc::sockaddr, &mut len) == 0 {
            if addr.ss_family == libc::AF_INET as libc::sa_family_t {
                let addr_in = &addr as *const _ as *const libc::sockaddr_in;
                return u16::from_be((*addr_in).sin_port) as i32;
            } else if addr.ss_family == libc::AF_INET6 as libc::sa_family_t {
                let addr_in6 = &addr as *const _ as *const libc::sockaddr_in6;
                return u16::from_be((*addr_in6).sin6_port) as i32;
            }
        }
    }
    -1
}

/// Ported from original/sysdeps/posix/network.cpp: static int _network_loopback_client(bool ipv6, int port, int type, std::string* error)
#[cfg(unix)]
fn _network_loopback_client(ipv6: bool, port: i32, _type: libc::c_int) -> Result<OwnedFd, String> {
    // SAFETY: Creating socket and connecting it.
    unsafe {
        let fd = libc::socket(
            if ipv6 { libc::AF_INET6 } else { libc::AF_INET },
            _type | SOCK_CLOEXEC,
            0,
        );
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        close_on_exec(fd);

        let mut addr_storage: libc::sockaddr_storage = std::mem::zeroed();
        let addrlen: libc::socklen_t;
        if ipv6 {
            let addr = &mut addr_storage as *mut _ as *mut libc::sockaddr_in6;
            (*addr).sin6_family = libc::AF_INET6 as libc::sa_family_t;
            (*addr).sin6_addr = libc::in6addr_loopback;
            (*addr).sin6_port = 0;
            addrlen = std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t;
        } else {
            let addr = &mut addr_storage as *mut _ as *mut libc::sockaddr_in;
            (*addr).sin_family = libc::AF_INET as libc::sa_family_t;
            (*addr).sin_addr.s_addr = libc::INADDR_LOOPBACK.to_be();
            (*addr).sin_port = 0;
            addrlen = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        }

        if libc::bind(
            fd,
            &addr_storage as *const _ as *const libc::sockaddr,
            addrlen,
        ) != 0
        {
            let err = std::io::Error::last_os_error().to_string();
            libc::close(fd);
            return Err(err);
        }

        if ipv6 {
            let addr = &mut addr_storage as *mut _ as *mut libc::sockaddr_in6;
            (*addr).sin6_port = (port as u16).to_be();
        } else {
            let addr = &mut addr_storage as *mut _ as *mut libc::sockaddr_in;
            (*addr).sin_port = (port as u16).to_be();
        }

        if libc::connect(
            fd,
            &addr_storage as *const _ as *const libc::sockaddr,
            addrlen,
        ) != 0
        {
            let err = std::io::Error::last_os_error().to_string();
            libc::close(fd);
            return Err(err);
        }

        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Ported from original/sysdeps/posix/network.cpp: int network_loopback_client(int port, int type, std::string* error)
#[cfg(unix)]
fn network_loopback_client(port: i32, _type: libc::c_int) -> Result<OwnedFd, String> {
    _network_loopback_client(false, port, _type)
        .or_else(|_| _network_loopback_client(true, port, _type))
}

/// Ported from original/sysdeps/posix/network.cpp: static int _network_loopback_server(bool ipv6, int port, int type, std::string* error)
#[cfg(unix)]
fn _network_loopback_server(ipv6: bool, port: i32, _type: libc::c_int) -> Result<OwnedFd, String> {
    // SAFETY: Creating socket and binding it for server.
    unsafe {
        let fd = libc::socket(
            if ipv6 { libc::AF_INET6 } else { libc::AF_INET },
            _type | SOCK_CLOEXEC,
            0,
        );
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        close_on_exec(fd);

        let n: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &n as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );

        let mut addr_storage: libc::sockaddr_storage = std::mem::zeroed();
        let addrlen: libc::socklen_t;
        if ipv6 {
            let addr = &mut addr_storage as *mut _ as *mut libc::sockaddr_in6;
            (*addr).sin6_family = libc::AF_INET6 as libc::sa_family_t;
            (*addr).sin6_addr = libc::in6addr_loopback;
            (*addr).sin6_port = (port as u16).to_be();
            addrlen = std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t;
        } else {
            let addr = &mut addr_storage as *mut _ as *mut libc::sockaddr_in;
            (*addr).sin_family = libc::AF_INET as libc::sa_family_t;
            (*addr).sin_addr.s_addr = libc::INADDR_LOOPBACK.to_be();
            (*addr).sin_port = (port as u16).to_be();
            addrlen = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        }

        if libc::bind(
            fd,
            &addr_storage as *const _ as *const libc::sockaddr,
            addrlen,
        ) != 0
        {
            let err = std::io::Error::last_os_error().to_string();
            libc::close(fd);
            return Err(err);
        }

        if _type == libc::SOCK_STREAM || _type == libc::SOCK_SEQPACKET {
            if libc::listen(fd, libc::SOMAXCONN) != 0 {
                let err = std::io::Error::last_os_error().to_string();
                libc::close(fd);
                return Err(err);
            }
        }

        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Ported from original/sysdeps/posix/network.cpp: int network_loopback_server(int port, int type, std::string* error, bool prefer_ipv4)
#[cfg(unix)]
fn network_loopback_server(
    port: i32,
    _type: libc::c_int,
    prefer_ipv4: bool,
) -> Result<OwnedFd, String> {
    if prefer_ipv4 {
        if let Ok(fd) = _network_loopback_server(false, port, _type) {
            return Ok(fd);
        }
    }
    _network_loopback_server(true, port, _type)
}

/// Ported from original/sysdeps.h: int network_inaddr_any_server(int port, int type, std::string* error)
#[cfg(unix)]
fn network_inaddr_any_server(port: i32, _type: libc::c_int) -> Result<OwnedFd, String> {
    // SAFETY: Binding to INADDR_ANY for server.
    unsafe {
        let fd = libc::socket(libc::AF_INET, _type | SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        close_on_exec(fd);

        let n: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &n as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );

        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as libc::sa_family_t;
        addr.sin_addr.s_addr = libc::INADDR_ANY.to_be();
        addr.sin_port = (port as u16).to_be();

        if libc::bind(
            fd,
            &addr as *const _ as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        ) != 0
        {
            let err = std::io::Error::last_os_error().to_string();
            libc::close(fd);
            return Err(err);
        }

        if _type == libc::SOCK_STREAM || _type == libc::SOCK_SEQPACKET {
            if libc::listen(fd, libc::SOMAXCONN) != 0 {
                let err = std::io::Error::last_os_error().to_string();
                libc::close(fd);
                return Err(err);
            }
        }

        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Ported from original/sysdeps.h: inline int network_local_client(const char* name, int namespace_id, int type, std::string* error)
#[cfg(unix)]
fn network_local_client(
    name: &str,
    namespace_id: i32,
    _type: libc::c_int,
) -> Result<OwnedFd, String> {
    let mut path = name.to_string();
    if namespace_id == ANDROID_SOCKET_NAMESPACE_RESERVED {
        path = format!("/dev/socket/{}", name);
    }

    // SAFETY: Setting up sockaddr_un and connecting.
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, _type | SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        close_on_exec(fd);

        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

        let path_bytes = path.as_bytes();
        let sun_path_len = addr.sun_path.len();

        let len: libc::socklen_t;
        if namespace_id == ANDROID_SOCKET_NAMESPACE_ABSTRACT {
            if path_bytes.len() + 1 > sun_path_len {
                libc::close(fd);
                return Err("path too long".to_string());
            }
            std::ptr::copy_nonoverlapping(
                path_bytes.as_ptr(),
                addr.sun_path.as_mut_ptr().offset(1) as *mut u8,
                path_bytes.len(),
            );
            len = (std::mem::size_of::<libc::sa_family_t>() + 1 + path_bytes.len())
                as libc::socklen_t;
        } else {
            if path_bytes.len() >= sun_path_len {
                libc::close(fd);
                return Err("path too long".to_string());
            }
            std::ptr::copy_nonoverlapping(
                path_bytes.as_ptr(),
                addr.sun_path.as_mut_ptr() as *mut u8,
                path_bytes.len(),
            );
            len = (std::mem::size_of::<libc::sa_family_t>() + path_bytes.len() + 1)
                as libc::socklen_t;
        }

        if libc::connect(fd, &addr as *const _ as *const libc::sockaddr, len) != 0 {
            let err = std::io::Error::last_os_error().to_string();
            libc::close(fd);
            return Err(err);
        }

        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Ported from original/sysdeps.h: inline int network_local_server(const char* name, int namespace_id, int type, std::string* error)
#[cfg(unix)]
fn network_local_server(
    name: &str,
    namespace_id: i32,
    _type: libc::c_int,
) -> Result<OwnedFd, String> {
    let mut path = name.to_string();
    if namespace_id == ANDROID_SOCKET_NAMESPACE_RESERVED {
        path = format!("/dev/socket/{}", name);
    }

    if namespace_id == ANDROID_SOCKET_NAMESPACE_FILESYSTEM {
        let _ = std::fs::remove_file(&path);
    }

    // SAFETY: Binding Unix domain socket.
    unsafe {
        let fd = libc::socket(libc::AF_UNIX, _type | SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        close_on_exec(fd);

        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        addr.sun_family = libc::AF_UNIX as libc::sa_family_t;

        let path_bytes = path.as_bytes();
        let sun_path_len = addr.sun_path.len();

        let len: libc::socklen_t;
        if namespace_id == ANDROID_SOCKET_NAMESPACE_ABSTRACT {
            if path_bytes.len() + 1 > sun_path_len {
                libc::close(fd);
                return Err("path too long".to_string());
            }
            std::ptr::copy_nonoverlapping(
                path_bytes.as_ptr(),
                addr.sun_path.as_mut_ptr().offset(1) as *mut u8,
                path_bytes.len(),
            );
            len = (std::mem::size_of::<libc::sa_family_t>() + 1 + path_bytes.len())
                as libc::socklen_t;
        } else {
            if path_bytes.len() >= sun_path_len {
                libc::close(fd);
                return Err("path too long".to_string());
            }
            std::ptr::copy_nonoverlapping(
                path_bytes.as_ptr(),
                addr.sun_path.as_mut_ptr() as *mut u8,
                path_bytes.len(),
            );
            len = (std::mem::size_of::<libc::sa_family_t>() + path_bytes.len() + 1)
                as libc::socklen_t;
        }

        if libc::bind(fd, &addr as *const _ as *const libc::sockaddr, len) != 0 {
            let err = std::io::Error::last_os_error().to_string();
            libc::close(fd);
            return Err(err);
        }

        if _type == libc::SOCK_STREAM || _type == libc::SOCK_SEQPACKET {
            if libc::listen(fd, libc::SOMAXCONN) != 0 {
                let err = std::io::Error::last_os_error().to_string();
                libc::close(fd);
                return Err(err);
            }
        }

        Ok(OwnedFd::from_raw_fd(fd))
    }
}

/// Helper function to parse network addresses.
/// Ported from android::base::ParseNetAddress logic.
fn parse_net_address(addr: &str, default_port: i32) -> Result<(String, i32, String), String> {
    let host;
    let mut port = default_port;

    if addr.starts_with('[') {
        let end_bracket = addr
            .find(']')
            .ok_or_else(|| format!("missing ']' in address: {}", addr))?;
        host = addr[1..end_bracket].to_string();
        let rest = &addr[end_bracket + 1..];
        if rest.starts_with(':') {
            if rest.len() == 1 {
                return Err(format!("bad port number in: {}", addr));
            }
            port = rest[1..]
                .parse::<i32>()
                .map_err(|_| format!("bad port number in: {}", addr))?;
        } else if !rest.is_empty() {
            return Err(format!("garbage after ']': {}", rest));
        }
    } else {
        if let Some(last_colon) = addr.rfind(':') {
            let first_colon = addr.find(':').unwrap();
            if first_colon != last_colon {
                // IPv6
                host = addr.to_string();
            } else {
                // host:port
                host = addr[..last_colon].to_string();
                if last_colon + 1 == addr.len() {
                    return Err(format!("bad port number in: {}", addr));
                }
                port = addr[last_colon + 1..]
                    .parse::<i32>()
                    .map_err(|_| format!("bad port number in: {}", addr))?;
            }
        } else {
            host = addr.to_string();
        }
    }

    let serial = if host.contains(':') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    };

    Ok((host, port, serial))
}

/// Ported from original/socket_spec.cpp: bool parse_tcp_socket_spec(...)
pub fn parse_tcp_socket_spec(spec: &str) -> Result<(String, i32, String), String> {
    if !spec.starts_with("tcp:") {
        return Err(format!("specification is not tcp: {}", spec));
    }

    let remainder = &spec[4..];
    if let Ok(port) = remainder.parse::<i32>() {
        if port < 0 || port > 65535 {
            return Err(format!("bad port number '{}'", port));
        }
        return Ok(("".to_string(), port, "".to_string()));
    }

    if remainder.is_empty() {
        return Err(format!("bad port number in: {}", spec));
    }

    let (host, port, serial) = parse_net_address(remainder, DEFAULT_ADB_LOCAL_TRANSPORT_PORT)?;
    if port < 0 || port > 65535 {
        return Err(format!("bad port number '{}'", port));
    }

    Ok((host, port, serial))
}

/// Ported from original/socket_spec.cpp: int get_host_socket_spec_port(...)
pub fn get_host_socket_spec_port(spec: &str) -> Result<i32, String> {
    if spec.starts_with("tcp:") {
        let (_, port, _) = parse_tcp_socket_spec(spec)?;
        Ok(port)
    } else if spec.starts_with("vsock:") {
        let parts: Vec<&str> = spec.split(':').collect();
        if parts.len() != 2 {
            return Err("given vsock server socket string was invalid".to_string());
        }
        let port = parts[1]
            .parse::<i32>()
            .map_err(|_| "could not parse vsock port".to_string())?;
        if port < 0 {
            return Err("vsock port was negative.".to_string());
        }
        Ok(port)
    } else {
        Err("given socket spec string was invalid".to_string())
    }
}

/// Ported from original/socket_spec.cpp: bool is_socket_spec(std::string_view spec)
pub fn is_socket_spec(spec: &str) -> bool {
    let local_socket_types = get_local_socket_types();
    for (name, _, _) in local_socket_types {
        let prefix = format!("{}:", name);
        if spec.starts_with(&prefix) {
            return true;
        }
    }
    spec.starts_with("tcp:") || spec.starts_with("acceptfd:") || spec.starts_with("vsock:")
}

/// Helper function to check if a TCP host is local.
fn tcp_host_is_local(hostname: &str) -> bool {
    hostname.is_empty()
        || hostname == "localhost"
        || hostname == "127.0.0.1"
        || hostname == "::1"
        || hostname == "::ffff:127.0.0.1"
}

/// Ported from original/socket_spec.cpp: bool is_local_socket_spec(std::string_view spec)
pub fn is_local_socket_spec(spec: &str) -> bool {
    let local_socket_types = get_local_socket_types();
    for (name, _, _) in local_socket_types {
        let prefix = format!("{}:", name);
        if spec.starts_with(&prefix) {
            return true;
        }
    }

    if let Ok((hostname, _, _)) = parse_tcp_socket_spec(spec) {
        return tcp_host_is_local(&hostname);
    }
    false
}

/// Ported from original/socket_spec.cpp: static auto& kLocalSocketTypes = ...
fn get_local_socket_types() -> Vec<(&'static str, i32, bool)> {
    vec![
        (
            "local",
            if IS_HOST {
                ANDROID_SOCKET_NAMESPACE_FILESYSTEM
            } else {
                ANDROID_SOCKET_NAMESPACE_RESERVED
            },
            cfg!(unix),
        ),
        (
            "localreserved",
            ANDROID_SOCKET_NAMESPACE_RESERVED,
            cfg!(unix) && !IS_HOST,
        ),
        (
            "localabstract",
            ANDROID_SOCKET_NAMESPACE_ABSTRACT,
            cfg!(target_os = "linux"),
        ),
        (
            "localfilesystem",
            ANDROID_SOCKET_NAMESPACE_FILESYSTEM,
            cfg!(unix),
        ),
    ]
}

/// Cross-platform Owned handle type
#[cfg(unix)]
pub type NativeOwnedHandle = OwnedFd;
#[cfg(windows)]
pub type NativeOwnedHandle = OwnedSocket;

/// Ported from original/socket_spec.cpp: bool socket_spec_connect(...)
pub fn socket_spec_connect(
    address: &str,
    port: Option<&mut i32>,
    serial: Option<&mut String>,
) -> Result<NativeOwnedHandle, String> {
    #[cfg(unix)]
    {
        if address.starts_with("tcp:") {
            let (hostname, port_value, serial_value) = parse_tcp_socket_spec(address)?;
            let fd = if tcp_host_is_local(&hostname) {
                network_loopback_client(port_value, libc::SOCK_STREAM)?
            } else {
                // network_connect
                let addrs = format!("{}:{}", hostname, port_value)
                    .to_socket_addrs()
                    .map_err(|e| e.to_string())?;
                let mut last_err = format!("failed to connect to {}:{}", hostname, port_value);
                let mut fd = None;
                for addr in addrs {
                    match TcpStream::connect(addr) {
                        Ok(stream) => {
                            let raw_fd = stream.as_raw_fd();
                            std::mem::forget(stream);
                            // SAFETY: We forget the stream, taking ownership of the raw FD.
                            fd = Some(unsafe { OwnedFd::from_raw_fd(raw_fd) });
                            break;
                        }
                        Err(e) => last_err = e.to_string(),
                    }
                }
                fd.ok_or(last_err)?
            };

            let keepalive_interval = std::env::var("ADB_TCP_KEEPALIVE_INTERVAL")
                .ok()
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(1);

            set_tcp_keepalive(fd.as_raw_fd(), keepalive_interval)?;
            disable_tcp_nagle(fd.as_raw_fd())?;

            if let Some(p) = port {
                *p = port_value;
            }
            if let Some(s) = serial {
                *s = serial_value;
            }
            return Ok(fd);
        } else if address.starts_with("vsock:") {
            #[cfg(target_os = "linux")]
            {
                let parts: Vec<&str> = address.split(':').collect();
                let mut port_value = if let Some(p) = port.as_ref() {
                    **p as u32
                } else {
                    0
                };
                let cid: u32;
                if parts.len() == 2 {
                    cid = parts[1]
                        .parse::<u32>()
                        .map_err(|_| format!("could not parse vsock cid in '{}'", address))?;
                } else if parts.len() == 3 {
                    cid = parts[1]
                        .parse::<u32>()
                        .map_err(|_| format!("could not parse vsock cid in '{}'", address))?;
                    port_value = parts[2]
                        .parse::<u32>()
                        .map_err(|_| format!("could not parse vsock port in '{}'", address))?;
                } else {
                    return Err(format!(
                        "expected vsock:cid or vsock:cid:port in '{}'",
                        address
                    ));
                }
                if port_value == 0 {
                    return Err(format!("vsock port was not provided."));
                }

                // SAFETY: Manual socket creation and connection for AF_VSOCK.
                unsafe {
                    let raw_fd = libc::socket(AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
                    if raw_fd < 0 {
                        return Err("could not open vsock socket".to_string());
                    }
                    let mut addr: sockaddr_vm = std::mem::zeroed();
                    addr.svm_family = AF_VSOCK as libc::sa_family_t;
                    addr.svm_port = port_value;
                    addr.svm_cid = cid;

                    if libc::connect(
                        raw_fd,
                        &addr as *const _ as *const libc::sockaddr,
                        std::mem::size_of::<sockaddr_vm>() as libc::socklen_t,
                    ) != 0
                    {
                        let err = std::io::Error::last_os_error().to_string();
                        libc::close(raw_fd);
                        return Err(format!(
                            "could not connect to vsock address '{}': {}",
                            address, err
                        ));
                    }

                    if let Some(p) = port {
                        *p = port_value as i32;
                    }
                    if let Some(s) = serial {
                        *s = format!("vsock:{}:{}", cid, port_value);
                    }
                    return Ok(OwnedFd::from_raw_fd(raw_fd));
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                return Err("vsock is only supported on Linux".to_string());
            }
        } else if address.starts_with("acceptfd:") {
            return Err("cannot connect to acceptfd".to_string());
        }

        let local_socket_types = get_local_socket_types();
        for (name, namespace_id, available) in local_socket_types {
            let prefix = format!("{}:", name);
            if address.starts_with(&prefix) {
                if !available {
                    return Err(format!(
                        "socket type {} is unavailable on this platform",
                        name
                    ));
                }
                let fd = network_local_client(
                    &address[prefix.len()..],
                    namespace_id,
                    libc::SOCK_STREAM,
                )?;
                if let Some(s) = serial {
                    *s = address.to_string();
                }
                return Ok(fd);
            }
        }

        Err(format!("unknown socket specification: {}", address))
    }
    #[cfg(windows)]
    {
        Err("socket_spec_connect not implemented on Windows".to_string())
    }
}

/// Ported from original/socket_spec.cpp: int socket_spec_listen(...)
pub fn socket_spec_listen(
    spec: &str,
    resolved_port: Option<&mut i32>,
) -> Result<NativeOwnedHandle, String> {
    #[cfg(unix)]
    {
        if spec.starts_with("tcp:") {
            let (hostname, port, _) = parse_tcp_socket_spec(spec)?;
            let fd = if hostname.is_empty() {
                if IS_HOST {
                    if G_LISTEN_ALL.load(Ordering::Relaxed) {
                        network_inaddr_any_server(port, libc::SOCK_STREAM)?
                    } else {
                        network_loopback_server(port, libc::SOCK_STREAM, true)?
                    }
                } else {
                    network_inaddr_any_server(port, libc::SOCK_STREAM)?
                }
            } else if tcp_host_is_local(&hostname) {
                network_loopback_server(port, libc::SOCK_STREAM, true)?
            } else if hostname == "::1" {
                network_loopback_server(port, libc::SOCK_STREAM, false)?
            } else {
                return Err("listening on specified hostname currently unsupported".to_string());
            };

            if let Some(p) = resolved_port {
                *p = adb_socket_get_local_port(fd.as_raw_fd());
            }
            return Ok(fd);
        } else if spec.starts_with("vsock:") {
            #[cfg(target_os = "linux")]
            {
                let parts: Vec<&str> = spec.split(':').collect();
                if parts.len() != 2 {
                    return Err("given vsock server socket string was invalid".to_string());
                }
                let port = parts[1]
                    .parse::<i32>()
                    .map_err(|_| "could not parse vsock port".to_string())?;
                if port < 0 {
                    return Err("vsock port was negative.".to_string());
                }

                // SAFETY: Manual server socket creation for AF_VSOCK.
                unsafe {
                    let raw_fd = libc::socket(AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0);
                    if raw_fd < 0 {
                        return Err(format!(
                            "could not create vsock server: {}",
                            std::io::Error::last_os_error()
                        ));
                    }
                    let mut addr: sockaddr_vm = std::mem::zeroed();
                    addr.svm_family = AF_VSOCK as libc::sa_family_t;
                    addr.svm_port = if port == 0 {
                        VMADDR_PORT_ANY
                    } else {
                        port as u32
                    };
                    addr.svm_cid = VMADDR_CID_ANY;

                    if libc::bind(
                        raw_fd,
                        &addr as *const _ as *const libc::sockaddr,
                        std::mem::size_of::<sockaddr_vm>() as libc::socklen_t,
                    ) != 0
                    {
                        let err = std::io::Error::last_os_error().to_string();
                        libc::close(raw_fd);
                        return Err(err);
                    }
                    if libc::listen(raw_fd, 4) != 0 {
                        let err = std::io::Error::last_os_error().to_string();
                        libc::close(raw_fd);
                        return Err(err);
                    }

                    if let Some(p) = resolved_port {
                        let mut addr_len = std::mem::size_of::<sockaddr_vm>() as libc::socklen_t;
                        if libc::getsockname(
                            raw_fd,
                            &mut addr as *mut _ as *mut libc::sockaddr,
                            &mut addr_len,
                        ) == 0
                        {
                            *p = addr.svm_port as i32;
                        } else {
                            libc::close(raw_fd);
                            return Err(std::io::Error::last_os_error().to_string());
                        }
                    }
                    return Ok(OwnedFd::from_raw_fd(raw_fd));
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                return Err("vsock is only supported on linux".to_string());
            }
        } else if spec.starts_with("acceptfd:") {
            let fd_str = &spec["acceptfd:".len()..];
            let fd_u = fd_str
                .parse::<u32>()
                .map_err(|_| "invalid fd".to_string())?;
            let fd = fd_u as i32;

            // SAFETY: Duping inherited FD and checking its status.
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(format!(
                        "could not get flags of inherited fd {}: {}",
                        fd,
                        std::io::Error::last_os_error()
                    ));
                }
                if (flags & libc::FD_CLOEXEC) != 0 {
                    return Err(format!("fd {} was not inherited from parent", fd));
                }

                let mut sock_type: libc::c_int = 0;
                let mut sock_type_size = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
                if libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_TYPE,
                    &mut sock_type as *mut _ as *mut libc::c_void,
                    &mut sock_type_size,
                ) != 0
                {
                    return Err(format!("fd {} does not refer to a socket", fd));
                }

                let new_fd = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0);
                if new_fd < 0 {
                    return Err(format!(
                        "could not dup inherited fd {}: {}",
                        fd,
                        std::io::Error::last_os_error()
                    ));
                }
                return Ok(OwnedFd::from_raw_fd(new_fd));
            }
        }

        let local_socket_types = get_local_socket_types();
        for (name, namespace_id, available) in local_socket_types {
            let prefix = format!("{}:", name);
            if spec.starts_with(&prefix) {
                if !available {
                    return Err(format!(
                        "attempted to listen on unavailable socket type: {}",
                        spec
                    ));
                }
                return network_local_server(
                    &spec[prefix.len()..],
                    namespace_id,
                    libc::SOCK_STREAM,
                );
            }
        }

        Err(format!("unknown socket specification: {}", spec))
    }
    #[cfg(windows)]
    {
        Err("socket_spec_listen not implemented on Windows".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use tempfile::tempdir;

    #[test]
    fn parse_tcp_socket_spec_failure_error_check() {
        let spec = "sneakernet:5037";
        let res = parse_tcp_socket_spec(spec);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("sneakernet"));
        assert_eq!(err, format!("specification is not tcp: {}", spec));
    }

    #[test]
    fn parse_tcp_socket_spec_just_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:5037").unwrap();
        assert_eq!("", hostname);
        assert_eq!(5037, port);
        assert_eq!("", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_bad_ports_failure() {
        assert!(parse_tcp_socket_spec("tcp:").is_err());
        assert!(parse_tcp_socket_spec("tcp:-1").is_err());
        assert!(parse_tcp_socket_spec("tcp:65536").is_err());
    }

    #[test]
    fn parse_tcp_socket_spec_host_and_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:localhost:1234").unwrap();
        assert_eq!("localhost", hostname);
        assert_eq!(1234, port);
        assert_eq!("localhost:1234", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_host_no_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:localhost").unwrap();
        assert_eq!("localhost", hostname);
        assert_eq!(5555, port);
        assert_eq!("localhost:5555", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_host_ipv4_no_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:127.0.0.1").unwrap();
        assert_eq!("127.0.0.1", hostname);
        assert_eq!(5555, port);
        assert_eq!("127.0.0.1:5555", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_host_bad_ports_failure() {
        assert!(parse_tcp_socket_spec("tcp:localhost:").is_err());
        assert!(parse_tcp_socket_spec("tcp:localhost:-1").is_err());
        assert!(parse_tcp_socket_spec("tcp:localhost:65536").is_err());
    }

    #[test]
    fn parse_tcp_socket_spec_host_ipv4_bad_ports_failure() {
        assert!(parse_tcp_socket_spec("tcp:127.0.0.1:").is_err());
        assert!(parse_tcp_socket_spec("tcp:127.0.0.1:-1").is_err());
        assert!(parse_tcp_socket_spec("tcp:127.0.0.1:65536").is_err());
    }

    #[test]
    fn parse_tcp_socket_spec_ipv6_and_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:[::1]:1234").unwrap();
        assert_eq!("::1", hostname);
        assert_eq!(1234, port);
        assert_eq!("[::1]:1234", serial);

        let (hostname, port, serial) =
            parse_tcp_socket_spec("tcp:[2601:644:8e80:620::fbbc]:2345").unwrap();
        assert_eq!("2601:644:8e80:620::fbbc", hostname);
        assert_eq!(2345, port);
        assert_eq!("[2601:644:8e80:620::fbbc]:2345", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_ipv6_no_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:::1").unwrap();
        assert_eq!("::1", hostname);
        assert_eq!(5555, port);
        assert_eq!("[::1]:5555", serial);

        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:[::1]").unwrap();
        assert_eq!("::1", hostname);
        assert_eq!(5555, port);
        assert_eq!("[::1]:5555", serial);

        let (hostname, port, serial) =
            parse_tcp_socket_spec("tcp:2601:644:8e80:620::fbbc").unwrap();
        assert_eq!("2601:644:8e80:620::fbbc", hostname);
        assert_eq!(5555, port);
        assert_eq!("[2601:644:8e80:620::fbbc]:5555", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_ipv6_bad_ports_failure() {
        assert!(parse_tcp_socket_spec("tcp:[::1]:").is_err());
        assert!(parse_tcp_socket_spec("tcp:[::1]:-1").is_err());
    }

    #[test]
    fn get_host_socket_spec_port_success() {
        assert_eq!(5555, get_host_socket_spec_port("tcp:5555").unwrap());
        assert_eq!(
            5555,
            get_host_socket_spec_port("tcp:localhost:5555").unwrap()
        );
        assert_eq!(5555, get_host_socket_spec_port("tcp:[::1]:5555").unwrap());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn get_host_socket_spec_port_vsock_success() {
        assert_eq!(5555, get_host_socket_spec_port("vsock:5555").unwrap());
    }

    #[test]
    fn get_host_socket_spec_port_no_port() {
        assert_eq!(5555, get_host_socket_spec_port("tcp:localhost").unwrap());
        assert!(get_host_socket_spec_port("vsock:localhost").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn socket_spec_listen_connect_tcp() {
        let mut port = 0;
        let mut serial = String::new();
        let _server_fd = socket_spec_listen("tcp:localhost:0", Some(&mut port)).unwrap();
        let _client_fd =
            socket_spec_connect(&format!("tcp:localhost:{}", port), None, Some(&mut serial))
                .unwrap();
        assert_eq!(serial, format!("localhost:{}", port));
    }

    #[test]
    #[cfg(unix)]
    fn socket_spec_connect_failure() {
        assert!(socket_spec_connect("tcp:", None, None).is_err());
        assert!(socket_spec_connect("acceptfd:", None, None).is_err());
        assert!(socket_spec_connect("vsock:", None, None).is_err());
        assert!(socket_spec_connect("sneakernet:", None, None).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn socket_spec_listen_connect_localfilesystem() {
        let dir = tempdir().unwrap();
        let sock_path = dir.path().join("af_unix_socket");
        let sock_addr = format!("localfilesystem:{}", sock_path.display());

        let _server_fd = socket_spec_listen(&sock_addr, None).unwrap();
        let _client_fd = socket_spec_connect(&sock_addr, None, None).unwrap();
    }

    #[test]
    fn test_is_socket_spec() {
        assert!(is_socket_spec("tcp:blah"));
        assert!(is_socket_spec("acceptfd:blah"));
        assert!(is_socket_spec("local:blah"));
        assert!(is_socket_spec("vsock:123:456"));
    }

    #[test]
    fn test_is_local_socket_spec() {
        assert!(is_local_socket_spec("local:blah"));
        assert!(is_local_socket_spec("tcp:localhost"));
        assert!(!is_local_socket_spec("tcp:1.2.3.4"));
    }
}
