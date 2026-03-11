/*
 * Copyright (C) 2015 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Shell service implementation.
//! Ported from original/daemon/shell_service.cpp.

use adb_protocol::shell_protocol::{ShellId, ShellProtocol};
use sysdeps::poll::{adb_poll, AdbPollFd, POLLIN};
use sysdeps::AdbFd;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawHandle, IntoRawHandle};

/// Argument for raw shell mode.
pub const K_SHELL_SERVICE_ARG_RAW: &str = "raw";
/// Argument for PTY shell mode.
pub const K_SHELL_SERVICE_ARG_PTY: &str = "pty";
/// Argument for shell protocol v2.
pub const K_SHELL_SERVICE_ARG_SHELL_PROTOCOL: &str = "v2";

#[cfg(unix)]
fn open_pty() -> std::io::Result<(AdbFd, AdbFd)> {
    unsafe {
        let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if master_fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        if libc::grantpt(master_fd) < 0 || libc::unlockpt(master_fd) < 0 {
            let err = std::io::Error::last_os_error();
            libc::close(master_fd);
            return Err(err);
        }

        let pts_ptr = libc::ptsname(master_fd);
        if pts_ptr.is_null() {
            let err = std::io::Error::last_os_error();
            libc::close(master_fd);
            return Err(err);
        }
        let pts_name = std::ffi::CStr::from_ptr(pts_ptr);
        let slave_fd = libc::open(pts_name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if slave_fd < 0 {
            let err = std::io::Error::last_os_error();
            libc::close(master_fd);
            return Err(err);
        }

        Ok((AdbFd::from_raw_fd(master_fd), AdbFd::from_raw_fd(slave_fd)))
    }
}

#[cfg(windows)]
struct ConPty {
    hpcon: windows_sys::Win32::System::Console::HPCON,
    h_stdin_write: AdbFd,
    h_stdout_read: AdbFd,
    h_process: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
fn open_conpty(command_str: &str) -> std::io::Result<ConPty> {
    use windows_sys::Win32::System::Console::*;
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::Foundation::*;

    unsafe {
        let mut h_stdin_read = 0;
        let mut h_stdin_write = 0;
        let mut h_stdout_read = 0;
        let mut h_stdout_write = 0;

        if CreatePipe(&mut h_stdin_read, &mut h_stdin_write, std::ptr::null(), 0) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if CreatePipe(&mut h_stdout_read, &mut h_stdout_write, std::ptr::null(), 0) == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let size = COORD { X: 80, Y: 24 };
        let mut hpcon = 0;
        let res = CreatePseudoConsole(size, h_stdin_read, h_stdout_write, 0, &mut hpcon);

        CloseHandle(h_stdin_read);
        CloseHandle(h_stdout_write);

        if res != 0 {
            return Err(std::io::Error::last_os_error());
        }

        let mut size_attr = 0;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size_attr);
        let mut buffer = vec![0u8; size_attr];
        let lp_attribute_list = buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        InitializeProcThreadAttributeList(lp_attribute_list, 1, 0, &mut size_attr);

        UpdateProcThreadAttribute(
            lp_attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as _,
            &hpcon as *const _ as *const _,
            std::mem::size_of::<HPCON>(),
            std::ptr::null(),
            std::ptr::null(),
        );

        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = lp_attribute_list;

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();

        let cmd = if command_str.is_empty() {
            "cmd.exe\0".encode_utf16().collect::<Vec<u16>>()
        } else {
            format!("cmd.exe /c {}\0", command_str).encode_utf16().collect::<Vec<u16>>()
        };

        if CreateProcessW(
            std::ptr::null(),
            cmd.as_ptr() as _,
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            &si.StartupInfo as *const _ as _,
            &mut pi,
        ) == 0 {
            ClosePseudoConsole(hpcon);
            return Err(std::io::Error::last_os_error());
        }

        CloseHandle(pi.hThread);

        Ok(ConPty {
            hpcon,
            h_stdin_write: AdbFd::from_raw_handle(h_stdin_write as _),
            h_stdout_read: AdbFd::from_raw_handle(h_stdout_read as _),
            h_process: pi.hProcess,
        })
    }
}

/// Service that runs a shell command.
pub fn shell_service(adb_fd: AdbFd, args: &str) {
    let mut adb_fd_opt = Some(adb_fd);
    let command_str;
    let mut subprocess_type;
    let mut protocol = "none";
    let mut terminal_type = "dumb";

    if let Some(colon_idx) = args.find(':') {
        let service_args = &args[..colon_idx];
        command_str = &args[colon_idx + 1..];

        subprocess_type = if command_str.is_empty() { "pty" } else { "raw" };

        for arg in service_args.split(',') {
            match arg {
                K_SHELL_SERVICE_ARG_RAW => subprocess_type = "raw",
                K_SHELL_SERVICE_ARG_PTY => subprocess_type = "pty",
                K_SHELL_SERVICE_ARG_SHELL_PROTOCOL => protocol = "v2",
                _ if arg.starts_with("TERM=") => terminal_type = &arg[5..],
                _ => {}
            }
        }
    } else {
        command_str = args;
        subprocess_type = if command_str.is_empty() { "pty" } else { "raw" };
    }

    let mut make_pty_raw = false;
    if protocol == "none" && subprocess_type == "raw" {
        subprocess_type = "pty";
        make_pty_raw = true;
    }

    let is_pty = subprocess_type == "pty";
    let is_v2 = protocol == "v2";

    #[cfg(windows)]
    if is_pty {
        let conpty = match open_conpty(command_str) {
            Ok(c) => c,
            Err(e) => {
                log::error!("failed to open ConPTY: {}", e);
                return;
            }
        };

        let mut adb_fd = adb_fd_opt.take().unwrap();
        let con_stdin = Arc::new(Mutex::new(conpty.h_stdin_write));
        let mut con_stdout = conpty.h_stdout_read;
        let hpcon = conpty.hpcon;

        let adb_fd_clone = adb_fd.try_clone().expect("failed to clone adb fd");
        let con_stdin_clone = con_stdin.clone();

        // Thread to read from ADB and write to ConPTY stdin
        std::thread::spawn(move || {
            let mut adb_fd = adb_fd_clone;
            let mut shell_read = ShellProtocol::new();
            loop {
                if is_v2 {
                    match shell_read.read(&mut adb_fd) {
                        Ok(true) => match shell_read.id {
                            ShellId::Stdin => {
                                let mut con_stdin = con_stdin_clone.lock().unwrap();
                                if let Err(_) = con_stdin.write_all(&shell_read.data) {
                                    break;
                                }
                            }
                            ShellId::WindowSizeChange => {
                                use windows_sys::Win32::System::Console::{ResizePseudoConsole, COORD};
                                let s = String::from_utf8_lossy(&shell_read.data);
                                if let Some((rows_cols, _)) = s.split_once(',') {
                                    if let Some((rows, cols)) = rows_cols.split_once('x') {
                                        if let (Ok(r), Ok(c)) = (rows.parse::<u16>(), cols.parse::<u16>()) {
                                            unsafe { ResizePseudoConsole(hpcon, COORD { X: c as i16, Y: r as i16 }); }
                                        }
                                    }
                                }
                            }
                            ShellId::CloseStdin => {
                                let mut con_stdin = con_stdin_clone.lock().unwrap();
                                con_stdin.close();
                                break;
                            }
                            _ => {}
                        },
                        _ => break,
                    }
                } else {
                    let mut buf = [0u8; 4096];
                    match adb_fd.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            let mut con_stdin = con_stdin_clone.lock().unwrap();
                            if let Err(_) = con_stdin.write_all(&buf[..n]) {
                                break;
                            }
                        }
                        _ => break,
                    }
                }
            }
        });

        // Loop to read from ConPTY stdout and write to ADB
        loop {
            let mut buf = [0u8; 4096];
            match con_stdout.read(&mut buf) {
                Ok(n) if n > 0 => {
                    if is_v2 {
                        if let Err(_) = ShellProtocol::write_packet(&mut adb_fd, ShellId::Stdout, &buf[..n]) {
                            break;
                        }
                    } else {
                        if let Err(_) = adb_fd.write_all(&buf[..n]) {
                            break;
                        }
                    }
                }
                _ => break,
            }

            unsafe {
                let mut exit_code: u32 = 0;
                if windows_sys::Win32::System::Threading::GetExitCodeProcess(conpty.h_process, &mut exit_code) != 0 {
                    if exit_code != windows_sys::Win32::System::Threading::STILL_ACTIVE as u32 {
                        if is_v2 {
                            let _ = ShellProtocol::write_packet(&mut adb_fd, ShellId::Exit, &[exit_code as u8]);
                        }
                        break;
                    }
                }
            }
        }

        unsafe {
            windows_sys::Win32::System::Console::ClosePseudoConsole(conpty.hpcon);
            windows_sys::Win32::Foundation::CloseHandle(conpty.h_process);
        }
        return;
    }

    let mut master_fd_opt: Option<AdbFd> = None;
    let child_stdin: std::process::Stdio;
    let child_stdout: std::process::Stdio;
    let child_stderr: std::process::Stdio;

    #[cfg(unix)]
    if is_pty {
        let (master, slave) = match open_pty() {
            Ok(pair) => pair,
            Err(e) => {
                log::error!("failed to open pty: {}", e);
                return;
            }
        };

        if make_pty_raw {
            unsafe {
                let mut tattr: libc::termios = std::mem::zeroed();
                libc::tcgetattr(slave.as_raw_fd(), &mut tattr);
                libc::cfmakeraw(&mut tattr);
                libc::tcsetattr(slave.as_raw_fd(), libc::TCSADRAIN, &mut tattr);
            }
        }

        let slave_clone = slave.try_clone().expect("failed to clone slave fd");
        let slave_clone2 = slave.try_clone().expect("failed to clone slave fd");

        child_stdin = unsafe { std::process::Stdio::from_raw_fd(slave.into_raw_fd()) };
        child_stdout = unsafe { std::process::Stdio::from_raw_fd(slave_clone.into_raw_fd()) };
        child_stderr = unsafe { std::process::Stdio::from_raw_fd(slave_clone2.into_raw_fd()) };
        master_fd_opt = Some(master);
    } else if !is_v2 {
        let adb_fd = adb_fd_opt.take().unwrap();
        let adb_fd_clone = adb_fd.try_clone().expect("failed to clone adb fd");
        let adb_fd_clone2 = adb_fd.try_clone().expect("failed to clone adb fd");

        child_stdin = unsafe { std::process::Stdio::from_raw_fd(adb_fd.into_raw_fd()) };
        child_stdout = unsafe { std::process::Stdio::from_raw_fd(adb_fd_clone.into_raw_fd()) };
        child_stderr = unsafe { std::process::Stdio::from_raw_fd(adb_fd_clone2.into_raw_fd()) };
    } else {
        child_stdin = std::process::Stdio::piped();
        child_stdout = std::process::Stdio::piped();
        child_stderr = std::process::Stdio::piped();
    }
    #[cfg(windows)]
    {
        child_stdin = std::process::Stdio::piped();
        child_stdout = std::process::Stdio::piped();
        child_stderr = std::process::Stdio::piped();
    }

    let mut cmd = if command_str.is_empty() {
        #[cfg(unix)]
        {
            std::process::Command::new("/bin/sh")
        }
        #[cfg(windows)]
        {
            std::process::Command::new("cmd.exe")
        }
    } else {
        #[cfg(unix)]
        {
            let mut c = std::process::Command::new("/bin/sh");
            c.arg("-c").arg(command_str);
            c
        }
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("cmd.exe");
            c.arg("/c").arg(command_str);
            c
        }
    };

    if !terminal_type.is_empty() {
        cmd.env("TERM", terminal_type);
    }

    let mut child = match cmd
        .stdin(child_stdin)
        .stdout(child_stdout)
        .stderr(child_stderr)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            log::error!("failed to spawn shell: {}", e);
            return;
        }
    };

    if !is_v2 && !is_pty {
        let _ = child.wait();
        return;
    }

    let mut adb_fd = adb_fd_opt.take().unwrap();

    let mut sub_stdin_fd = if is_pty {
        None
    } else {
        #[cfg(unix)]
        {
            child
                .stdin
                .take()
                .map(|s| unsafe { AdbFd::from_raw_fd(s.into_raw_fd()) })
        }
        #[cfg(windows)]
        {
            child
                .stdin
                .take()
                .map(|s| unsafe { AdbFd::from_raw_handle(s.into_raw_handle() as _) })
        }
    };
    let mut sub_stdout_fd = if is_pty {
        master_fd_opt.take()
    } else {
        #[cfg(unix)]
        {
            child
                .stdout
                .take()
                .map(|s| unsafe { AdbFd::from_raw_fd(s.into_raw_fd()) })
        }
        #[cfg(windows)]
        {
            child
                .stdout
                .take()
                .map(|s| unsafe { AdbFd::from_raw_handle(s.into_raw_handle() as _) })
        }
    };
    let mut sub_stderr_fd = if is_pty {
        None
    } else {
        #[cfg(unix)]
        {
            child
                .stderr
                .take()
                .map(|s| unsafe { AdbFd::from_raw_fd(s.into_raw_fd()) })
        }
        #[cfg(windows)]
        {
            child
                .stderr
                .take()
                .map(|s| unsafe { AdbFd::from_raw_handle(s.into_raw_handle() as _) })
        }
    };

    let mut pfds = Vec::new();
    pfds.push(AdbPollFd {
        #[cfg(unix)]
        fd: adb_fd.as_raw_fd(),
        #[cfg(windows)]
        fd: adb_fd.as_raw_socket() as usize,
        events: POLLIN,
        revents: 0,
    });
    if let Some(ref f) = sub_stdout_fd {
        pfds.push(AdbPollFd {
            #[cfg(unix)]
            fd: f.as_raw_fd(),
            #[cfg(windows)]
            fd: f.as_raw_handle() as usize,
            events: POLLIN,
            revents: 0,
        });
    }
    if let Some(ref f) = sub_stderr_fd {
        pfds.push(AdbPollFd {
            #[cfg(unix)]
            fd: f.as_raw_fd(),
            #[cfg(windows)]
            fd: f.as_raw_handle() as usize,
            events: POLLIN,
            revents: 0,
        });
    }

    let mut shell_read = ShellProtocol::new();

    loop {
        let n = adb_poll(&mut pfds, 100);

        if n > 0 {
            if pfds[0].revents & POLLIN != 0 {
                if is_v2 {
                    match shell_read.read(&mut adb_fd) {
                        Ok(true) => match shell_read.id {
                            ShellId::Stdin => {
                                let write_fd = if is_pty {
                                    sub_stdout_fd.as_mut()
                                } else {
                                    sub_stdin_fd.as_mut()
                                };
                                if let Some(f) = write_fd {
                                    let _ = f.write_all(&shell_read.data);
                                }
                            }
                            ShellId::WindowSizeChange => {
                                if is_pty {
                                    #[cfg(unix)]
                                    if let Some(ref f) = sub_stdout_fd {
                                        let s = String::from_utf8_lossy(&shell_read.data);
                                        if let Some((rows_cols, pixels)) = s.split_once(',') {
                                            if let (Some((rows, cols)), Some((xpix, ypix))) =
                                                (rows_cols.split_once('x'), pixels.split_once('x'))
                                            {
                                                if let (Ok(r), Ok(c), Ok(xp), Ok(yp)) = (
                                                    rows.parse::<u16>(),
                                                    cols.parse::<u16>(),
                                                    xpix.parse::<u16>(),
                                                    ypix.parse::<u16>(),
                                                ) {
                                                    let ws = libc::winsize {
                                                        ws_row: r,
                                                        ws_col: c,
                                                        ws_xpixel: xp,
                                                        ws_ypixel: yp,
                                                    };
                                                    unsafe {
                                                        libc::ioctl(
                                                            f.as_raw_fd(),
                                                            libc::TIOCSWINSZ,
                                                            &ws,
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            ShellId::CloseStdin => {
                                sub_stdin_fd.take();
                            }
                            _ => {}
                        },
                        _ => {
                            let _ = child.kill();
                            break;
                        }
                    }
                } else {
                    let mut buf = [0u8; 4096];
                    match adb_fd.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            let write_fd = if is_pty {
                                sub_stdout_fd.as_mut()
                            } else {
                                sub_stdin_fd.as_mut()
                            };
                            if let Some(f) = write_fd {
                                let _ = f.write_all(&buf[..n]);
                            }
                        }
                        _ => {
                            let _ = child.kill();
                            break;
                        }
                    }
                }
            }

            if pfds.len() > 1 && pfds[1].revents & POLLIN != 0 {
                let mut buf = [0u8; 4096];
                if let Some(ref mut f) = sub_stdout_fd {
                    match f.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            if is_v2 {
                                ShellProtocol::write_packet(
                                    &mut adb_fd,
                                    ShellId::Stdout,
                                    &buf[..n],
                                )
                                .unwrap();
                            } else {
                                adb_fd.write_all(&buf[..n]).unwrap();
                            }
                        }
                        _ => {
                            pfds[1].fd = -1;
                            sub_stdout_fd.take();
                        }
                    }
                }
            }

            if pfds.len() > 2 && pfds[2].revents & POLLIN != 0 {
                let mut buf = [0u8; 4096];
                if let Some(ref mut f) = sub_stderr_fd {
                    match f.read(&mut buf) {
                        Ok(n) if n > 0 => {
                            if is_v2 {
                                ShellProtocol::write_packet(
                                    &mut adb_fd,
                                    ShellId::Stderr,
                                    &buf[..n],
                                )
                                .unwrap();
                            } else {
                                adb_fd.write_all(&buf[..n]).unwrap();
                            }
                        }
                        _ => {
                            pfds[2].fd = -1;
                            sub_stderr_fd.take();
                        }
                    }
                }
            }
        } else if n < 0 {
            break;
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                let exit_code = status
                    .code()
                    .unwrap_or(if status.success() { 0 } else { 1 })
                    as u8;
                if is_v2 {
                    ShellProtocol::write_packet(&mut adb_fd, ShellId::Exit, &[exit_code]).unwrap();
                }
                break;
            }
            _ => {}
        }

        if sub_stdout_fd.is_none() && sub_stderr_fd.is_none() {
            let _ = child.wait();
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_service_thread;

    #[test]
    #[cfg(unix)]
    fn test_shell_service_echo() {
        let (mut s1, s2) = sysdeps::net::adb_socketpair().unwrap();

        let handle = std::thread::spawn(move || {
            shell_service(s2, ":echo hello");
        });

        let mut buf = [0u8; 64];
        let n = s1.read(&mut buf).unwrap();
        assert_eq!(String::from_utf8_lossy(&buf[..n]).trim(), "hello");

        handle.join().unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn test_raw_no_protocol_subprocess() {
        let (mut s1, s2) = sysdeps::net::adb_socketpair().unwrap();

        let handle = std::thread::spawn(move || {
            shell_service(s2, "raw:echo foo; echo bar >&2; [ -t 0 ]; echo $?");
        });

        let mut buf = String::new();
        s1.read_to_string(&mut buf).unwrap();
        let lines: Vec<&str> = buf.lines().collect();
        // Even when requesting a raw subprocess, without the shell protocol
        // we should always force a PTY to ensure proper cleanup.
        // PTY combines stdout and stderr.
        assert!(lines.contains(&"foo"));
        assert!(lines.contains(&"bar"));
        assert!(lines.contains(&"0"));

        handle.join().unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn test_raw_shell_protocol_subprocess() {
        let (mut s1, s2) = sysdeps::net::adb_socketpair().unwrap();

        let handle = std::thread::spawn(move || {
            shell_service(s2, "v2,raw:echo foo; echo bar >&2; echo baz; exit 24");
        });

        let mut protocol = ShellProtocol::new();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_code = None;

        while exit_code.is_none() {
            if protocol.read(&mut s1).unwrap() {
                match protocol.id {
                    ShellId::Stdout => stdout.push_str(&String::from_utf8_lossy(&protocol.data)),
                    ShellId::Stderr => stderr.push_str(&String::from_utf8_lossy(&protocol.data)),
                    ShellId::Exit => exit_code = Some(protocol.data[0]),
                    _ => {}
                }
            } else {
                break;
            }
        }

        assert_eq!(exit_code, Some(24));
        assert!(stdout.contains("foo"));
        assert!(stdout.contains("baz"));
        assert!(stderr.contains("bar"));

        handle.join().unwrap();
    }
}
