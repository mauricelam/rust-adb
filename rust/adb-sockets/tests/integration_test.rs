use adb_sockets::{create_local_socket, create_remote_socket, Socket, SocketRegistry, Transport};
use adb_types::Apacket;
use fdevent::fdevent::Fdevent;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;
use std::io::{Read, Write};
use sysdeps::AdbFd;

struct MockTransport {
    packets: Arc<Mutex<Vec<Apacket>>>,
}

impl Transport for MockTransport {
    fn id(&self) -> u64 { 1 }
    fn send_packet(&self, packet: Apacket) { self.packets.lock().unwrap().push(packet); }
    fn send_ready(&self, _local: u32, _remote: u32, _ack_bytes: u32) {}
    fn get_max_payload(&self) -> usize { 1024 }
    fn supports_delayed_ack(&self) -> bool { false }
}

#[test]
fn test_remote_to_local_flow() {
    let registry = Arc::new(SocketRegistry::new());
    let mut fdevent = Fdevent::new().unwrap();

    let (pair_a, mut pair_b) = sysdeps::net::adb_socketpair().unwrap();

    let local = create_local_socket(pair_a, registry.clone(), &mut fdevent);

    let packets = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(MockTransport { packets: packets.clone() });
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

    pair_b.write_all(b"hello").unwrap();

    let mut success = false;
    for _ in 0..100 {
        {
            let pkts = packets.lock().unwrap();
            if !pkts.is_empty() {
                let p = &pkts[0];
                if p.msg.command == adb_protocol::A_WRTE && p.msg.arg1 == 100 && p.payload.get_ref() == b"hello" {
                    success = true;
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    running.store(false, Ordering::SeqCst);
    thread_handle.join().unwrap();
    pair_b.close();

    assert!(success, "No packets received by transport");
}
