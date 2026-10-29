//! Version constants and runtime version string generation for `adb`.
//!
//! Mirrors platform-tools version 36.0.2 (build 14143358), corresponding to upstream
//! C++ ADB `original/adb.cpp` and `original/adb.h`.

use std::env;

/// Protocol major version (`ADB_VERSION_MAJOR` in `original/adb.h`).
pub const ADB_VERSION_MAJOR: u32 = 1;

/// Protocol minor version (`ADB_VERSION_MINOR` in `original/adb.h`).
pub const ADB_VERSION_MINOR: u32 = 0;

/// Internal server version (`ADB_SERVER_VERSION` in `original/adb.h`).
pub const ADB_SERVER_VERSION: u32 = 41;

/// Platform tools version string (`PLATFORM_TOOLS_VERSION` in `original/adb.cpp`).
pub const PLATFORM_TOOLS_VERSION: &str = "37.0.1";

/// Build number string (`android::build::GetBuildNumber()` in `original/adb.cpp`).
pub const BUILD_NUMBER: &str = "15733141";

/// Header line 1 for version output, e.g. "Android Debug Bridge version 1.0.41".
pub const VERSION_LINE_1: &str = "Android Debug Bridge version 1.0.41";

/// Header line 2 for version output, e.g. "Version 37.0.1-15733141".
pub const VERSION_LINE_2: &str = "Version 37.0.1-15733141";

/// Returns the operating system description string matching `GetOSVersion()` in upstream C++:
/// - Unix/Linux/macOS (`original/sysdeps_unix.cpp:105`): `StringPrintf("%s %s (%s)", name.sysname, name.release, name.machine)`
/// - Windows (`original/sysdeps_win32.cpp:2360`): `StringPrintf("Windows %lu.%lu.%lu", dwMajorVersion, dwMinorVersion, dwBuildNumber)`
pub fn get_os_version() -> String {
    #[cfg(unix)]
    {
        // Safe wrapper around POSIX uname(2) matching sysdeps_unix.cpp:105
        let mut uts = std::mem::MaybeUninit::<libc::utsname>::uninit();
        // SAFETY: uname takes a pointer to a mutable utsname struct.
        let ret = unsafe { libc::uname(uts.as_mut_ptr()) };
        if ret == 0 {
            // SAFETY: uts is initialized upon successful return of uname.
            let uts = unsafe { uts.assume_init() };

            fn to_str(buf: &[libc::c_char]) -> &str {
                let null_pos = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                let slice = &buf[..null_pos];
                // Cast c_char to u8 safely
                let bytes: &[u8] = unsafe { &*(slice as *const [_] as *const [u8]) };
                std::str::from_utf8(bytes).unwrap_or("")
            }

            let sysname = to_str(&uts.sysname);
            let release = to_str(&uts.release);
            let machine = to_str(&uts.machine);
            return format!("{sysname} {release} ({machine})");
        }
        format!(
            "{} unknown ({})",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    }

    #[cfg(windows)]
    {
        // Match sysdeps_win32.cpp:2360 using RtlGetVersion dynamically from ntdll.dll
        #[repr(C)]
        struct OsVersionInfoW {
            dw_os_version_info_size: u32,
            dw_major_version: u32,
            dw_minor_version: u32,
            dw_build_number: u32,
            dw_platform_id: u32,
            sz_csd_version: [u16; 128],
        }

        type RtlGetVersionFn = unsafe extern "system" fn(*mut OsVersionInfoW) -> i32;

        unsafe {
            use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
            let ntdll_name: Vec<u16> = "ntdll.dll\0".encode_utf16().collect();
            let module = GetModuleHandleW(ntdll_name.as_ptr());
            if !module.is_null() {
                let proc = GetProcAddress(module, c"RtlGetVersion".as_ptr());
                if let Some(rtl_get_version) = proc {
                    let rtl_get_version: RtlGetVersionFn = std::mem::transmute(rtl_get_version);
                    let mut info = std::mem::zeroed::<OsVersionInfoW>();
                    info.dw_os_version_info_size = std::mem::size_of::<OsVersionInfoW>() as u32;
                    if rtl_get_version(&mut info) == 0 {
                        return format!(
                            "Windows {}.{}.{}",
                            info.dw_major_version, info.dw_minor_version, info.dw_build_number
                        );
                    }
                }
            }
        }

        "<Could not get handle to RtlGetVersion in ntdll.dll>".to_string()
    }

    #[cfg(not(any(unix, windows)))]
    {
        format!(
            "{} unknown ({})",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    }
}

/// Returns the formatted version string matching `adb_version()` in `original/adb.cpp:101`.
///
/// Output format:
/// ```text
/// Android Debug Bridge version 1.0.41
/// Version 36.0.2-14143358
/// Installed as /path/to/adb
/// Running on Darwin 25.6.0 (arm64)
/// ```
pub fn get_version_string() -> String {
    let installed_as = env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "adb".to_string());
    let os_version = get_os_version();

    format!(
        "{VERSION_LINE_1}\n{VERSION_LINE_2}\nInstalled as {installed_as}\nRunning on {os_version}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constants() {
        assert_eq!(
            VERSION_LINE_1,
            format!("Android Debug Bridge version {ADB_VERSION_MAJOR}.{ADB_VERSION_MINOR}.{ADB_SERVER_VERSION}")
        );
        assert_eq!(
            VERSION_LINE_2,
            format!("Version {PLATFORM_TOOLS_VERSION}-{BUILD_NUMBER}")
        );
    }

    #[test]
    fn test_version_string_structure() {
        let v = get_version_string();
        let lines: Vec<&str> = v.lines().collect();
        assert_eq!(lines.len(), 4, "Version string should have 4 lines: {v}");
        assert_eq!(lines[0], VERSION_LINE_1);
        assert_eq!(lines[1], VERSION_LINE_2);
        assert!(lines[2].starts_with("Installed as "));
        assert!(lines[3].starts_with("Running on "));
    }
}
