//! Version constants and OS version detection for `adb`.
//!
//! Ports `adb_version()` from `original/adb.cpp:101` and `GetOSVersion()` from
//! `original/sysdeps_unix.cpp:105` / `original/sysdeps_win32.cpp:2360`.
//! Mirrors platform-tools version 36.0.2 (Build 14143358).

/// ADB major protocol version (ADB_VERSION_MAJOR in `original/adb.h:65`).
pub const ADB_VERSION_MAJOR: u32 = 1;

/// ADB minor protocol version (ADB_VERSION_MINOR in `original/adb.h:66`).
pub const ADB_VERSION_MINOR: u32 = 0;

/// ADB server protocol version (ADB_SERVER_VERSION in `original/adb.h`).
pub const ADB_SERVER_VERSION: u32 = 41;

/// Platform tools release version string (PLATFORM_TOOLS_VERSION in `original/adb.cpp:108`).
pub const PLATFORM_TOOLS_VERSION: &str = "36.0.2";

/// Build number string (`GetBuildNumber()` in `original/adb.cpp:109`).
pub const BUILD_NUMBER: &str = "14143358";

/// Returns the host OS description string.
///
/// Ports `GetOSVersion()` in `original/sysdeps_unix.cpp:105` and `original/sysdeps_win32.cpp:2360`.
#[cfg(unix)]
pub fn get_os_version() -> String {
    unsafe {
        let mut uts: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut uts) == 0 {
            let sysname = std::ffi::CStr::from_ptr(uts.sysname.as_ptr()).to_string_lossy();
            let release = std::ffi::CStr::from_ptr(uts.release.as_ptr()).to_string_lossy();
            let machine = std::ffi::CStr::from_ptr(uts.machine.as_ptr()).to_string_lossy();
            format!("{sysname} {release} ({machine})")
        } else {
            "Unknown".to_string()
        }
    }
}

/// Returns the host OS description string.
///
/// Ports `GetOSVersion()` in `original/sysdeps_win32.cpp:2360`.
#[cfg(windows)]
pub fn get_os_version() -> String {
    use std::ffi::CString;

    #[repr(C)]
    #[allow(non_snake_case)]
    struct OSVERSIONINFOW {
        dwOSVersionInfoSize: u32,
        dwMajorVersion: u32,
        dwMinorVersion: u32,
        dwBuildNumber: u32,
        dwPlatformId: u32,
        szCSDVersion: [u16; 128],
    }

    type RtlGetVersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;

    unsafe {
        let ntdll_name = CString::new("ntdll.dll").unwrap();
        let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleA(
            ntdll_name.as_ptr() as *const u8,
        );
        if module == 0 {
            return "<Could not get handle to ntdll.dll>".to_string();
        }
        let proc_name = CString::new("RtlGetVersion").unwrap();
        let proc = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            proc_name.as_ptr() as *const u8,
        );
        if proc.is_none() {
            return "<Could not get handle to RtlGetVersion in ntdll.dll>".to_string();
        }
        let rtl_get_version: RtlGetVersionFn = std::mem::transmute(proc);

        let mut info: OSVERSIONINFOW = std::mem::zeroed();
        info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;

        if rtl_get_version(&mut info) == 0 {
            format!(
                "Windows {}.{}.{}",
                info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
            )
        } else {
            "Windows".to_string()
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub fn get_os_version() -> String {
    "Unknown".to_string()
}

/// Returns the formatted multi-line version string for `adb version`.
///
/// Ports `adb_version()` in `original/adb.cpp:101`.
pub fn get_version_string() -> String {
    let installed_as = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "adb".to_string());
    let os_version = get_os_version();
    format!(
        "Android Debug Bridge version {}.{}.{}\nVersion {}-{}\nInstalled as {}\nRunning on {}\n",
        ADB_VERSION_MAJOR,
        ADB_VERSION_MINOR,
        ADB_SERVER_VERSION,
        PLATFORM_TOOLS_VERSION,
        BUILD_NUMBER,
        installed_as,
        os_version
    )
}
