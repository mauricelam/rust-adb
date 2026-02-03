//! Smoke tests for the `fdevent` crate.
//!
//! These tests mirror the `smoke` test in `fdevent_test.cpp`, creating a
//! chain of socket pairs and passing a message through them.

use fdevent::fdevent::{Fdevent, FdeventHandler};
use mio::Interest;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// A handler that reads from one end of a socket pair and writes to another.
struct FdHandler {
    reader: UnixStream,
    writer: UnixStream,
    queue: Arc<Mutex<Vec<u8>>>,
}

impl FdeventHandler for FdHandler {
    fn on_event(&mut self, event: &mio::event::Event) {
        if event.is_readable() {
            let mut buf = [0; 1];
            match self.reader.read(&mut buf) {
                Ok(1) => {
                    let mut queue = self.queue.lock().unwrap();
                    queue.push(buf[0]);
                }
                Ok(0) => {}
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("read error: {}", e),
            }
        }
        if event.is_writable() {
            let mut queue = self.queue.lock().unwrap();
            if !queue.is_empty() {
                let data = queue.remove(0);
                if let Err(e) = self.writer.write(&[data]) {
                    if e.kind() != io::ErrorKind::WouldBlock {
                        panic!("write error: {}", e);
                    }
                }
            }
        }
    }
    fn on_timeout(&mut self) {}
}

/// Tests passing a message through a chain of 10 handlers.
#[test]
fn smoke() {
    let mut fdevent = Fdevent::new().unwrap();
    let queue = Arc::new(Mutex::new(Vec::new()));

    // Create a chain of socket pairs.
    // Main thread writes to first_w.
    // first_r is handled by FdHandler 0, which writes to write_fds[0].
    // read_fds[1] is handled by FdHandler 1, and so on.
    // Finally, FdHandler 10 writes to last_w, and main thread reads from last_r.

    let (first_r, mut writer) = UnixStream::pair().unwrap();
    first_r.set_nonblocking(true).unwrap();
    writer.set_nonblocking(true).unwrap();

    let mut read_fds = vec![first_r];
    let mut write_fds = Vec::new();

    for _ in 0..10 {
        let (r, w) = UnixStream::pair().unwrap();
        r.set_nonblocking(true).unwrap();
        w.set_nonblocking(true).unwrap();
        read_fds.push(r);
        write_fds.push(w);
    }

    let (mut reader, last_w) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    last_w.set_nonblocking(true).unwrap();
    write_fds.push(last_w);

    for i in 0..read_fds.len() {
        let r = read_fds[i].try_clone().unwrap();
        let w = write_fds[i].try_clone().unwrap();
        let handler = Box::new(FdHandler {
            reader: r.try_clone().unwrap(),
            writer: w.try_clone().unwrap(),
            queue: queue.clone(),
        });
        fdevent
            .register(&r, handler, Interest::READABLE | Interest::WRITABLE)
            .unwrap();
    }

    let stop = Arc::new(Mutex::new(false));
    let stop_clone = stop.clone();
    let handle = thread::spawn(move || {
        while !*stop_clone.lock().unwrap() {
            fdevent.poll(Some(Duration::from_millis(10))).unwrap();
        }
    });

    let message = "fdevent_test";
    for c in message.chars() {
        let b = [c as u8];
        writer.write_all(&b).unwrap();

        let mut buf = [0; 1];
        let mut total_read = 0;
        let start = std::time::Instant::now();
        while total_read < 1 {
            if start.elapsed() > Duration::from_secs(5) {
                panic!("Timeout waiting for read");
            }
            match reader.read(&mut buf) {
                Ok(1) => total_read += 1,
                Ok(0) => panic!("Unexpected EOF"),
                Ok(_) => {},
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("Read error: {}", e),
            }
        }
        assert_eq!(c as u8, buf[0]);
    }

    *stop.lock().unwrap() = true;
    handle.join().unwrap();
}
