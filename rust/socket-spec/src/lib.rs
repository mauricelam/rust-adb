//! ADB socket specification parsing and connection utilities.
//! Ported from `socket_spec.cpp`, `socket_spec.h`, and `sysdeps/posix/network.cpp`.

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{FromRawSocket, OwnedSocket};

use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;

/// Ported from original/adb.h: #define DEFAULT_ADB_LOCAL_TRANSPORT_PORT 5555
pub const DEFAULT_ADB_LOCAL_TRANSPORT_PORT: i32 = 5555;

/// Ported from original/socket_spec.cpp: bool gListenAll = false;
pub static G_LISTEN_ALL: AtomicBool = AtomicBool::new(false);

/// Errors that can occur during socket specification parsing or connection.
#[derive(Error, Debug)]
pub enum SocketSpecError {
    /// Generic error with a message.
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

        let n: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &n as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );

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

/// Parses a TCP socket specification string.
/// Ported from `parse_tcp_socket_spec` in `socket_spec.cpp`.
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

/// Returns the port number from a host socket specification string.
/// Ported from `get_host_socket_spec_port` in `socket_spec.cpp`.
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

/// Checks if a string is a valid socket specification.
/// Ported from `is_socket_spec` in `socket_spec.cpp`.
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

/// Checks if a socket specification refers to a local socket.
/// Ported from `is_local_socket_spec` in `socket_spec.cpp`.
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

/// Connects to a socket described by a specification string.
/// Ported from `socket_spec_connect` in `socket_spec.cpp`.
pub fn socket_spec_connect(
    address: &str,
    port: Option<&mut i32>,
    serial: Option<&mut String>,
) -> Result<NativeOwnedHandle, String> {
    if address.starts_with("tcp:") {
        let (hostname, port_value, serial_value) = parse_tcp_socket_spec(address)?;
        let stream = if tcp_host_is_local(&hostname) {
            TcpStream::connect(format!("127.0.0.1:{}", port_value))
        } else {
            TcpStream::connect(format!("{}:{}", hostname, port_value))
        }.map_err(|e| e.to_string())?;

        #[cfg(unix)]
        {
            use std::os::unix::io::IntoRawFd;
            let fd = unsafe { OwnedFd::from_raw_fd(stream.into_raw_fd()) };
            let keepalive_interval = std::env::var("ADB_TCP_KEEPALIVE_INTERVAL")
                .ok()
                .and_then(|v| v.parse::<i32>().ok())
                .unwrap_or(1);
            let _ = sysdeps::net::set_tcp_keepalive(&fd, keepalive_interval);
            sysdeps::net::disable_tcp_nagle(&fd);
            if let Some(p) = port { *p = port_value; }
            if let Some(s) = serial { *s = serial_value; }
            return Ok(fd);
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::IntoRawSocket;
            let s = unsafe { OwnedSocket::from_raw_socket(stream.into_raw_socket() as _) };
            let _ = sysdeps::net::set_tcp_keepalive(&s, 1);
            sysdeps::net::disable_tcp_nagle(&s);
            if let Some(p) = port { *p = port_value; }
            if let Some(s) = serial { *s = serial_value; }
            return Ok(s);
        }
    }

    #[cfg(unix)]
    {
        if address.starts_with("vsock:") {
            #[cfg(target_os = "linux")]
            {
                let parts: Vec<&str> = address.split(':').collect();
                let mut port_value = if let Some(p) = port.as_ref() { **p as u32 } else { 0 };
                let cid: u32;
                if parts.len() == 2 { cid = parts[1].parse::<u32>().map_err(|_| format!("could not parse vsock cid in '{}'", address))?; }
                else if parts.len() == 3 { cid = parts[1].parse::<u32>().map_err(|_| format!("could not parse vsock cid in '{}'", address))?; port_value = parts[2].parse::<u32>().map_err(|_| format!("could not parse vsock port in '{}'", address))?; }
                else { return Err(format!("expected vsock:cid or vsock:cid:port in '{}'", address)); }
                if port_value == 0 { return Err(format!("vsock port was not provided.")); }
                unsafe {
                    let raw_fd = libc::socket(AF_VSOCK, libc::SOCK_STREAM | SOCK_CLOEXEC, 0);
                    if raw_fd < 0 { return Err("could not open vsock socket".to_string()); }
                    let mut addr: sockaddr_vm = std::mem::zeroed();
                    addr.svm_family = AF_VSOCK as libc::sa_family_t;
                    addr.svm_port = port_value; addr.svm_cid = cid;
                    if libc::connect(raw_fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<sockaddr_vm>() as libc::socklen_t) != 0 {
                        let err = std::io::Error::last_os_error().to_string();
                        libc::close(raw_fd);
                        return Err(format!("could not connect to vsock address '{}': {}", address, err));
                    }
                    if let Some(p) = port { *p = port_value as i32; }
                    if let Some(s) = serial { *s = format!("vsock:{}:{}", cid, port_value); }
                    return Ok(OwnedFd::from_raw_fd(raw_fd));
                }
            }
            #[cfg(not(target_os = "linux"))] { return Err("vsock is only supported on Linux".to_string()); }
        } else if address.starts_with("acceptfd:") { return Err("cannot connect to acceptfd".to_string()); }
        let local_socket_types = get_local_socket_types();
        for (name, namespace_id, available) in local_socket_types {
            let prefix = format!("{}:", name);
            if address.starts_with(&prefix) {
                if !available { return Err(format!("socket type {} is unavailable on this platform", name)); }
                let fd = network_local_client(&address[prefix.len()..], namespace_id, libc::SOCK_STREAM)?;
                if let Some(s) = serial { *s = address.to_string(); }
                return Ok(fd);
            }
        }
    }

    #[cfg(windows)]
    {
        let local_socket_types = get_local_socket_types();
        for (name, _, _) in local_socket_types {
            let prefix = format!("{}:", name);
            if address.starts_with(&prefix) {
                return Err(format!("socket type {} is unavailable on Windows", name));
            }
        }
    }

    Err(format!("unknown socket specification: {}", address))
}

