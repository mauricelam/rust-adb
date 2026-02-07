/*
 * Copyright (C) 2023 The Android Open Source Project
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! JDWP service implementation.
//! Ported from `original/daemon/jdwp_service.cpp`.

use adb_sockets::{Socket, SocketRegistry};
use bytes::Bytes;
use std::sync::{Arc, Mutex, OnceLock, Weak};

static JDWP_OBSERVERS: OnceLock<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>> = OnceLock::new();

/// Registers an observer to be notified of JDWP process updates.
pub fn register_jdwp_observer(observer: Box<dyn Fn() + Send + Sync>) {
    JDWP_OBSERVERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(observer);
}

/// Notifies all registered JDWP observers of an update.
pub fn notify_jdwp_update() {
    if let Some(observers_mutex) = JDWP_OBSERVERS.get() {
        let observers = observers_mutex.lock().unwrap();
        for observer in observers.iter() {
            observer();
        }
    }
}

/// Returns a string representing the current list of JDWP process PIDs.
pub fn get_jdwp_list() -> String {
    // Ported from `jdwp_process_list_msg` in `original/daemon/jdwp_service.cpp`.
    // For now, we return an empty list or a mock list.
    // In a real adbd, this would be populated by processes connecting to the JDWP socket.
    String::new()
}

/// A service that tracks JDWP processes on the device.
/// Ported from `jdwp_tracker` in `original/daemon/jdwp_service.cpp`.
pub struct JdwpTracker {
    id: u32,
    inner: Mutex<JdwpTrackerInner>,
}

struct JdwpTrackerInner {
    registry: Weak<SocketRegistry>,
    peer: Option<Weak<dyn Socket>>,
    track: bool,
}

impl JdwpTracker {
    pub fn new(id: u32, registry: Arc<SocketRegistry>, track: bool) -> Self {
        Self {
            id,
            inner: Mutex::new(JdwpTrackerInner {
                registry: Arc::downgrade(&registry),
                peer: None,
                track,
            }),
        }
    }

    /// Sends the current JDWP process list to the peer socket.
    pub fn send_jdwp_list(&self) {
        let inner = self.inner.lock().unwrap();
        if let Some(peer) = inner.peer.as_ref().and_then(|p| p.upgrade()) {
            let list = get_jdwp_list();
            if inner.track {
                let data = format!("{:04x}{}", list.len(), list);
                peer.enqueue(Bytes::from(data));
            } else {
                let data = if list.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", list)
                };
                peer.enqueue(Bytes::from(data));
                peer.close();
            }
        }
    }
}

impl Socket for JdwpTracker {
    fn id(&self) -> u32 {
        self.id
    }
    fn enqueue(&self, _data: Bytes) -> i32 {
        0
    }
    fn ready(&self) {
        self.send_jdwp_list();
    }
    fn ack(&self, _acked_bytes: Option<i32>) {}
    fn shutdown(&self) {}
    fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(peer) = inner.peer.take().and_then(|p| p.upgrade()) {
            peer.close();
        }
        if let Some(registry) = inner.registry.upgrade() {
            registry.remove(self.id);
        }
    }
    fn peer_id(&self) -> Option<u32> {
        self.inner
            .lock()
            .unwrap()
            .peer
            .as_ref()
            .and_then(|p| p.upgrade())
            .map(|p| p.id())
    }
    fn transport_id(&self) -> Option<u64> {
        None
    }
    fn set_peer(&self, peer: Arc<dyn Socket>) {
        self.inner.lock().unwrap().peer = Some(Arc::downgrade(&peer));
    }
}
