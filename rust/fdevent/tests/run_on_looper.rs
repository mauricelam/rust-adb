//! Tests for the `run_on_looper` functionality.
//!
//! These tests mirror the `run_on_looper_thread_queued` and
//! `run_on_looper_thread_reentrant` tests in `fdevent_test.cpp`.

use fdevent::fdevent::Fdevent;
use std::sync::{Arc, Mutex};
use std::thread;

/// Tests that multiple functions can be queued and executed on the looper thread.
#[test]
fn run_on_looper_thread_queued() {
    let mut fdevent = Fdevent::new().unwrap();
    let vec = Arc::new(Mutex::new(Vec::new()));

    let vec_clone = vec.clone();
    let handle = thread::spawn(move || {
        for i in 0..1000 {
            let vec_inner = vec_clone.clone();
            fdevent.run_on_looper(move |_fdevent| {
                let mut v = vec_inner.lock().unwrap();
                v.push(i);
            });
        }
        fdevent.poll(None).unwrap();
        fdevent
    });

    let _fdevent = handle.join().unwrap();
    let v = vec.lock().unwrap();
    assert_eq!(1000, v.len());
    for i in 0..1000 {
        assert_eq!(i, v[i]);
    }
}

/// Tests that `run_on_looper` can be called from within a function already
/// running on the looper thread.
#[test]
fn run_on_looper_thread_reentrant() {
    let mut fdevent = Fdevent::new().unwrap();
    let b = Arc::new(Mutex::new(false));
    let b_clone = b.clone();
    let handle = fdevent.get_handle();

    fdevent.run_on_looper(move |_fdevent| {
        let b_clone2 = b_clone.clone();
        handle.run_on_looper(move |_fdevent| {
            let mut b = b_clone2.lock().unwrap();
            *b = true;
        });
    });

    fdevent.poll(None).unwrap();
    let b = b.lock().unwrap();
    assert_eq!(true, *b);
}