/// Listens on a socket described by a specification string.
/// Ported from `socket_spec_listen` in `socket_spec.cpp`.
pub fn socket_spec_listen(spec: &str, resolved_port: Option<&mut i32>) -> Result<NativeOwnedHandle, String> {
    if spec.starts_with("tcp:") {
        let (hostname, port, _) = parse_tcp_socket_spec(spec)?;
        let addr = if hostname.is_empty() { if IS_HOST && G_LISTEN_ALL.load(Ordering::Relaxed) { "0.0.0.0" } else { "127.0.0.1" } }
        else if hostname == "localhost" { "127.0.0.1" }
        else { &hostname };
        let listener = std::net::TcpListener::bind(format!("{}:{}", addr, port)).map_err(|e| e.to_string())?;
        let local_port = listener.local_addr().unwrap().port() as i32;
        if let Some(p) = resolved_port { *p = local_port; }
        #[cfg(unix)] { use std::os::unix::io::IntoRawFd; return Ok(unsafe { OwnedFd::from_raw_fd(listener.into_raw_fd()) }); }
        #[cfg(windows)] { use std::os::windows::io::IntoRawSocket; return Ok(unsafe { OwnedSocket::from_raw_socket(listener.into_raw_socket() as _) }); }
    }

    #[cfg(unix)]
    {
        if spec.starts_with("vsock:") {
            #[cfg(target_os = "linux")]
            {
                let parts: Vec<&str> = spec.split(':').collect();
                if parts.len() != 2 { return Err("given vsock server socket string was invalid".to_string()); }
                let port = parts[1].parse::<i32>().map_err(|_| "could not parse vsock port".to_string())?;
                if port < 0 { return Err("vsock port was negative.".to_string()); }
                unsafe {
                    let raw_fd = libc::socket(AF_VSOCK, libc::SOCK_STREAM | SOCK_CLOEXEC, 0);
                    if raw_fd < 0 { return Err(format!("could not create vsock server: {}", std::io::Error::last_os_error())); }
                    let mut addr: sockaddr_vm = std::mem::zeroed();
                    addr.svm_family = AF_VSOCK as libc::sa_family_t;
                    addr.svm_port = if port == 0 { VMADDR_PORT_ANY } else { port as u32 };
                    addr.svm_cid = VMADDR_CID_ANY;
                    if libc::bind(raw_fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of::<sockaddr_vm>() as libc::socklen_t) != 0 { let err = std::io::Error::last_os_error().to_string(); libc::close(raw_fd); return Err(err); }
                    if libc::listen(raw_fd, 4) != 0 { let err = std::io::Error::last_os_error().to_string(); libc::close(raw_fd); return Err(err); }
                    if let Some(p) = resolved_port {
                        let mut addr_len = std::mem::size_of::<sockaddr_vm>() as libc::socklen_t;
                        if libc::getsockname(raw_fd, &mut addr as *mut _ as *mut libc::sockaddr, &mut addr_len) == 0 { *p = addr.svm_port as i32; }
                        else { libc::close(raw_fd); return Err(std::io::Error::last_os_error().to_string()); }
                    }
                    return Ok(OwnedFd::from_raw_fd(raw_fd));
                }
            }
            #[cfg(not(target_os = "linux"))] { return Err("vsock is only supported on linux".to_string()); }
        } else if spec.starts_with("acceptfd:") {
            let fd_str = &spec["acceptfd:".len()..];
            let fd = fd_str.parse::<i32>().map_err(|_| "invalid fd".to_string())?;
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 { return Err(format!("could not get flags of inherited fd {}: {}", fd, std::io::Error::last_os_error())); }
                if (flags & libc::FD_CLOEXEC) != 0 { return Err(format!("fd {} was not inherited from parent", fd)); }
                let new_fd = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0);
                if new_fd < 0 { return Err(format!("could not dup inherited fd {}: {}", fd, std::io::Error::last_os_error())); }
                return Ok(OwnedFd::from_raw_fd(new_fd));
            }
        }
        let local_socket_types = get_local_socket_types();
        for (name, namespace_id, available) in local_socket_types {
            let prefix = format!("{}:", name);
            if spec.starts_with(&prefix) {
                if !available { return Err(format!("attempted to listen on unavailable socket type: {}", spec)); }
                return network_local_server(&spec[prefix.len()..], namespace_id, libc::SOCK_STREAM);
            }
        }
    }

    #[cfg(windows)]
    {
        let local_socket_types = get_local_socket_types();
        for (name, _, _) in local_socket_types {
            let prefix = format!("{}:", name);
            if spec.starts_with(&prefix) {
                return Err(format!("socket type {} is unavailable on Windows", name));
            }
        }
    }

    Err(format!("unknown socket specification: {}", spec))
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
    }

    #[test]
    fn parse_tcp_socket_spec_ipv6_no_port_success() {
        let (hostname, port, serial) = parse_tcp_socket_spec("tcp:::1").unwrap();
        assert_eq!("::1", hostname);
        assert_eq!(5555, port);
        assert_eq!("[::1]:5555", serial);
    }

    #[test]
    fn parse_tcp_socket_spec_ipv6_bad_ports_failure() {
        assert!(parse_tcp_socket_spec("tcp:[::1]:").is_err());
        assert!(parse_tcp_socket_spec("tcp:[::1]:-1").is_err());
    }

    #[test]
    fn get_host_socket_spec_port_success() {
        assert_eq!(5555, get_host_socket_spec_port("tcp:5555").unwrap());
        assert_eq!(5555, get_host_socket_spec_port("tcp:localhost:5555").unwrap());
    }

    #[test]
    fn get_host_socket_spec_port_no_port() {
        assert_eq!(5555, get_host_socket_spec_port("tcp:localhost").unwrap());
    }

    #[test]
    fn socket_spec_listen_connect_tcp() {
        let mut port = 0;
        let mut serial = String::new();
        let _server_fd = socket_spec_listen("tcp:127.0.0.1:0", Some(&mut port)).unwrap();
        let _client_fd = socket_spec_connect(&format!("tcp:127.0.0.1:{}", port), None, Some(&mut serial)).unwrap();
        assert_eq!(serial, format!("127.0.0.1:{}", port));
    }

    #[test]
    fn socket_spec_connect_failure() {
        assert!(socket_spec_connect("tcp:", None, None).is_err());
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
        assert!(is_socket_spec("local:blah"));
    }

    #[test]
    fn test_is_local_socket_spec() {
        assert!(is_local_socket_spec("local:blah"));
        assert!(is_local_socket_spec("tcp:localhost"));
        assert!(!is_local_socket_spec("tcp:1.2.3.4"));
    }
}
