pub mod env;
pub mod errno;
pub mod net;
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
    fn from(fd: OwnedFd) -> Self {
        Self::File(std::fs::File::from(fd))
    }
}

#[cfg(windows)]
impl From<std::fs::File> for AdbFd {
    fn from(f: std::fs::File) -> Self {
        Self::File(f)
    }
}

#[cfg(windows)]
impl From<std::net::TcpStream> for AdbFd {
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
    #[cfg(unix)]
    pub unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::File(std::fs::File::from_raw_fd(fd))
    }

    /// Creates an `AdbFd` from a raw handle (Windows only).
    #[cfg(windows)]
    pub unsafe fn from_raw_handle(h: RawHandle) -> Self {
        Self::File(std::fs::File::from_raw_handle(h))
    }

    /// Creates an `AdbFd` from a raw socket (Windows only).
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
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(f) => f.as_raw_fd(),
            _ => -1,
        }
    }
}

#[cfg(unix)]
impl IntoRawFd for AdbFd {
    fn into_raw_fd(self) -> RawFd {
        match self {
            Self::File(f) => f.into_raw_fd(),
            _ => -1,
        }
    }
}

#[cfg(windows)]
impl AsRawHandle for AdbFd {
    fn as_raw_handle(&self) -> RawHandle {
        match self {
            Self::File(f) => f.as_raw_handle(),
            _ => std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
impl IntoRawHandle for AdbFd {
    fn into_raw_handle(self) -> RawHandle {
        match self {
            Self::File(f) => f.into_raw_handle(),
            _ => std::ptr::null_mut(),
        }
    }
}

#[cfg(windows)]
impl AsRawSocket for AdbFd {
    fn as_raw_socket(&self) -> RawSocket {
        match self {
            Self::Socket(s) => s.as_raw_socket(),
            _ => windows_sys::Win32::Networking::WinSock::INVALID_SOCKET as RawSocket,
        }
    }
}

#[cfg(windows)]
impl IntoRawSocket for AdbFd {
    fn into_raw_socket(self) -> RawSocket {
        match self {
            Self::Socket(s) => s.into_raw_socket(),
            _ => windows_sys::Win32::Networking::WinSock::INVALID_SOCKET as RawSocket,
        }
    }
}

impl Read for AdbFd {
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
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::File(f) => f.write(buf),
            #[cfg(windows)]
            Self::Socket(s) => s.write(buf),
            Self::None => Err(std::io::Error::new(std::io::ErrorKind::Other, "Closed")),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::File(f) => f.flush(),
            #[cfg(windows)]
            Self::Socket(s) => s.flush(),
            Self::None => Ok(()),
        }
    }
}
