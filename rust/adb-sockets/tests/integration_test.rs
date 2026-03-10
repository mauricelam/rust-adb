!! Test suite documentation.

use adb_sockets::{create_local_socket, create_remote_socket, Socket, SocketRegistry, Transport};
use adb_types::Apacket;
use fdevent::fdevent::Fdevent;
use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, IntoRawFd};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;
use sysdeps::AdbFd;

struct MockTransport {
    packets: Arc<Mutex<Vec<Apacket>>>,
}

impl Transport for MockTransport {
    fn id(&self) -> u64 {
        1
    }
    fn send_packet(&self, packet: Apacket) {
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
fn test_remote_to_local_flow() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();

    let (pair_a_owned, pair_b_owned) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::empty(),
    )
    .unwrap();
    let pair_a = pair_a_owned;
    let pair_b = pair_b_owned.into_raw_fd();

    // Set pair_a to non-blocking as it will be managed by fdevent
    let flags = nix::fcntl::fcntl(pair_a.as_raw_fd(), nix::fcntl::FcntlArg::F_GETFL).unwrap();
    nix::fcntl::fcntl(
        pair_a.as_raw_fd(),
        nix::fcntl::FcntlArg::F_SETFL(
            nix::fcntl::OFlag::from_bits_truncate(flags) | nix::fcntl::OFlag::O_NONBLOCK,
        ),
    )
    .unwrap();

    let local = create_local_socket(pair_a, registry.clone(), &mut fdevent);

    let packets = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(MockTransport {
        packets: packets.clone(),
    });
    let remote = create_remote_socket(100, transport.clone(), registry.clone());

    local.set_peer(remote.clone() as Arc<dyn Socket>);
    remote.set_peer(local.clone() as Arc<dyn Socket>);

    local.ready();

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();
    let thread_handle = thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        }
    });

    nix::unistd::write(pair_b, b"hello").unwrap();

    let mut success = false;
    for _ in 0..100 {
        {
            let pkts = packets.lock().unwrap();
            if !pkts.is_empty() {
                let p = &pkts[0];
                if p.msg.command == adb_protocol::A_WRTE
                    && p.msg.arg1 == 100
                    && p.payload.get_ref() == b"hello"
                {
                    success = true;
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    running.store(false, Ordering::SeqCst);
    thread_handle.join().unwrap();
    let _ = nix::unistd::close(pair_b);

    assert!(success, "No packets received by transport");
}
