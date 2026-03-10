!! Test suite documentation.

use adb_sockets::{create_local_socket, internal, Socket, SocketRegistry};
use fdevent::fdevent::Fdevent;
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;
use sysdeps::AdbFd;

fn set_nonblocking(fd: RawFd) {
    let flags = fcntl(fd, FcntlArg::F_GETFL).expect("F_GETFL failed");
    let mut flags = OFlag::from_bits_truncate(flags);
    flags.insert(OFlag::O_NONBLOCK);
    let _ = fcntl(fd, FcntlArg::F_SETFL(flags));
}

#[test]
fn test_smoke() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();

    let (first_a_owned, first_b_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let (last_a_owned, last_b_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();

    let first_a = first_a_owned.into_raw_fd();
    let first_b = first_b_owned;
    let last_a = last_a_owned;
    let last_b = last_b_owned.into_raw_fd();

    // Only set the ends managed by fdevent to non-blocking
    set_nonblocking(first_b.as_raw_fd());
    set_nonblocking(last_a.as_raw_fd());

    let mut prev_tail = create_local_socket(first_b, registry.clone(), &mut fdevent);
    const INTERMEDIATE_COUNT: usize = 20;
    for _ in 0..INTERMEDIATE_COUNT {
        let (pair_a_owned, pair_b_owned) = socketpair(
            AddressFamily::Unix,
            SockType::Stream,
            None,
            SockFlag::empty(),
        )
        .unwrap();
        set_nonblocking(pair_a_owned.as_raw_fd());
        set_nonblocking(pair_b_owned.as_raw_fd());

        let head = create_local_socket(pair_a_owned, registry.clone(), &mut fdevent);
        let tail = create_local_socket(pair_b_owned, registry.clone(), &mut fdevent);

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
    let thread_handle = thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        }
    });

    const MESSAGE: &[u8] = b"socket_test";
    const LOOP_COUNT: usize = 10;
    let first_a = first_a;
    let last_b = last_b;
    for _ in 0..LOOP_COUNT {
        nix::unistd::write(first_a, MESSAGE).unwrap();
        let mut buf = [0u8; 11];
        let mut total = 0;
        while total < buf.len() {
            let n = nix::unistd::read(last_b, &mut buf[total..]).unwrap();
            if n == 0 {
                break;
            }
            total += n;
        }
        assert_eq!(&buf, MESSAGE);
    }
    running.store(false, Ordering::SeqCst);
    thread_handle.join().unwrap();
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

struct MockTransport {
    packets: Arc<Mutex<Vec<adb_types::Apacket>>>,
}
impl adb_sockets::Transport for MockTransport {
    fn id(&self) -> u64 {
        1
    }
    fn send_packet(&self, packet: adb_types::Apacket) {
        self.packets.lock().unwrap().push(packet);
    }
    fn send_ready(&self, _local: u32, _remote: u32, _ack_bytes: u32) {}
    fn get_max_payload(&self) -> usize {
        1024
    }
    fn supports_delayed_ack(&self) -> bool {
        false
    }
}

#[test]
fn test_connect_to_remote() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1_owned, _s2_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    set_nonblocking(s1_owned.as_raw_fd());

    let socket = create_local_socket(s1_owned, registry, &mut fdevent);
    let packets = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(MockTransport {
        packets: packets.clone(),
    });
    socket.set_transport(transport);
    adb_sockets::connect_to_remote(&socket, "shell:ls");
    let p = &packets.lock().unwrap()[0];
    assert_eq!(p.msg.command, adb_protocol::A_OPEN);
    assert_eq!(p.msg.arg0, socket.id());
    let payload = String::from_utf8_lossy(p.payload.get_ref());
    assert!(payload.starts_with("shell:ls"));
}

#[test]
fn test_close_socket_with_packet() {
    let _ = env_logger::builder().is_test(true).try_init();
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1_owned, s2_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let s1 = s1_owned.as_raw_fd();
    let s2 = s2_owned.into_raw_fd();
    set_nonblocking(s1);
    set_nonblocking(s2);

    let socket = create_local_socket(s1_owned, registry.clone(), &mut fdevent);

    // Enqueue some data.
    socket.enqueue(bytes::Bytes::from("hello"));
    socket.close();

    // Run fdevent.
    for _ in 0..10 {
        fdevent.poll(Some(Duration::from_millis(10))).unwrap();
    }

    let mut buf = [0u8; 5];
    let mut total = 0;
    while total < 5 {
        let n = nix::unistd::read(s2, &mut buf[total..]).unwrap();
        if n == 0 {
            break;
        }
        total += n;
    }
    assert_eq!(&buf, b"hello");

    // After flushing, it should be destroyed.
    for _ in 0..10 {
        fdevent.poll(Some(Duration::from_millis(10))).unwrap();
    }
    assert!(registry.find(socket.id()).is_none());
    // The FD might be closed already if it flushed quickly.
    let _ = fcntl(s1, FcntlArg::F_GETFD);

    // Now read from s2 to clear the buffer.
    let mut buf = [0u8; 1024];
    while let Ok(n) = nix::unistd::read(s2, &mut buf) {
        if n == 0 {
            break;
        }
    }

    // Run fdevent until s1 is closed or timeout.
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        let res = fcntl(s1, FcntlArg::F_GETFD);
        // println!("Polling... fcntl({}) = {:?}", s1, res);
        if res.is_err() {
            break;
        }
    }

    // Check if s1 is closed.
    assert!(fcntl(s1, FcntlArg::F_GETFD).is_err());
    let _ = nix::unistd::close(s2);
}

