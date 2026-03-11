use std::env;
use std::io::{Error, ErrorKind, Result};

#[cfg(unix)]
use libc;
#[cfg(unix)]
use users;

/// Returns the value of the environment variable `var`.
/// Ported from `getenv` in `sysdeps.h`.
pub fn get_environment_variable(var: &str) -> Option<String> {
    env::var(var).ok()
}

/// Returns the host name in UTF-8.
/// Ported from `get_host_name_utf8` in `sysdeps.h`.
pub fn get_host_name_utf8() -> Result<String> {
    if let Ok(host) = env::var("HOSTNAME") {
        if !host.is_empty() {
            return Ok(host);
        }
    }

    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .map_err(|e| Error::new(ErrorKind::Other, e))
}

/// Returns the login name in UTF-8.
/// Ported from `get_login_name_utf8` in `sysdeps.h`.
pub fn get_login_name_utf8() -> Result<String> {
    #[cfg(unix)]
    {
        if let Ok(user) = env::var("LOGNAME") {
            if !user.is_empty() {
                return Ok(user);
            }
        }

        users::get_current_username()
            .map(|u| u.to_string_lossy().into_owned())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "failed to get current username"))
    }
    #[cfg(windows)]
    {
        if let Ok(user) = env::var("USERNAME") {
            if !user.is_empty() {
                return Ok(user);
            }
        }
        Err(Error::new(ErrorKind::NotFound, "failed to get current username"))
    }
}

/// Returns a human-readable OS version string.
/// Ported from `get_os_version` in `sysdeps.h`.
pub fn get_os_version() -> String {
    #[cfg(unix)]
    {
        // SAFETY: utsname is a plain-old-data struct from libc. Zero-initializing it
        // is safe before passing it to uname.
        let mut name: libc::utsname = unsafe { std::mem::zeroed() };

        // SAFETY: uname is a standard libc function. It returns 0 on success.
        // We check the return value before accessing the struct members.
        if unsafe { libc::uname(&mut name) } == 0 {
            // SAFETY: The members of utsname are null-terminated byte arrays.
            // from_ptr is safe here as we are pointing into our own stack-allocated struct.
            let sysname = unsafe { std::ffi::CStr::from_ptr(name.sysname.as_ptr()) }.to_string_lossy();
            let release = unsafe { std::ffi::CStr::from_ptr(name.release.as_ptr()) }.to_string_lossy();
            let machine = unsafe { std::ffi::CStr::from_ptr(name.machine.as_ptr()) }.to_string_lossy();
            format!("{} {} ({})", sysname, release, machine)
        } else {
            String::new()
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

        type RtlGetVersionPtr = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;

        unsafe {
            let ntdll = GetModuleHandleW(
                [b'n' as u16, b't' as u16, b'd' as u16, b'l' as u16, b'l' as u16, b'.' as u16, b'd' as u16, b'l' as u16, b'l' as u16, 0].as_ptr()
            );
            if ntdll == 0 {
                return "<Could not get handle to ntdll.dll>".to_string();
            }

            let proc_name = b"RtlGetVersion\0";
            let rtl_get_version = GetProcAddress(ntdll, proc_name.as_ptr());
            if rtl_get_version.is_none() {
                return "<Could not get address of RtlGetVersion>".to_string();
            }

            let rtl_get_version: RtlGetVersionPtr = std::mem::transmute(rtl_get_version);
            let mut version: OSVERSIONINFOW = std::mem::zeroed();
            version.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;

            if rtl_get_version(&mut version) == 0 {
                format!("Windows {}.{}.{}", version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber)
            } else {
                "<RtlGetVersion failed>".to_string()
            }
        }
    }
}

/// Given a sequence of UTF-8 bytes, return the number of bytes that are complete
/// UTF-8 sequences and the remaining bytes that are incomplete.
/// Ported from `parse_complete_utf8` in `sysdeps.h`.
pub fn parse_complete_utf8(data: &[u8]) -> (usize, Vec<u8>) {
    for i in (0..data.len()).rev() {
        let ch = data[i];
        if (ch & 0x80) == 0 {
            // Found an ASCII byte, so everything up to the end is complete
            // (trailing bytes with no lead byte are considered "complete" in terms of being processed).
            break;
        } else if (ch & 0xC0) == 0xC0 {
            // Found a lead byte
            let len = utf8_codepoint_len(ch);
            if data.len() - i < len {
                // Incomplete sequence
                return (i, data[i..].to_vec());
            } else {
                // Complete sequence
                break;
            }
        }
    }
    (data.len(), Vec::new())
}

fn utf8_codepoint_len(ch: u8) -> usize {
    if (ch & 0x80) == 0 {
        1
    } else if (ch & 0xE0) == 0xC0 {
        2
    } else if (ch & 0xF0) == 0xE0 {
        3
    } else if (ch & 0xF8) == 0xF0 {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_complete_utf8() {
        // Complete ASCII
        assert_eq!(parse_complete_utf8(b"hello"), (5, vec![]));

        // Complete multi-byte
        let c = "©".as_bytes(); // [0xC2, 0xA9]
        assert_eq!(parse_complete_utf8(c), (2, vec![]));

        // Incomplete multi-byte
        assert_eq!(parse_complete_utf8(&c[..1]), (0, vec![0xC2]));

        // Incomplete 3-byte
        let euro = "€".as_bytes(); // [0xE2, 0x82, 0xAC]
        assert_eq!(parse_complete_utf8(&euro[..1]), (0, vec![0xE2]));
        assert_eq!(parse_complete_utf8(&euro[..2]), (0, vec![0xE2, 0x82]));
        assert_eq!(parse_complete_utf8(euro), (3, vec![]));

        // Mixed
        let mut mixed = b"abc".to_vec();
        mixed.extend_from_slice(&euro[..2]);
        assert_eq!(parse_complete_utf8(&mixed), (3, vec![0xE2, 0x82]));
    }
}
