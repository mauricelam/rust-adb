//! ADB version information and OS description resolution.
//!
//! Mirrors version definition and formatting in `original/adb.h`, `original/adb.cpp`,
//! `original/sysdeps_unix.cpp`, and `original/sysdeps_win32.cpp`.
//! Pinned to `platform-tools-36.0.2` (Build 14143358).

/// Used for help/version information.
/// See `ADB_VERSION_MAJOR` in `original/adb.h:65`.
pub const ADB_VERSION_MAJOR: u32 = 1;

/// Used for help/version information.
/// See `ADB_VERSION_MINOR` in `original/adb.h:66`.
pub const ADB_VERSION_MINOR: u32 = 0;

/// ADB server protocol version string.
/// See `ADB_SERVER_VERSION` in `original/adb.h:70` and `crates/adb-client/src/lib.rs`.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Platform tools release version string.
/// See `PLATFORM_TOOLS_VERSION` usage in `original/adb.cpp:108`.
pub const PLATFORM_TOOLS_VERSION: &str = "36.0.2";

/// Android SDK build number identifier.
/// See `android::build::GetBuildNumber()` usage in `original/adb.cpp:109`.
pub const BUILD_NUMBER: &str = "14143358";

/// Static usage text included from `help.txt`.
pub const USAGE_TEXT: &str = include_str!("help.txt");

/// Resolves OS version description.
///
/// Ported from `GetOSVersion()` in `original/sysdeps_unix.cpp:105` and `original/sysdeps_win32.cpp:2360`.
pub fn get_os_version() -> String {
    #[cfg(unix)]
    {
        use std::ffi::CStr;
        let mut name: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut name) } == 0 {
            let sysname = unsafe { CStr::from_ptr(name.sysname.as_ptr()) }.to_string_lossy();
            let release = unsafe { CStr::from_ptr(name.release.as_ptr()) }.to_string_lossy();
            let machine = unsafe { CStr::from_ptr(name.machine.as_ptr()) }.to_string_lossy();
            format!("{sysname} {release} ({machine})")
        } else {
            "Unknown".to_string()
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
        type RtlGetVersionPtr = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
        unsafe {
            let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(
                widestring("ntdll.dll").as_ptr(),
            );
            if !module.is_null() {
                let func_name = std::ffi::CString::new("RtlGetVersion").unwrap();
                let proc = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
                    module,
                    func_name.as_ptr() as *const u8,
                );
                if let Some(rtl_get_version) = proc {
                    let rtl_get_version: RtlGetVersionPtr = std::mem::transmute(rtl_get_version);
                    let mut ver: OSVERSIONINFOW = std::mem::zeroed();
                    ver.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
                    if rtl_get_version(&mut ver) == 0 {
                        return format!(
                            "Windows {}.{}.{}",
                            ver.dwMajorVersion, ver.dwMinorVersion, ver.dwBuildNumber
                        );
                    }
                }
            }
        }
        "<Could not get handle to RtlGetVersion in ntdll.dll>".to_string()
    }
    #[cfg(not(any(unix, windows)))]
    {
        "Unknown".to_string()
    }
}

#[cfg(windows)]
fn widestring(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Formats standard ADB version block.
///
/// Ported from `adb_version()` in `original/adb.cpp:101`.
pub fn adb_version() -> String {
    let exe_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "adb".to_string());
    let os_ver = get_os_version();
    format!(
        "Android Debug Bridge version {ADB_VERSION_MAJOR}.{ADB_VERSION_MINOR}.{ADB_SERVER_VERSION}\n\
         Version {PLATFORM_TOOLS_VERSION}-{BUILD_NUMBER}\n\
         Installed as {exe_path}\n\
         Running on {os_ver}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constants_format() {
        let ver = adb_version();
        assert!(ver.starts_with("Android Debug Bridge version 1.0.41\nVersion 36.0.2-14143358\n"));
        assert!(ver.contains("Installed as "));
        assert!(ver.contains("Running on "));
    }
}
