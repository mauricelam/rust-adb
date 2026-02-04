use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;
use std::os::unix::io::{RawFd, IntoRawFd};
use adb_sockets::{SocketRegistry, create_local_socket, Socket, internal};
use fdevent::fdevent::Fdevent;
use nix::sys::socket::{socketpair, AddressFamily, SockType, SockFlag};
use nix::fcntl::{fcntl, FcntlArg, OFlag};

fn set_nonblocking(fd: RawFd) {
    let flags = fcntl(fd, FcntlArg::F_GETFL).unwrap();
    let mut new_flags = OFlag::from_bits_truncate(flags);
    new_flags |= OFlag::O_NONBLOCK;
    fcntl(fd, FcntlArg::F_SETFL(new_flags)).unwrap();
}

#[test]
fn test_smoke() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();

    let (first_a_owned, first_b_owned) = socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
    let (last_a_owned, last_b_owned) = socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();

    let first_a = first_a_owned.into_raw_fd();
    let first_b = first_b_owned.into_raw_fd();
    let last_a = last_a_owned.into_raw_fd();
    let last_b = last_b_owned.into_raw_fd();

    // Only set the ends managed by fdevent to non-blocking
    set_nonblocking(first_b);
    set_nonblocking(last_a);

    let mut prev_tail = create_local_socket(first_b, registry.clone(), &mut fdevent);

    const INTERMEDIATE_COUNT: usize = 50;
    for _ in 0..INTERMEDIATE_COUNT {
        let (pair_a_owned, pair_b_owned) = socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
        let pair_a = pair_a_owned.into_raw_fd();
        let pair_b = pair_b_owned.into_raw_fd();
        set_nonblocking(pair_a);
        set_nonblocking(pair_b);

        let head = create_local_socket(pair_a, registry.clone(), &mut fdevent);
        let tail = create_local_socket(pair_b, registry.clone(), &mut fdevent);

        prev_tail.set_peer(head.clone() as Arc<dyn Socket>);
        head.set_peer(prev_tail.clone() as Arc<dyn Socket>);
        prev_tail.ready();

        prev_tail = tail;
    }

    let end = create_local_socket(last_a, registry.clone(), &mut fdevent);
    prev_tail.set_peer(end.clone() as Arc<dyn Socket>);
    end.set_peer(prev_tail.clone() as Arc<dyn Socket>);
    prev_tail.ready();

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    // Move fdevent to a thread
    let thread_handle = thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        }
    });

    const MESSAGE: &[u8] = b"socket_test";
    const LOOP_COUNT: usize = 100;

    for _ in 0..LOOP_COUNT {
        nix::unistd::write(first_a, MESSAGE).unwrap();
        let mut buf = [0u8; 11];
        // Read might still need to wait because it's through many sockets.
        // But since first_a and last_b are blocking, it should be fine.
        let mut total_read = 0;
        while total_read < MESSAGE.len() {
            let n = nix::unistd::read(last_b, &mut buf[total_read..]).unwrap();
            total_read += n;
        }
        assert_eq!(&buf, MESSAGE);
    }

    running.store(false, Ordering::SeqCst);
    thread_handle.join().unwrap();

    let _ = nix::unistd::close(first_a);
    let _ = nix::unistd::close(last_b);
}

#[test]
fn test_parse_host_service() {
    let cases = [
        ("usb:foo:bar", Some(("usb:foo", "bar"))),
        ("tcp:foo:123:bar", Some(("tcp:foo:123", "bar"))),
        ("tcp:[::1]:5555:foo", Some(("tcp:[::1]:5555", "foo"))),
        ("device:serial:command", Some(("device:serial", "command"))),
    ];

    for (input, expected) in cases {
        assert_eq!(internal::parse_host_service(input), expected);
    }
}
