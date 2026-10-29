//! Version constants and OS version resolution for `adb`.
//!
//! Mirrored from platform-tools 36.0.2 (Build 14143358).
//! Cites `adb_version` in `original/adb.cpp:101`, `GetOSVersion` in `original/sysdeps_unix.cpp:105`,
//! and `original/sysdeps_win32.cpp:2360`.

pub const ADB_VERSION_MAJOR: u32 = 1;
pub const ADB_VERSION_MINOR: u32 = 0;
pub const ADB_SERVER_VERSION: u32 = 41;
pub const PLATFORM_TOOLS_VERSION: &str = "36.0.2";
pub const BUILD_NUMBER: &str = "14143358";

/// Formats the OS description matching C++ `GetOSVersion()`.
///
/// Ports `GetOSVersion` from `original/sysdeps_unix.cpp` and `original/sysdeps_win32.cpp`.
pub fn get_os_version() -> String {
    #[cfg(unix)]
    {
        unsafe {
            let mut uts: libc::utsname = std::mem::zeroed();
            if libc::uname(&mut uts) == 0 {
                let sysname = std::ffi::CStr::from_ptr(uts.sysname.as_ptr()).to_string_lossy();
                let release = std::ffi::CStr::from_ptr(uts.release.as_ptr()).to_string_lossy();
                let machine = std::ffi::CStr::from_ptr(uts.machine.as_ptr()).to_string_lossy();
                format!("{sysname} {release} ({machine})")
            } else {
                "unknown".to_string()
            }
        }
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        #[allow(non_snake_case, clippy::upper_case_acronyms)]
        struct OSVERSIONINFOW {
            dwOSVersionInfoSize: u32,
            dwMajorVersion: u32,
            dwMinorVersion: u32,
            dwBuildNumber: u32,
            dwPlatformId: u32,
            szCSDVersion: [u16; 128],
        }

        type RtlGetVersionPtr = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;

        unsafe {
            extern "system" {
                fn GetModuleHandleW(lpModuleName: *const u16) -> *mut std::ffi::c_void;
                fn GetProcAddress(
                    hModule: *mut std::ffi::c_void,
                    lpProcName: *const u8,
                ) -> Option<RtlGetVersionPtr>;
            }

            let module_name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
            let handle = GetModuleHandleW(module_name.as_ptr());
            if handle.is_null() {
                return "<Could not get handle to RtlGetVersion in ntdll.dll>".to_string();
            }

            let proc_name = b"RtlGetVersion\0";
            if let Some(rtl_get_version) = GetProcAddress(handle, proc_name.as_ptr()) {
                let mut version: OSVERSIONINFOW = std::mem::zeroed();
                version.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
                rtl_get_version(&mut version);
                format!(
                    "Windows {}.{}.{}",
                    version.dwMajorVersion, version.dwMinorVersion, version.dwBuildNumber
                )
            } else {
                "<Could not get handle to RtlGetVersion in ntdll.dll>".to_string()
            }
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        "unknown".to_string()
    }
}

/// Returns the formatted multi-line version string matching C++ `adb_version()`.
///
/// Cites `adb_version` in `original/adb.cpp:101`.
pub fn version_string() -> String {
    let installed_as = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| "adb".to_string());
    format!(
        "Android Debug Bridge version {}.{}.{}\nVersion {}-{}\nInstalled as {}\nRunning on {}\n",
        ADB_VERSION_MAJOR,
        ADB_VERSION_MINOR,
        ADB_SERVER_VERSION,
        PLATFORM_TOOLS_VERSION,
        BUILD_NUMBER,
        installed_as,
        get_os_version()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constants() {
        assert_eq!(ADB_VERSION_MAJOR, 1);
        assert_eq!(ADB_VERSION_MINOR, 0);
        assert_eq!(ADB_SERVER_VERSION, 41);
        assert_eq!(PLATFORM_TOOLS_VERSION, "36.0.2");
        assert_eq!(BUILD_NUMBER, "14143358");
    }

    #[test]
    fn test_version_string_format() {
        let v_str = version_string();
        let lines: Vec<&str> = v_str.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "Android Debug Bridge version 1.0.41");
        assert_eq!(lines[1], "Version 36.0.2-14143358");
        assert!(lines[2].starts_with("Installed as "));
        assert!(lines[3].starts_with("Running on "));
    }
}
