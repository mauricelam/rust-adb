//! Tests for the timeout functionality.
//!
//! This test mirrors the `timeout` test in `fdevent_test.cpp`.

use fdevent::fdevent::{Fdevent, FdeventHandler};
use mio::Interest;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use std::os::unix::io::OwnedFd;

/// A handler that records read and timeout events.
struct TimeoutHandler {
    events: Arc<Mutex<Vec<String>>>,
}

impl FdeventHandler for TimeoutHandler {
    fn on_event(&mut self, event: &mio::event::Event, _fdevent: &mut Fdevent) {
        if event.is_readable() {
            let mut events = self.events.lock().unwrap();
            events.push("read".to_string());
        }
    }

    fn on_timeout(&mut self, _fdevent: &mut Fdevent) {
        let mut events = self.events.lock().unwrap();
        events.push("timeout".to_string());
    }
}

/// Tests that both read events and timeout events are correctly triggered.
#[test]
fn timeout() {
    let mut fdevent = Fdevent::new().unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let (r, mut w) = UnixStream::pair().unwrap();
    r.set_nonblocking(true).unwrap();

    let handler = Box::new(TimeoutHandler {
        events: events.clone(),
    });
    let token = fdevent.register(OwnedFd::from(r).into(), handler, Interest::READABLE).unwrap();

    let delta = Duration::from_millis(100);
    fdevent.set_timeout(token, delta).unwrap();

    let handle = thread::spawn(move || {
        // First poll: returns when w.write happens.
        fdevent.poll(None).unwrap();

        // Wait for timeout to expire twice.
        thread::sleep(delta + delta / 2);
        fdevent.poll(None).unwrap();

        thread::sleep(delta);
        fdevent.poll(None).unwrap();

        fdevent
    });

    w.write_all(&[0]).unwrap();
    let _fdevent = handle.join().unwrap();

    let events = events.lock().unwrap();
    // Should have: 1 read, 2 timeouts
    assert_eq!(3, events.len());
    assert_eq!("read", events[0]);
    assert_eq!("timeout", events[1]);
    assert_eq!("timeout", events[2]);
}
