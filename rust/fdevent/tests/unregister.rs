//! Tests for unregistering handlers.
//!
//! This test mirrors the `unregister_with_pending_event` test in `fdevent_test.cpp`.

use fdevent::fdevent::{Fdevent, FdeventHandler};
use mio::{Interest, Token};
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct TestHandler {
    token_to_unregister: Arc<Mutex<Option<Token>>>,
    hit_count: Arc<Mutex<usize>>,
}

impl FdeventHandler for TestHandler {
    fn on_event(&mut self, _event: &mio::event::Event, fdevent: &mut Fdevent) {
        let mut count = self.hit_count.lock().unwrap();
        *count += 1;
        let to_unreg = {
            let mut lock = self.token_to_unregister.lock().unwrap();
            lock.take()
        };
        if let Some(other) = to_unreg {
            fdevent.unregister(other).unwrap();
        }
    }
    fn on_timeout(&mut self, _fdevent: &mut Fdevent) {}
}

#[test]
fn unregister_with_pending_event() {
    let mut fdevent = Fdevent::new().unwrap();
    let (r1, mut w1) = UnixStream::pair().unwrap();
    let (r2, mut w2) = UnixStream::pair().unwrap();
    r1.set_nonblocking(true).unwrap();
    r2.set_nonblocking(true).unwrap();

    let hit1 = Arc::new(Mutex::new(0));
    let hit2 = Arc::new(Mutex::new(0));
    let unreg = Arc::new(Mutex::new(None));

    let h1 = Box::new(TestHandler {
        token_to_unregister: unreg.clone(),
        hit_count: hit1.clone(),
    });
    let h2 = Box::new(TestHandler {
        token_to_unregister: Arc::new(Mutex::new(None)),
        hit_count: hit2.clone(),
    });

    let _t1 = fdevent.register(r1.into(), h1, Interest::READABLE).unwrap();
    let t2 = fdevent.register(r2.into(), h2, Interest::READABLE).unwrap();

    // Handler 1 will unregister Handler 2
    *unreg.lock().unwrap() = Some(t2);

    // Make both readable
    w1.write_all(b"a").unwrap();
    w2.write_all(b"a").unwrap();

    // Poll once.
    fdevent.poll(Some(Duration::from_millis(100))).unwrap();

    let c1 = *hit1.lock().unwrap();
    let c2 = *hit2.lock().unwrap();

    assert!(c1 + c2 <= 2);
}
