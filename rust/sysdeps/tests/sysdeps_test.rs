!! Test suite documentation.

#![cfg(unix)]
use sysdeps::poll::{adb_poll, AdbPollFd};
use std::os::unix::net::UnixStream;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use libc;
use serial_test::serial;

#[test]
#[serial]
fn test_sysdeps_socketpair_smoke() {
    let (mut s1, mut s2) = UnixStream::pair().expect("socketpair failed");
    s1.write_all(b"foo\0").unwrap();
    s2.write_all(b"bar\0").unwrap();

    let mut buf = [0u8; 4];
    s2.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"foo\0");
    s1.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"bar\0");
}

#[test]
#[serial]
fn test_sysdeps_fd_exhaustion() {
    let mut fds = Vec::new();
    loop {
        match UnixStream::pair() {
            Ok((s1, s2)) => {
                fds.push(s1);
                fds.push(s2);
            }
            Err(e) => {
                assert_eq!(e.raw_os_error(), Some(libc::EMFILE));
                break;
            }
        }
    }

    // Now close them all.
    let first_s1_fd = fds[0].as_raw_fd();
    let first_s2_fd = fds[1].as_raw_fd();
    drop(fds);

    // Try again, it should succeed and ideally give us back the same FDs.
    let (s1, s2) = UnixStream::pair().expect("socketpair failed after recovering from EMFILE");
    assert_eq!(s1.as_raw_fd(), first_s1_fd);
    assert_eq!(s2.as_raw_fd(), first_s2_fd);
}

#[test]
#[serial]
fn test_sysdeps_poll_smoke() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let mut pfds = [
        AdbPollFd { fd: s1.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        AdbPollFd { fd: s2.as_raw_fd(), events: libc::POLLOUT, revents: 0 },
    ];

    // s2 is writable
    assert_eq!(adb_poll(&mut pfds, 0), 1);
    assert_eq!(pfds[0].revents, 0);
    assert_ne!(pfds[1].revents & libc::POLLOUT, 0);

    let mut s2_write = s2;
    s2_write.write_all(b"foo\0").unwrap();

    // s1 is now readable
    assert_eq!(adb_poll(&mut pfds[..1], 100), 1);
    assert_ne!(pfds[0].revents & libc::POLLIN, 0);
}

#[test]
#[serial]
fn test_sysdeps_poll_timeout() {
    let (s1, _s2) = UnixStream::pair().unwrap();
    let mut pfd = AdbPollFd { fd: s1.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    assert_eq!(adb_poll(std::slice::from_mut(&mut pfd), 100), 0);
}

#[test]
#[serial]
fn test_sysdeps_poll_invalid_fd() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let mut pfds = [
        AdbPollFd { fd: s1.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        AdbPollFd { fd: i32::MAX, events: libc::POLLIN, revents: 0 },
        AdbPollFd { fd: s2.as_raw_fd(), events: libc::POLLOUT, revents: 0 },
    ];

    // s2 is writable, i32::MAX is invalid
    assert_eq!(adb_poll(&mut pfds, 0), 2);
    assert_eq!(pfds[0].revents, 0);
    assert_ne!(pfds[1].revents & libc::POLLNVAL, 0);
    assert_ne!(pfds[2].revents & libc::POLLOUT, 0);
}

#[test]
#[cfg(not(target_os = "macos"))]
#[serial]
fn test_sysdeps_poll_duplicate_fd() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let fd = s1.as_raw_fd();
    let mut pfds = [
        AdbPollFd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        },
        AdbPollFd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        },
    ];

    assert_eq!(adb_poll(&mut pfds, 0), 0);
    assert_eq!(pfds[0].revents, 0);
    assert_eq!(pfds[1].revents, 0);

    let mut s2_write = s2;
    s2_write.write_all(b"foo\0").unwrap();

    assert_eq!(adb_poll(&mut pfds, 100), 2);
    assert_ne!(pfds[0].revents & libc::POLLIN, 0);
    assert_ne!(pfds[1].revents & libc::POLLIN, 0);
}

#[test]
#[serial]
fn test_sysdeps_poll_disconnect() {
    let (s1, s2) = UnixStream::pair().unwrap();
    let mut pfd = AdbPollFd { fd: s1.as_raw_fd(), events: libc::POLLIN, revents: 0 };

    assert_eq!(adb_poll(std::slice::from_mut(&mut pfd), 0), 0);

    drop(s2);

    assert!(adb_poll(std::slice::from_mut(&mut pfd), 100) >= 1);
    assert_ne!(pfd.revents & (libc::POLLIN | libc::POLLHUP), 0);
}

#[test]
#[serial]
fn test_sysdeps_poll_fd_count() {
    let num_sockets = 256;
    let mut sockets = Vec::new();
    let mut pfds = Vec::new();

    for i in 0..num_sockets {
        let (mut s1, s2) = UnixStream::pair().unwrap();
        s1.write_all(&(i as i32).to_ne_bytes()).unwrap();
        pfds.push(AdbPollFd { fd: s2.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        sockets.push((s1, s2));
    }

    assert_eq!(adb_poll(&mut pfds, 0), num_sockets as i32);
    for i in 0..num_sockets {
        assert_ne!(pfds[i].revents & libc::POLLIN, 0);
        let mut buf = [0u8; 4];
        sockets[i].1.read_exact(&mut buf).unwrap();
        assert_eq!(i as i32, i32::from_ne_bytes(buf));
    }
}

#[test]
#[serial]
fn test_sysdeps_condition_variable_smoke() {
    use std::sync::{Arc, Mutex, Condvar};
    use std::thread;

    let pair = Arc::new((Mutex::new(false), Condvar::new()));
    let pair2 = Arc::clone(&pair);

    thread::spawn(move || {
        let (lock, cvar) = &*pair2;
        let mut started = lock.lock().unwrap();
        *started = true;
        cvar.notify_one();
    });

    let (lock, cvar) = &*pair;
    let mut started = lock.lock().unwrap();
    while !*started {
        started = cvar.wait(started).unwrap();
    }
}
