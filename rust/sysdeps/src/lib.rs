//! System-dependent abstractions for ADB.
//! Ported from `sysdeps.h`.

/// Environment variable utilities.
pub mod env;
/// System error code utilities.
pub mod errno;
/// Networking utilities.
pub mod net;
/// I/O polling utilities.
pub mod poll;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
#[cfg(windows)]
use std::os::windows::io::{
    AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, IntoRawHandle, IntoRawSocket,
    RawHandle, RawSocket,
};

use std::io::{Read, Write};

/// A unified file descriptor type for ADB.
/// Ported from `adb_fd` in `sysdeps.h`.
///
/// Abstracts over Unix file descriptors and Windows handles/sockets.
#[derive(Debug)]
pub enum AdbFd {
    /// Wrapper around a file or handle.
    File(std::fs::File),
    /// Wrapper around a network socket (Windows only).
    #[cfg(windows)]
    Socket(std::net::TcpStream),
    /// Represents an empty or closed file descriptor.
    None,
}

#[cfg(unix)]
impl From<OwnedFd> for AdbFd {
    /// Converts an `OwnedFd` into an `AdbFd`.
    fn from(fd: OwnedFd) -> Self {
        Self::File(std::fs::File::from(fd))
    }
}

#[cfg(windows)]
impl From<std::fs::File> for AdbFd {
    /// Converts a `std::fs::File` into an `AdbFd`.
    fn from(f: std::fs::File) -> Self {
        Self::File(f)
    }
}

#[cfg(windows)]
impl From<std::net::TcpStream> for AdbFd {
    /// Converts a `std::net::TcpStream` into an `AdbFd`.
    fn from(s: std::net::TcpStream) -> Self {
        Self::Socket(s)
    }
}

impl AdbFd {
    /// Attempts to clone the file descriptor.
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::File(f) => f.try_clone().map(Self::File),
            #[cfg(windows)]
            Self::Socket(s) => s.try_clone().map(Self::Socket),
            Self::None => Ok(Self::None),
        }
    }

    /// Explicitly closes the file descriptor.
    pub fn close(&mut self) {
        *self = Self::None;
    }

    /// Creates an `AdbFd` from a raw file descriptor (Unix only).
    ///
    /// # Safety
    /// The caller must ensure that the file descriptor is valid.
    #[cfg(unix)]
    pub unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::File(std::fs::File::from_raw_fd(fd))
    }

    /// Creates an `AdbFd` from a raw handle (Windows only).
    ///
    /// # Safety
    /// The caller must ensure that the handle is valid.
    #[cfg(windows)]
    pub unsafe fn from_raw_handle(h: RawHandle) -> Self {
        Self::File(std::fs::File::from_raw_handle(h))
    }

    /// Creates an `AdbFd` from a raw socket (Windows only).
    ///
    /// # Safety
    /// The caller must ensure that the socket is valid.
    #[cfg(windows)]
    pub unsafe fn from_raw_socket(s: RawSocket) -> Self {
        Self::Socket(std::net::TcpStream::from_raw_socket(s as _))
    }

    /// Converts the `AdbFd` into an `OwnedFd` (Unix only).
    #[cfg(unix)]
    pub fn try_into_owned_fd(self) -> Option<OwnedFd> {
        match self {
            Self::File(f) => Some(f.into()),
            _ => None,
        }
    }
}

#[cfg(unix)]
impl AsRawFd for AdbFd {
    /// Returns the raw file descriptor.
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(f) => f.as_raw_fd(),
            _ => -1,
        }
    }
}

#[cfg(unix)]
impl IntoRawFd for AdbFd {
    /// Consumes the `AdbFd` and returns the raw file descriptor.
    fn into_raw_fd(self) -> RawFd {
        match self {
            Self::File(f) => f.into_raw_fd(),
            _ => -1,
        }
    }
}

#[cfg(windows)]
impl AsRawHandle for AdbFd {
    /// Returns the raw handle.
    fn as_raw_handle(&self) -> RawHandle {
        match self {
            Self::File(f) => f.as_raw_handle(),
            _ => std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
impl IntoRawHandle for AdbFd {
    /// Consumes the `AdbFd` and returns the raw handle.
    fn into_raw_handle(self) -> RawHandle {
        match self {
            Self::File(f) => f.into_raw_handle(),
            _ => std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
impl AsRawSocket for AdbFd {
    /// Returns the raw socket.
    fn as_raw_socket(&self) -> RawSocket {
        match self {
            Self::Socket(s) => s.as_raw_socket(),
            _ => windows_sys::Win32::Networking::WinSock::INVALID_SOCKET as RawSocket,
        }
    }
}

#[cfg(windows)]
impl IntoRawSocket for AdbFd {
    /// Consumes the `AdbFd` and returns the raw socket.
    fn into_raw_socket(self) -> RawSocket {
        match self {
            Self::Socket(s) => s.into_raw_socket(),
            _ => windows_sys::Win32::Networking::WinSock::INVALID_SOCKET as RawSocket,
        }
    }
}

impl Read for AdbFd {
    /// Reads data from the file descriptor.
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(f) => f.read(buf),
            #[cfg(windows)]
            Self::Socket(s) => s.read(buf),
            Self::None => Err(std::io::Error::new(std::io::ErrorKind::Other, "Closed")),
        }
    }
}

impl Write for AdbFd {
    /// Writes data to the file descriptor.
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::File(f) => f.write(buf),
            #[cfg(windows)]
            Self::Socket(s) => s.write(buf),
            Self::None => Err(std::io::Error::new(std::io::ErrorKind::Other, "Closed")),
        }
    }

    /// Flushes the file descriptor.
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::File(f) => f.flush(),
            #[cfg(windows)]
            Self::Socket(s) => s.flush(),
            Self::None => Ok(()),
        }
    }
}
