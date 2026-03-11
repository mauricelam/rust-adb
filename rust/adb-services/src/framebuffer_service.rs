//! Framebuffer service implementation.
//! Ported from `original/daemon/framebuffer_service.cpp`.

use adb_io::{read_exactly, write_exactly};
use sysdeps::AdbFd;
use zerocopy::{IntoBytes, FromBytes, Immutable, KnownLayout};
use std::io::Read;
use std::process::{Command, Stdio};

/// This version number defines the format of the fbinfo struct.
/// It must match versioning in ddms where this data is consumed.
pub const DDMS_RAWIMAGE_VERSION: u32 = 2;

/// Framebuffer information header.
/// Ported from `fbinfo` in `original/daemon/framebuffer_service.cpp`.
#[repr(C, packed)]
#[derive(Clone, Copy, Default, IntoBytes, FromBytes, Immutable, KnownLayout, Debug, PartialEq)]
pub struct FbInfo {
    /// Version of the header format.
    pub version: u32,
    /// Bits per pixel.
    pub bpp: u32,
    /// Color space identifier.
    pub color_space: u32,
    /// Size of the framebuffer data in bytes.
    pub size: u32,
    /// Width of the framebuffer in pixels.
    pub width: u32,
    /// Height of the framebuffer in pixels.
    pub height: u32,
    /// Offset of the red channel in bits.
    pub red_offset: u32,
    /// Length of the red channel in bits.
    pub red_length: u32,
    /// Offset of the blue channel in bits.
    pub blue_offset: u32,
    /// Length of the blue channel in bits.
    pub blue_length: u32,
    /// Offset of the green channel in bits.
    pub green_offset: u32,
    /// Length of the green channel in bits.
    pub green_length: u32,
    /// Offset of the alpha channel in bits.
    pub alpha_offset: u32,
    /// Length of the alpha channel in bits.
    pub alpha_length: u32,
}

