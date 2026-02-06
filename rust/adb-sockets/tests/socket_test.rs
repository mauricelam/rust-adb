use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;
use adb_sockets::{SocketRegistry, create_local_socket, Socket, internal};
use fdevent::fdevent::Fdevent;
use std::io::{Read, Write};
use sysdeps::AdbFd;

#[test]
fn test_smoke() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (first_a, first_b) = sysdeps::net::adb_socketpair().unwrap();
    let (last_a, last_b) = sysdeps::net::adb_socketpair().unwrap();

    let mut prev_tail = create_local_socket(first_b, registry.clone(), &mut fdevent);
    const INTERMEDIATE_COUNT: usize = 20;
    for _ in 0..INTERMEDIATE_COUNT {
        let (pair_a, pair_b) = sysdeps::net::adb_socketpair().unwrap();
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
    let thread_handle = thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        }
    });

    const MESSAGE: &[u8] = b"socket_test";
    const LOOP_COUNT: usize = 10;
    let mut first_a = first_a;
    let mut last_b = last_b;
    for _ in 0..LOOP_COUNT {
        first_a.write_all(MESSAGE).unwrap();
        let mut buf = [0u8; 11];
        last_b.read_exact(&mut buf).unwrap();
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
    for (input, expected) in cases { assert_eq!(internal::parse_host_service(input), expected); }
}

struct MockTransport { packets: Arc<Mutex<Vec<adb_types::Apacket>>> }
impl adb_sockets::Transport for MockTransport {
    fn id(&self) -> u64 { 1 }
    fn send_packet(&self, packet: adb_types::Apacket) { self.packets.lock().unwrap().push(packet); }
    fn send_ready(&self, _local: u32, _remote: u32, _ack_bytes: u32) {}
    fn get_max_payload(&self) -> usize { 1024 }
    fn supports_delayed_ack(&self) -> bool { false }
}

#[test]
fn test_connect_to_remote() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1, _s2) = sysdeps::net::adb_socketpair().unwrap();
    let socket = create_local_socket(s1, registry, &mut fdevent);
    let packets = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(MockTransport { packets: packets.clone() });
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
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1, mut s2) = sysdeps::net::adb_socketpair().unwrap();
    let socket = create_local_socket(s1, registry.clone(), &mut fdevent);

    // Enqueue some data.
    socket.enqueue(bytes::Bytes::from("hello"));
    socket.close();

    // Run fdevent.
    for _ in 0..10 { fdevent.poll(Some(Duration::from_millis(10))).unwrap(); }

    let mut buf = [0u8; 5];
    s2.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"hello");

    // After flushing, it should be destroyed.
    for _ in 0..10 { fdevent.poll(Some(Duration::from_millis(10))).unwrap(); }
    assert!(registry.find(socket.id()).is_none());
}

#[test]
fn test_read_from_closing_socket() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1, mut s2) = sysdeps::net::adb_socketpair().unwrap();
    let socket = create_local_socket(s1, registry.clone(), &mut fdevent);
    socket.enqueue(bytes::Bytes::from("hello"));
    socket.close();
    let mut buf = vec![0u8; 5];
    let mut total_read = 0;
    while total_read < 5 {
        match s2.read(&mut buf[total_read..]) {
            Ok(n) if n > 0 => total_read += n,
            _ => { fdevent.poll(Some(Duration::from_millis(10))).unwrap(); }
        }
    }
    assert_eq!(&buf, b"hello");
}

#[test]
fn test_close_socket_in_close_wait_state() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();
    let (s1, s2) = sysdeps::net::adb_socketpair().unwrap();
    let socket = create_local_socket(s1, registry.clone(), &mut fdevent);
    assert!(registry.find(socket.id()).is_some());
    drop(s2); // Close remote end.
    for _ in 0..10 { fdevent.poll(Some(Duration::from_millis(10))).unwrap(); }
    assert!(registry.find(socket.id()).is_none());
}