#[test]
fn test_read_from_closing_socket() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1_owned, s2_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let s1 = s1_owned.as_raw_fd();
    let s2 = s2_owned.into_raw_fd();
    set_nonblocking(s1);
    set_nonblocking(s2);

    let socket = create_local_socket(s1_owned, registry.clone(), &mut fdevent);

    // Block s1.
    let data = vec![0u8; 1024 * 1024];
    while nix::unistd::write(s1, &data).is_ok() {}

    // Enqueue "hello".
    socket.enqueue(bytes::Bytes::from("hello"));
    socket.close();
    let mut buf = Vec::new();
    let mut temp = [0u8; 1024];
    loop {
        match nix::unistd::read(s2, &mut temp) {
            Ok(n) if n > 0 => {
                buf.extend_from_slice(&temp[..n]);
                // Poll to allow the writer to make progress draining its queue
                fdevent.poll(Some(Duration::from_millis(1))).unwrap();
            }
            Ok(0) => break,
            _ => {
                fdevent.poll(Some(Duration::from_millis(10))).unwrap();
            }
        }
    }

    assert!(String::from_utf8_lossy(&buf).contains("hello"));

    let _ = nix::unistd::close(s2);
}

#[test]
fn test_write_error_when_having_packets() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1_owned, s2_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let s1 = s1_owned.as_raw_fd();
    let s2 = s2_owned.into_raw_fd();
    set_nonblocking(s1);

    let socket = create_local_socket(s1_owned, registry.clone(), &mut fdevent);

    // Fill the socket buffer so write blocks.
    let data = vec![0u8; 1024 * 1024];
    while nix::unistd::write(s1, &data).is_ok() {}

    socket.enqueue(bytes::Bytes::from("hello"));

    // Close the other end to cause write error on next flush.
    let _ = nix::unistd::close(s2);

    // Run fdevent.
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(1) {
        fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        if fcntl(s1, FcntlArg::F_GETFD).is_err() {
            break;
        }
    }

    // Socket should have been destroyed due to write error.
    assert!(registry.find(socket.id()).is_none());
    assert!(fcntl(s1, FcntlArg::F_GETFD).is_err());
}

#[test]
fn test_flush_after_shutdown() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1_owned, s2_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let s1 = s1_owned.as_raw_fd();
    let s2 = s2_owned.into_raw_fd();
    set_nonblocking(s1);
    set_nonblocking(s2);

    let socket = create_local_socket(s1_owned, registry.clone(), &mut fdevent);

    // Block.
    let data = vec![0u8; 1024 * 1024];
    while nix::unistd::write(s1, &data).is_ok() {}
    socket.enqueue(bytes::Bytes::from("hello"));

    socket.shutdown(); // shutdown does nothing in current Rust impl, matching C++ mostly.

    // Clear s2.
    let mut buf = [0u8; 1024];
    while nix::unistd::read(s2, &mut buf).is_ok() {}

    fdevent.poll(Some(Duration::from_millis(10))).unwrap();

    // Should have flushed.
    let n = nix::unistd::read(s2, &mut buf).unwrap();
    assert!(n > 0);

    let _ = nix::unistd::close(s2);
}

#[test]
fn test_close_socket_in_close_wait_state() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1_owned, s2_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let s1 = s1_owned.as_raw_fd();
    let s2 = s2_owned.into_raw_fd();
    set_nonblocking(s1);

    let socket = create_local_socket(s1_owned, registry.clone(), &mut fdevent);
    assert!(registry.find(socket.id()).is_some());
    let _ = nix::unistd::close(s2); // Close remote end.
    for _ in 0..10 {
        fdevent.poll(Some(Duration::from_millis(10))).unwrap();
    }
    assert!(registry.find(socket.id()).is_none());
}