/// Service that sends snapshots of the framebuffer to a client.
/// Ported from `framebuffer_service` in `original/daemon/framebuffer_service.cpp`.
///
/// # Arguments
/// * `fd` - The socket connected to the client.
pub fn framebuffer_service(mut fd: AdbFd) {
    log::info!("Starting framebuffer service");
    let mut child = match Command::new("screencap")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn() {
            Ok(c) => c,
            Err(e) => {
                log::error!("failed to execute screencap: {}", e);
                return;
            }
        };

    let mut stdout = child.stdout.take().expect("failed to open screencap stdout");
    let mut stderr = child.stderr.take().expect("failed to open screencap stderr");

    let mut header = [0u8; 16];
    if let Err(e) = read_exactly(&mut stdout, &mut header) {
        let mut stderr_msg = String::new();
        let _ = stderr.read_to_string(&mut stderr_msg);
        log::error!("failed to read screencap header: {}. stderr: {}", e, stderr_msg);
        let _ = child.kill();
        return;
    }

    let w = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let h = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let f = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let c = u32::from_le_bytes(header[12..16].try_into().unwrap());

    log::info!("screencap header: w={}, h={}, f={}, c={}", w, h, f, c);

    let mut fbinfo = FbInfo {
        version: DDMS_RAWIMAGE_VERSION,
        color_space: c,
        width: w,
        height: h,
        ..Default::default()
    };

    /* see hardware/hardware.h */
    match f {
        1 => { /* RGBA_8888 */
            fbinfo.bpp = 32;
            fbinfo.size = w * h * 4;
            fbinfo.red_offset = 0;
            fbinfo.red_length = 8;
            fbinfo.green_offset = 8;
            fbinfo.green_length = 8;
            fbinfo.blue_offset = 16;
            fbinfo.blue_length = 8;
            fbinfo.alpha_offset = 24;
            fbinfo.alpha_length = 8;
        }
        2 => { /* RGBX_8888 */
            fbinfo.bpp = 32;
            fbinfo.size = w * h * 4;
            fbinfo.red_offset = 0;
            fbinfo.red_length = 8;
            fbinfo.green_offset = 8;
            fbinfo.green_length = 8;
            fbinfo.blue_offset = 16;
            fbinfo.blue_length = 8;
            fbinfo.alpha_offset = 24;
            fbinfo.alpha_length = 0;
        }
        3 => { /* RGB_888 */
            fbinfo.bpp = 24;
            fbinfo.size = w * h * 3;
            fbinfo.red_offset = 0;
            fbinfo.red_length = 8;
            fbinfo.green_offset = 8;
            fbinfo.green_length = 8;
            fbinfo.blue_offset = 16;
            fbinfo.blue_length = 8;
            fbinfo.alpha_offset = 24;
            fbinfo.alpha_length = 0;
        }
        4 => { /* RGB_565 */
            fbinfo.bpp = 16;
            fbinfo.size = w * h * 2;
            fbinfo.red_offset = 11;
            fbinfo.red_length = 5;
            fbinfo.green_offset = 5;
            fbinfo.green_length = 6;
            fbinfo.blue_offset = 0;
            fbinfo.blue_length = 5;
            fbinfo.alpha_offset = 0;
            fbinfo.alpha_length = 0;
        }
        5 => { /* BGRA_8888 */
            fbinfo.bpp = 32;
            fbinfo.size = w * h * 4;
            fbinfo.red_offset = 16;
            fbinfo.red_length = 8;
            fbinfo.green_offset = 8;
            fbinfo.green_length = 8;
            fbinfo.blue_offset = 0;
            fbinfo.blue_length = 8;
            fbinfo.alpha_offset = 24;
            fbinfo.alpha_length = 8;
        }
        _ => {
            log::error!("unknown screencap format: {}", f);
            let _ = child.kill();
            return;
        }
    }

    if let Err(e) = write_exactly(&mut fd, fbinfo.as_bytes()) {
        log::error!("failed to write fbinfo: {}", e);
        let _ = child.kill();
        return;
    }

    let mut buf = [0u8; 4096];
    let mut total_sent = 0;
    while total_sent < fbinfo.size {
        let to_read = std::cmp::min(buf.len() as u32, fbinfo.size - total_sent) as usize;
        match stdout.read_exact(&mut buf[..to_read]) {
            Ok(()) => {
                if let Err(e) = write_exactly(&mut fd, &buf[..to_read]) {
                    log::error!("failed to write framebuffer data: {}", e);
                    break;
                }
                total_sent += to_read as u32;
            }
            Err(e) => {
                log::error!("failed to read screencap data: {}", e);
                break;
            }
        }
    }

    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::fs::File;
    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn test_framebuffer_service_rgba_8888() {
        let _ = env_logger::builder().is_test(true).try_init();

        let dir = tempdir().unwrap();
        let screencap_path = dir.path().join("screencap");

        // Create a mock screencap script
        // Header: w=100, h=200, f=1 (RGBA_8888), c=0
        {
            let mut script = File::create(&screencap_path).unwrap();
            script.write_all(b"#!/bin/sh\n").unwrap();
            // w=100 (0144 octal), h=200 (0310 octal), f=1 (0001), c=0
            script.write_all(b"printf '\\144\\000\\000\\000\\310\\000\\000\\000\\001\\000\\000\\000\\000\\000\\000\\000'\n").unwrap();
            // size = 100 * 200 * 4 = 80000. Send some dummy data.
            script.write_all(b"head -c 80000 /dev/zero\n").unwrap();

            use std::os::unix::fs::PermissionsExt;
            let mut perms = script.metadata().unwrap().permissions();
            perms.set_mode(0o755);
            script.set_permissions(perms).unwrap();
        }

        let old_path = std::env::var("PATH").unwrap();
        std::env::set_var("PATH", format!("{}:{}", dir.path().to_str().unwrap(), old_path));

        let (mut s1, s2) = sysdeps::net::adb_socketpair().unwrap();
        let handle = std::thread::spawn(move || {
            framebuffer_service(s2);
        });

        let mut fbinfo_bytes = [0u8; std::mem::size_of::<FbInfo>()];
        s1.read_exact(&mut fbinfo_bytes).expect("Failed to read FbInfo header from socket");
        let fbinfo = FbInfo::read_from_bytes(&fbinfo_bytes).unwrap();

        {
            let version = fbinfo.version;
            assert_eq!(version, DDMS_RAWIMAGE_VERSION);
            let width = fbinfo.width;
            assert_eq!(width, 100);
            let height = fbinfo.height;
            assert_eq!(height, 200);
            let bpp = fbinfo.bpp;
            assert_eq!(bpp, 32);
            let size = fbinfo.size;
            assert_eq!(size, 80000);
            let red_offset = fbinfo.red_offset;
            assert_eq!(red_offset, 0);
            let red_length = fbinfo.red_length;
            assert_eq!(red_length, 8);
            let alpha_length = fbinfo.alpha_length;
            assert_eq!(alpha_length, 8);
        }

        let mut data = vec![0u8; 80000];
        s1.read_exact(&mut data).expect("Failed to read framebuffer data from socket");
        assert_eq!(data, vec![0u8; 80000]);

        handle.join().unwrap();
        std::env::set_var("PATH", old_path);
    }

    #[cfg(unix)]
    #[test]
    fn test_framebuffer_service_unsupported_format() {
        let _ = env_logger::builder().is_test(true).try_init();

        let dir = tempdir().unwrap();
        let screencap_path = dir.path().join("screencap");

        // Create a mock screencap script
        // Header: w=10, h=10, f=99 (Unsupported), c=0
        {
            let mut script = File::create(&screencap_path).unwrap();
            script.write_all(b"#!/bin/sh\n").unwrap();
            script.write_all(b"printf '\\012\\000\\000\\000\\012\\000\\000\\000\\143\\000\\000\\000\\000\\000\\000\\000'\n").unwrap();

            use std::os::unix::fs::PermissionsExt;
            let mut perms = script.metadata().unwrap().permissions();
            perms.set_mode(0o755);
            script.set_permissions(perms).unwrap();
        }

        let old_path = std::env::var("PATH").unwrap();
        std::env::set_var("PATH", format!("{}:{}", dir.path().to_str().unwrap(), old_path));

        let (mut s1, s2) = sysdeps::net::adb_socketpair().unwrap();
        let handle = std::thread::spawn(move || {
            framebuffer_service(s2);
        });

        let mut buf = [0u8; 1];
        // Service should close the connection without sending anything
        assert_eq!(s1.read(&mut buf).unwrap(), 0);

        handle.join().unwrap();
        std::env::set_var("PATH", old_path);
    }

    #[cfg(unix)]
    #[test]
    fn test_framebuffer_service_rgb_565() {
        let _ = env_logger::builder().is_test(true).try_init();

        let dir = tempdir().unwrap();
        let screencap_path = dir.path().join("screencap");

        // Create a mock screencap script
        // Header: w=50, h=50, f=4 (RGB_565), c=0
        {
            let mut script = File::create(&screencap_path).unwrap();
            script.write_all(b"#!/bin/sh\n").unwrap();
            // w=50 (0062 octal), h=50 (0062 octal), f=4 (0004), c=0
            script.write_all(b"printf '\\062\\000\\000\\000\\062\\000\\000\\000\\004\\000\\000\\000\\000\\000\\000\\000'\n").unwrap();
            // size = 50 * 50 * 2 = 5000. Send some dummy data.
            script.write_all(b"printf 'PIXELS'\n").unwrap();
            script.write_all(b"head -c 4994 /dev/zero\n").unwrap();

            use std::os::unix::fs::PermissionsExt;
            let mut perms = script.metadata().unwrap().permissions();
            perms.set_mode(0o755);
            script.set_permissions(perms).unwrap();
        }

        let old_path = std::env::var("PATH").unwrap();
        std::env::set_var("PATH", format!("{}:{}", dir.path().to_str().unwrap(), old_path));

        let (mut s1, s2) = sysdeps::net::adb_socketpair().unwrap();
        let handle = std::thread::spawn(move || {
            framebuffer_service(s2);
        });

        let mut fbinfo_bytes = [0u8; std::mem::size_of::<FbInfo>()];
        s1.read_exact(&mut fbinfo_bytes).expect("Failed to read FbInfo header from socket");
        let fbinfo = FbInfo::read_from_bytes(&fbinfo_bytes).unwrap();

        {
            let bpp = fbinfo.bpp;
            assert_eq!(bpp, 16);
            let size = fbinfo.size;
            assert_eq!(size, 5000);
            let red_offset = fbinfo.red_offset;
            assert_eq!(red_offset, 11);
            let blue_offset = fbinfo.blue_offset;
            assert_eq!(blue_offset, 0);
        }

        let mut data = vec![0u8; 5000];
        s1.read_exact(&mut data).expect("Failed to read framebuffer data from socket");
        assert_eq!(&data[0..6], b"PIXELS");

        handle.join().unwrap();
        std::env::set_var("PATH", old_path);
    }
}
