//! Diagnostics for USB device permissions and udev configuration.
//!
//! Ported directly from `diagnose_usb.cpp` in Android platform tools:
//! `android-tools-cmake/vendor/core/diagnose_usb/diagnose_usb.cpp`.

const PERMISSIONS_HELP_URL: &str = "http://developer.android.com/tools/device.html";

/// Returns a message describing any potential problems found with udev,
/// or an empty string if plugdev information cannot be found or on non-Linux systems.
pub fn get_udev_problem() -> String {
    #[cfg(target_os = "linux")]
    {
        get_udev_problem_linux()
    }
    #[cfg(not(target_os = "linux"))]
    {
        String::new()
    }
}

#[cfg(target_os = "linux")]
fn get_udev_problem_linux() -> String {
    use std::ffi::CStr;

    unsafe {
        *libc::__errno_location() = 0;
        let plugdev_name = c"plugdev".as_ptr();
        let plugdev_group = libc::getgrnam(plugdev_name);

        if plugdev_group.is_null() {
            // Cannot give useful advice, let caller print help URL.
            return String::new();
        }

        let plugdev_gid = (*plugdev_group).gr_gid;

        let ngroups = libc::getgroups(0, std::ptr::null_mut());
        if ngroups < 0 {
            return String::new();
        }

        let mut groups = vec![0 as libc::gid_t; ngroups as usize];
        let ngroups = libc::getgroups(groups.len() as libc::c_int, groups.as_mut_ptr());
        if ngroups < 0 {
            return String::new();
        }
        groups.truncate(ngroups as usize);

        let egid = libc::getegid();
        if groups.contains(&plugdev_gid) || egid == plugdev_gid {
            // The user is in plugdev so the problem is likely with the udev rules.
            return "missing udev rules? user is in the plugdev group".to_string();
        }

        let uid = libc::getuid();
        let pwd = libc::getpwuid(uid);
        let user_name = if !pwd.is_null() && !(*pwd).pw_name.is_null() {
            CStr::from_ptr((*pwd).pw_name)
                .to_str()
                .unwrap_or("?")
                .to_string()
        } else {
            "?".to_string()
        };

        format!("user {user_name} is not in the plugdev group")
    }
}

/// Short help text for USB permission failures. Must be a single line:
///
/// `no permissions (reason); see [http://developer.android.com/tools/device.html]`
pub fn usb_no_permissions_short_help_text() -> String {
    let mut help_text = "no permissions".to_string();
    let problem = get_udev_problem();
    if !problem.is_empty() {
        help_text.push_str(&format!(" ({problem})"));
    }
    format!("{help_text}; see [{PERMISSIONS_HELP_URL}]")
}

/// Long help text for USB permission failures. Can span multiple lines:
///
/// `insufficient permissions for device: reason`
/// `See [http://developer.android.com/tools/device.html] for more information`
pub fn usb_no_permissions_long_help_text() -> String {
    let mut header = "insufficient permissions for device".to_string();
    let problem = get_udev_problem();
    if !problem.is_empty() {
        header.push_str(&format!(": {problem}"));
    }
    format!("{header}\nSee [{PERMISSIONS_HELP_URL}] for more information")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_urls_match_upstream() {
        assert_eq!(
            PERMISSIONS_HELP_URL,
            "http://developer.android.com/tools/device.html"
        );
        let short = usb_no_permissions_short_help_text();
        assert!(short.starts_with("no permissions"));
        assert!(short.contains("see [http://developer.android.com/tools/device.html]"));

        let long = usb_no_permissions_long_help_text();
        assert!(long.starts_with("insufficient permissions for device"));
        assert!(long
            .contains("See [http://developer.android.com/tools/device.html] for more information"));
    }
}
