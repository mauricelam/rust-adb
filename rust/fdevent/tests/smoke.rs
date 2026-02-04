//! Smoke tests for the `fdevent` crate.
//!
//! These tests mirror the `smoke` test in `fdevent_test.cpp`, creating a
//! chain of socket pairs and passing a message through them.

use fdevent::fdevent::{Fdevent, FdeventHandler};
use mio::unix::SourceFd;
use mio::{Interest, Token};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

struct SharedPipe {
    reader: UnixStream,
    writer: UnixStream,
    queue: VecDeque<u8>,
    writer_token: Token,
}

struct ReaderHandler {
    state: Arc<Mutex<SharedPipe>>,
}

impl FdeventHandler for ReaderHandler {
    fn on_event(&mut self, _event: &mio::event::Event, registry: &mio::Registry) {
        let mut state = self.state.lock().unwrap();
        let mut buf = [0; 1024];
        let mut added = false;
        loop {
            match state.reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for i in 0..n {
                        state.queue.push_back(buf[i]);
                    }
                    added = true;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("read error: {}", e),
            }
        }
        if added {
            registry
                .reregister(
                    &mut SourceFd(&state.writer.as_raw_fd()),
                    state.writer_token,
                    Interest::WRITABLE,
                )
                .ok();
        }
    }
    fn on_timeout(&mut self) {}
}

struct WriterHandler {
    state: Arc<Mutex<SharedPipe>>,
}

impl FdeventHandler for WriterHandler {
    fn on_event(&mut self, _event: &mio::event::Event, registry: &mio::Registry) {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(b) = state.queue.pop_front() {
                match state.writer.write(&[b]) {
                    Ok(1) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        state.queue.push_front(b);
                        break;
                    }
                    Err(e) => panic!("write error: {}", e),
                    _ => {
                        state.queue.push_front(b);
                        break;
                    }
                }
            } else {
                // Queue empty, stop listening for WRITABLE
                registry
                    .reregister(
                        &mut SourceFd(&state.writer.as_raw_fd()),
                        state.writer_token,
                        Interest::READABLE,
                    )
                    .ok();
                break;
            }
        }
    }
    fn on_timeout(&mut self) {}
}

/// Tests passing a message through a chain of 10 handlers.
#[test]
fn smoke() {
    let mut fdevent = Fdevent::new().unwrap();

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
        let state = Arc::new(Mutex::new(SharedPipe {
            reader: read_fds[i].try_clone().unwrap(),
            writer: write_fds[i].try_clone().unwrap(),
            queue: VecDeque::new(),
            writer_token: Token(0), // Temporary
        }));

        let writer_handler = Box::new(WriterHandler {
            state: state.clone(),
        });
        let w_token = fdevent
            .register(&write_fds[i], writer_handler, Interest::WRITABLE)
            .unwrap();
        state.lock().unwrap().writer_token = w_token;

        let reader_handler = Box::new(ReaderHandler {
            state: state.clone(),
        });
        fdevent
            .register(&read_fds[i], reader_handler, Interest::READABLE)
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
    writer.write_all(message.as_bytes()).unwrap();

    let mut buf = vec![0; message.len()];
    let mut total_read = 0;
    let start = std::time::Instant::now();
    while total_read < message.len() {
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "Timeout waiting for read. Read {}/{}",
                total_read,
                message.len()
            );
        }
        match reader.read(&mut buf[total_read..]) {
            Ok(n) if n > 0 => total_read += n,
            Ok(0) => panic!("Unexpected EOF"),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("Read error: {}", e),
            _ => {}
        }
    }
    assert_eq!(message.as_bytes(), &buf[..]);

    *stop.lock().unwrap() = true;
    handle.join().unwrap();
}
