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

/// Represents a process that can be tracked by JDWP services.
#[derive(Clone, Debug, Default)]
pub struct ProcessInfo {
    pub pid: u64,
    pub debuggable: bool,
    pub profileable: bool,
    pub architecture: String,
    pub user_id: Option<u64>,
    pub process_name: Option<String>,
    pub package_names: Vec<String>,
    pub waiting_for_debugger: Option<bool>,
    pub uid: Option<u64>,
}

/// The kind of tracker service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerKind {
    /// Tracks only debuggable processes (text format).
    Jdwp,
    /// Tracks debuggable and profileable processes (protobuf format).
    App,
}

// Protobuf messages for "track-app" service.
// Manually defined to match original/proto/app_processes.proto.
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ProcessEntry {
    #[prost(int64, tag = "1")]
    pub pid: i64,
    #[prost(bool, tag = "2")]
    pub debuggable: bool,
    #[prost(bool, tag = "3")]
    pub profileable: bool,
    #[prost(string, tag = "4")]
    pub architecture: ::prost::alloc::string::String,
    #[prost(int64, optional, tag = "5")]
    pub user_id: ::core::option::Option<i64>,
    #[prost(string, optional, tag = "6")]
    pub process_name: ::core::option::Option<::prost::alloc::string::String>,
    #[prost(string, repeated, tag = "7")]
    pub package_names: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    #[prost(bool, optional, tag = "8")]
    pub waiting_for_debugger: ::core::option::Option<bool>,
    #[prost(int64, optional, tag = "9")]
    pub uid: ::core::option::Option<i64>,
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AppProcesses {
    #[prost(message, repeated, tag = "1")]
    pub process: ::prost::alloc::vec::Vec<ProcessEntry>,
}

static MOCK_PROCESSES: OnceLock<Mutex<Vec<ProcessInfo>>> = OnceLock::new();

/// Sets a mock list of processes for testing.
pub fn set_mock_processes(processes: Vec<ProcessInfo>) {
    *MOCK_PROCESSES.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap() = processes;
    notify_jdwp_update();
}

fn get_processes() -> Vec<ProcessInfo> {
    MOCK_PROCESSES.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clone()
}

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
    // Ported from `jdwp_process_list` in `original/daemon/jdwp_service.cpp`.
    let mut list = String::new();
    for proc in get_processes() {
        if proc.debuggable {
            list.push_str(&format!("{}\n", proc.pid));
        }
    }
    list
}

/// Returns the serialized process list for "track-app" service.
pub fn get_app_process_list() -> Vec<u8> {
    use prost::Message;
    let mut app_processes = AppProcesses::default();
    for proc in get_processes() {
        if proc.debuggable || proc.profileable {
            app_processes.process.push(ProcessEntry {
                pid: proc.pid as i64,
                debuggable: proc.debuggable,
                profileable: proc.profileable,
                architecture: proc.architecture.clone(),
                user_id: proc.user_id.map(|id| id as i64),
                process_name: proc.process_name.clone(),
                package_names: proc.package_names.clone(),
                waiting_for_debugger: proc.waiting_for_debugger,
                uid: proc.uid.map(|id| id as i64),
            });
        }
    }
    app_processes.encode_to_vec()
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
    kind: TrackerKind,
    track: bool,
}

impl JdwpTracker {
    pub fn new(id: u32, registry: Arc<SocketRegistry>, kind: TrackerKind, track: bool) -> Self {
        Self {
            id,
            inner: Mutex::new(JdwpTrackerInner {
                registry: Arc::downgrade(&registry),
                peer: None,
                kind,
                track,
            }),
        }
    }

    /// Sends the current process list to the peer socket.
    pub fn send_process_list(&self) {
        let inner = self.inner.lock().unwrap();
        if let Some(peer) = inner.peer.as_ref().and_then(|p| p.upgrade()) {
            match inner.kind {
                TrackerKind::Jdwp => {
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
                TrackerKind::App => {
                    let list = get_app_process_list();
                    if inner.track {
                        let mut data = Vec::with_capacity(list.len() + 4);
                        data.extend_from_slice(format!("{:04x}", list.len()).as_bytes());
                        data.extend_from_slice(&list);
                        peer.enqueue(Bytes::from(data));
                    } else {
                        peer.enqueue(Bytes::from(list));
                        peer.close();
                    }
                }
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
        self.send_process_list();
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

#[cfg(test)]
mod tests {
    use super::*;
    use adb_sockets::SocketRegistry;

    struct MockSocket {
        id: u32,
        data: Mutex<Vec<Bytes>>,
        closed: Mutex<bool>,
    }

    impl Socket for MockSocket {
        fn id(&self) -> u32 { self.id }
        fn enqueue(&self, data: Bytes) -> i32 {
            self.data.lock().unwrap().push(data);
            0
        }
        fn ready(&self) {}
        fn ack(&self, _acked_bytes: Option<i32>) {}
        fn shutdown(&self) {}
        fn close(&self) {
            *self.closed.lock().unwrap() = true;
        }
        fn peer_id(&self) -> Option<u32> { None }
        fn transport_id(&self) -> Option<u64> { None }
        fn set_peer(&self, _peer: Arc<dyn Socket>) {}
    }

    #[test]
    fn test_jdwp_tracker_jdwp_kind() {
        let registry = Arc::new(SocketRegistry::new());
        let tracker = Arc::new(JdwpTracker::new(1, registry.clone(), TrackerKind::Jdwp, true));
        let mock_peer = Arc::new(MockSocket {
            id: 2,
            data: Mutex::new(Vec::new()),
            closed: Mutex::new(false),
        });

        tracker.set_peer(mock_peer.clone());

        set_mock_processes(vec![
            ProcessInfo { pid: 123, debuggable: true, ..Default::default() },
            ProcessInfo { pid: 456, debuggable: false, ..Default::default() },
        ]);

        tracker.send_process_list();

        let data = mock_peer.data.lock().unwrap();
        assert_eq!(data.len(), 1);
        let s = String::from_utf8_lossy(&data[0]);
        // 0004123\n
        assert_eq!(s, "0004123\n");
    }

    #[test]
    fn test_jdwp_tracker_app_kind() {
        let registry = Arc::new(SocketRegistry::new());
        let tracker = Arc::new(JdwpTracker::new(1, registry.clone(), TrackerKind::App, true));
        let mock_peer = Arc::new(MockSocket {
            id: 2,
            data: Mutex::new(Vec::new()),
            closed: Mutex::new(false),
        });

        tracker.set_peer(mock_peer.clone());

        set_mock_processes(vec![
            ProcessInfo {
                pid: 123,
                debuggable: true,
                profileable: false,
                architecture: "arm64".to_string(),
                process_name: Some("test.app".to_string()),
                ..Default::default()
            },
        ]);

        tracker.send_process_list();

        let data = mock_peer.data.lock().unwrap();
        assert_eq!(data.len(), 1);
        let len_str = String::from_utf8_lossy(&data[0][..4]);
        let len = usize::from_str_radix(&len_str, 16).unwrap();
        assert_eq!(len, data[0].len() - 4);

        use prost::Message;
        let app_processes = AppProcesses::decode(&data[0][4..]).unwrap();
        assert_eq!(app_processes.process.len(), 1);
        assert_eq!(app_processes.process[0].pid, 123);
        assert_eq!(app_processes.process[0].process_name.as_deref(), Some("test.app"));
    }
}
