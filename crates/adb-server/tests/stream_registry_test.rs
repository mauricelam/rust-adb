use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use adb_proto::{LocalId, RemoteId};
use adb_server::StreamRegistry;

#[derive(Debug, Clone, PartialEq, Eq)]
struct MockStream {
    name: String,
}

#[test]
fn test_basic_allocation_and_lookup() {
    let registry = StreamRegistry::<MockStream>::new();
    assert_eq!(registry.count(), 0);
    assert!(registry.is_empty());

    let id1 = registry
        .allocate(MockStream {
            name: "stream1".into(),
        })
        .expect("allocate stream 1");
    // Local IDs start at 1, matching C++ ADB
    assert_eq!(id1.get(), 1);
    assert_eq!(registry.count(), 1);
    assert!(!registry.is_empty());

    let entry1 = registry.get(id1).expect("stream 1 should exist");
    assert_eq!(entry1.local_id(), id1);
    assert_eq!(entry1.remote_id(), None);
    assert_eq!(entry1.handle().name, "stream1");

    let rid42 = RemoteId::from_u32(42).unwrap();
    let id2 = registry
        .allocate_with_remote(
            rid42,
            MockStream {
                name: "stream2".into(),
            },
        )
        .expect("allocate stream 2");
    assert_eq!(id2.get(), 2);
    assert_eq!(registry.count(), 2);

    let entry2 = registry.get(id2).expect("stream 2 should exist");
    assert_eq!(entry2.local_id(), id2);
    assert_eq!(entry2.remote_id(), Some(rid42));
    assert_eq!(entry2.handle().name, "stream2");
}

#[test]
fn test_remote_id_update_and_find() {
    let registry = StreamRegistry::<MockStream>::new();
    let local_id = registry
        .allocate(MockStream {
            name: "test".into(),
        })
        .expect("allocate");

    assert_eq!(registry.get(local_id).unwrap().remote_id(), None);

    // find with None matches regardless of remote_id (C++ find_local_socket semantics)
    assert!(registry.find(local_id, None).is_some());

    let rid100 = RemoteId::from_u32(100).unwrap();
    let rid101 = RemoteId::from_u32(101).unwrap();
    // find with specific non-matching remote_id fails
    assert!(registry.find(local_id, Some(rid100)).is_none());

    // Setting valid remote ID
    registry
        .set_remote_id(local_id, rid100)
        .expect("set remote id");
    assert_eq!(registry.get(local_id).unwrap().remote_id(), Some(rid100));

    // find with None still matches
    assert!(registry.find(local_id, None).is_some());
    // find with matching remote_id matches
    assert!(registry.find(local_id, Some(rid100)).is_some());
    // find with different remote_id fails
    assert!(registry.find(local_id, Some(rid101)).is_none());

    // find_by_remote
    let by_remote = registry.find_by_remote(rid100).expect("find by remote");
    assert_eq!(by_remote.local_id(), local_id);
    let rid999 = RemoteId::from_u32(999).unwrap();
    assert!(registry.find_by_remote(rid999).is_none());
}

#[test]
fn test_removal_and_reclamation() {
    let registry = StreamRegistry::<MockStream>::new();
    let id1 = registry.allocate(MockStream { name: "s1".into() }).unwrap();
    let id2 = registry.allocate(MockStream { name: "s2".into() }).unwrap();

    assert_eq!(registry.count(), 2);

    let removed = registry.remove(id1).expect("remove id1");
    assert_eq!(removed.local_id(), id1);
    assert_eq!(removed.handle().name, "s1");
    assert_eq!(registry.count(), 1);
    assert!(registry.get(id1).is_none());

    // Removing again returns None
    assert!(registry.remove(id1).is_none());

    // Clear removes all remaining
    let cleared = registry.clear();
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].local_id(), id2);
    assert_eq!(registry.count(), 0);
    assert!(registry.is_empty());
}

#[test]
fn test_id_bounds_and_zero_prohibition() {
    // In ADB, 0 is prohibited/unrepresentable for LocalId and RemoteId
    assert!(LocalId::from_u32(0).is_none());
    assert!(RemoteId::from_u32(0).is_none());
    assert!("0".parse::<LocalId>().is_err());
    assert!("0".parse::<RemoteId>().is_err());
}

#[test]
fn test_parallel_stream_id_generation_10000() {
    let registry = Arc::new(StreamRegistry::<MockStream>::new());
    let num_threads = 10;
    let allocations_per_thread = 1_000; // 10 * 1000 = 10,000 total

    let mut handles = Vec::new();
    for t in 0..num_threads {
        let reg = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            let mut allocated = Vec::with_capacity(allocations_per_thread);
            for i in 0..allocations_per_thread {
                let id = reg
                    .allocate(MockStream {
                        name: format!("t{t}-s{i}"),
                    })
                    .expect("allocation should succeed");
                allocated.push(id);
            }
            allocated
        }));
    }

    let mut all_ids = HashSet::new();
    for handle in handles {
        let thread_ids = handle.join().expect("thread should join");
        for id in thread_ids {
            assert!(all_ids.insert(id), "Collision detected for stream ID {id}");
        }
    }

    assert_eq!(all_ids.len(), num_threads * allocations_per_thread);
    assert_eq!(registry.count(), 10_000);
}

#[test]
fn test_stream_reclamation_rapid_creation_and_teardown() {
    let registry = Arc::new(StreamRegistry::<MockStream>::new());
    let num_threads = 8;
    let iterations = 2_500; // 20,000 total rapid lifecycle operations

    let mut handles = Vec::new();
    for t in 0..num_threads {
        let reg = Arc::clone(&registry);
        handles.push(thread::spawn(move || {
            for i in 0..iterations {
                let id = reg
                    .allocate(MockStream {
                        name: format!("t{t}-s{i}"),
                    })
                    .expect("allocation should succeed");

                let entry = reg.get(id).expect("stream must exist");
                assert_eq!(entry.local_id(), id);

                let removed = reg.remove(id).expect("stream removal must succeed");
                assert_eq!(removed.local_id(), id);
                assert!(reg.get(id).is_none());
            }
        }));
    }

    for handle in handles {
        handle.join().expect("thread join");
    }

    assert_eq!(registry.count(), 0);
    assert!(registry.is_empty());
}

#[test]
#[ignore]
fn test_stress_concurrent_allocation_and_reclamation_long() {
    let registry = Arc::new(StreamRegistry::<MockStream>::new());
    let num_workers = 16;
    let running = Arc::new(AtomicBool::new(true));

    let mut handles = Vec::new();
    for t in 0..num_workers {
        let reg = Arc::clone(&registry);
        let run = Arc::clone(&running);
        handles.push(thread::spawn(move || {
            let mut active = Vec::new();
            let mut iter = 0;
            while run.load(Ordering::Relaxed) && iter < 10_000 {
                iter += 1;
                if active.len() < 50 {
                    let id = reg
                        .allocate(MockStream {
                            name: format!("w{t}-{iter}"),
                        })
                        .unwrap();
                    active.push(id);
                } else {
                    let id = active.pop().unwrap();
                    let removed = reg.remove(id).expect("should remove active stream");
                    assert_eq!(removed.local_id(), id);
                }
            }
            // Cleanup
            for id in active {
                reg.remove(id);
            }
        }));
    }

    for handle in handles {
        handle.join().expect("worker join");
    }

    assert_eq!(registry.count(), 0);
}
