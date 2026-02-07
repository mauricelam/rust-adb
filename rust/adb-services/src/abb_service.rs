/*
 * Copyright (C) 2023 The Android Open Source Project
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

use adb_io::send_protocol_string;
use sysdeps::AdbFd;
use std::sync::Mutex;
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
#[cfg(unix)]
use nix::sys::socket::{recvmsg, ControlMessageOwned, MsgFlags};

pub struct AbbProcess {
    socket: Mutex<Option<AdbFd>>,
}

impl AbbProcess {
    pub const fn new() -> Self {
        Self {
            socket: Mutex::new(None),
        }
    }

    #[cfg(unix)]
    fn start_abb_process() -> std::io::Result<AdbFd> {
        let (s0, s1) = sysdeps::net::adb_socketpair()?;

        // Spawn "abb" process
        let mut child = Command::new("abb")
            .stdin(unsafe { Stdio::from_raw_fd(s1.try_into_owned_fd().unwrap().into_raw_fd()) })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        // We don't really want to wait for it here, it should run in background.
        // In a real adbd, this would be managed more carefully.
        std::thread::spawn(move || {
            let _ = child.wait();
        });

        Ok(s0)
    }

    #[cfg(unix)]
    pub fn send_command(&self, command: &str) -> Option<AdbFd> {
        let mut socket_guard = self.socket.lock().unwrap();

        for _ in 0..2 {
            if socket_guard.is_none() {
                *socket_guard = Self::start_abb_process().ok();
            }

            let socket = match socket_guard.as_mut() {
                Some(s) => s,
                None => return None,
            };

            if let Err(_) = send_protocol_string(socket, command) {
                *socket_guard = None;
                continue;
            }

            // Receive FD
            let mut buf = [0u8; 1];
            let mut iov = [std::io::IoSliceMut::new(&mut buf)];
            let mut cmsg_buf = nix::cmsg_space!(RawFd);

            match recvmsg::<()>(socket.as_raw_fd(), &mut iov, Some(&mut cmsg_buf), MsgFlags::empty()) {
                Ok(msg) => {
                    for cmsg in msg.cmsgs() {
                        if let ControlMessageOwned::ScmRights(fds) = cmsg {
                            if !fds.is_empty() {
                                return Some(unsafe { AdbFd::from_raw_fd(fds[0]) });
                            }
                        }
                    }
                    // No FD received
                    *socket_guard = None;
                }
                Err(_) => {
                    *socket_guard = None;
                }
            }
        }

        None
    }

    #[cfg(not(unix))]
    pub fn send_command(&self, _command: &str) -> Option<AdbFd> {
        None
    }
}

static ABB_PROCESS: AbbProcess = AbbProcess::new();

pub fn execute_abb_command(command: &str) -> Option<AdbFd> {
    ABB_PROCESS.send_command(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use nix::sys::socket::{sendmsg, ControlMessage, MsgFlags};

    #[test]
    #[cfg(unix)]
    fn test_abb_send_command() {
        let (s0, s1) = sysdeps::net::adb_socketpair().unwrap();

        let abb_mock = std::thread::spawn(move || {
            let mut s1 = s1;
            let mut buf = [0u8; 1024];
            // Read length-prefixed command
            let _n = s1.read(&mut buf).unwrap();
            let cmd_len = usize::from_str_radix(std::str::from_utf8(&buf[..4]).unwrap(), 16).unwrap();
            let cmd = std::str::from_utf8(&buf[4..4+cmd_len]).unwrap();
            assert_eq!(cmd, "test-command");

            // Create a dummy FD to send back
            let (p0, _p1) = sysdeps::net::adb_socketpair().unwrap();
            let fd = p0.try_into_owned_fd().unwrap().into_raw_fd();

            let iov = [std::io::IoSlice::new(&[0u8])];
            let cmsgs = [ControlMessage::ScmRights(&[fd])];
            sendmsg::<()>(s1.as_raw_fd(), &iov, &cmsgs, MsgFlags::empty(), None).unwrap();
        });

        let abb_process = AbbProcess::new();
        *abb_process.socket.lock().unwrap() = Some(s0);

        let fd = abb_process.send_command("test-command").expect("Failed to get FD");
        assert!(fd.as_raw_fd() >= 0);

        abb_mock.join().unwrap();
    }
}
