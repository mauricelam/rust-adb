#[cfg(unix)]
use libc;

#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::*;
#[cfg(windows)]
use windows_sys::Win32::Foundation::*;

/// Maps a host errno value to the ADB wire protocol value.
pub fn errno_to_wire(errno: i32) -> i32 {
    #[cfg(unix)]
    {
        match errno {
            libc::EACCES => 13,
            libc::EEXIST => 17,
            libc::EFAULT => 14,
            libc::EFBIG => 27,
            libc::EINTR => 4,
            libc::EINVAL => 22,
            libc::EIO => 5,
            libc::EISDIR => 21,
            libc::ELOOP => 40,
            libc::EMFILE => 24,
            libc::ENAMETOOLONG => 36,
            libc::ENFILE => 23,
            libc::ENOENT => 2,
            libc::ENOMEM => 12,
            libc::ENOSPC => 28,
            libc::ENOTDIR => 20,
            libc::EOVERFLOW => 75,
            libc::EPERM => 1,
            libc::EROFS => 30,
            libc::ETXTBSY => 26,
            _ => panic!("failed to convert errno {} to wire", errno),
        }
    }
    #[cfg(windows)]
    {
        // On Windows, these can come from WSAGetLastError() or GetLastError()
        match errno as u32 {
            ERROR_ACCESS_DENIED | WSAEACCES => 13,
            ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS => 17,
            ERROR_INVALID_ADDRESS | WSAEFAULT => 14,
            ERROR_INVALID_PARAMETER | WSAEINVAL => 22,
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => 2,
            ERROR_NOT_ENOUGH_MEMORY | WSA_NOT_ENOUGH_MEMORY => 12,
            ERROR_DISK_FULL => 28,
            ERROR_DIRECTORY | ERROR_INVALID_NAME => 20,
            ERROR_PRIVILEGE_NOT_HELD => 1,
            _ => 22, // Default to EINVAL for unknown errors
        }
    }
}

/// Maps an ADB wire protocol errno value to the host errno value.
pub fn errno_from_wire(wire_errno: i32) -> i32 {
    #[cfg(unix)]
    {
        match wire_errno {
            13 => libc::EACCES,
            17 => libc::EEXIST,
            14 => libc::EFAULT,
            27 => libc::EFBIG,
            4 => libc::EINTR,
            22 => libc::EINVAL,
            5 => libc::EIO,
            21 => libc::EISDIR,
            40 => libc::ELOOP,
            24 => libc::EMFILE,
            36 => libc::ENAMETOOLONG,
            23 => libc::ENFILE,
            2 => libc::ENOENT,
            12 => libc::ENOMEM,
            28 => libc::ENOSPC,
            20 => libc::ENOTDIR,
            75 => libc::EOVERFLOW,
            1 => libc::EPERM,
            30 => libc::EROFS,
            26 => libc::ETXTBSY,
            _ => panic!("failed to convert wire errno {} to host", wire_errno),
        }
    }
    #[cfg(windows)]
    {
         match wire_errno {
            13 => WSAEACCES as i32,
            17 => ERROR_ALREADY_EXISTS as i32,
            14 => WSAEFAULT as i32,
            22 => WSAEINVAL as i32,
            2 => ERROR_FILE_NOT_FOUND as i32,
            12 => ERROR_NOT_ENOUGH_MEMORY as i32,
            28 => ERROR_DISK_FULL as i32,
            20 => ERROR_DIRECTORY as i32,
            1 => ERROR_ACCESS_DENIED as i32,
            _ => WSAEINVAL as i32,
        }
    }
}
