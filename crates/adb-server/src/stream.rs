use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use adb_proto::{LocalId, RemoteId};

/// Errors that can occur during stream registry operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error("stream not found")]
    NotFound,
    #[error("stream registry capacity exhausted")]
    CapacityExhausted,
}

/// A registered stream entry holding the local_id, optional remote_id, and the stream handle `H`.
#[derive(Debug)]
pub struct StreamEntry<H> {
    local_id: LocalId,
    remote_id: RwLock<Option<RemoteId>>,
    handle: H,
}

impl<H> StreamEntry<H> {
    pub fn new(local_id: LocalId, remote_id: Option<RemoteId>, handle: H) -> Self {
        Self {
            local_id,
            remote_id: RwLock::new(remote_id),
            handle,
        }
    }

    /// Returns the local stream ID (always > 0).
    #[inline]
    pub fn local_id(&self) -> LocalId {
        self.local_id
    }

    /// Returns the remote peer's stream ID, if known.
    #[inline]
    pub fn remote_id(&self) -> Option<RemoteId> {
        *self.remote_id.read().expect("lock poisoned")
    }

    /// Sets or updates the remote peer's stream ID.
    #[inline]
    pub fn set_remote_id(&self, remote_id: RemoteId) {
        *self.remote_id.write().expect("lock poisoned") = Some(remote_id);
    }

    /// Clears the remote peer's stream ID.
    #[inline]
    pub fn clear_remote_id(&self) {
        *self.remote_id.write().expect("lock poisoned") = None;
    }

    /// Returns a reference to the inner stream handle.
    #[inline]
    pub fn handle(&self) -> &H {
        &self.handle
    }
}

/// Concurrency-safe registry for multiplexed logical ADB streams over a physical transport.
///
/// Implements stream ID allocation and stream mapping matching upstream C++ ADB constraints:
/// - Local stream IDs are positive non-zero 32-bit unsigned integers (`1..=u32::MAX`).
/// - ID generation starts at 1 and wraps around to 1 (skipping 0).
/// - Remote IDs must also never be 0.
/// - Sockets can be looked up by `local_id` or `(local_id, remote_id)` where `remote_id.is_none()`
///   matches any peer ID (matching `find_local_socket(local_id, peer_id)` in `original/sockets.cpp`).
pub struct StreamRegistry<H> {
    next_id: AtomicU32,
    streams: RwLock<HashMap<LocalId, Arc<StreamEntry<H>>>>,
}

impl<H> Default for StreamRegistry<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H> StreamRegistry<H> {
    /// Creates a new empty `StreamRegistry`.
    pub fn new() -> Self {
        Self {
            next_id: AtomicU32::new(1),
            streams: RwLock::new(HashMap::new()),
        }
    }

    /// Allocates the next unique `LocalId` and registers the stream handle.
    pub fn allocate(&self, handle: H) -> Result<LocalId, StreamError> {
        self.allocate_internal(None, handle)
    }

    /// Allocates the next unique `LocalId` with a known `RemoteId` and registers the stream handle.
    pub fn allocate_with_remote(
        &self,
        remote_id: RemoteId,
        handle: H,
    ) -> Result<LocalId, StreamError> {
        self.allocate_internal(Some(remote_id), handle)
    }

    fn allocate_internal(
        &self,
        remote_id: Option<RemoteId>,
        handle: H,
    ) -> Result<LocalId, StreamError> {
        let mut map = self.streams.write().expect("lock poisoned");

        // Guard against total exhaustion (2^32 - 1 entries)
        if map.len() >= u32::MAX as usize {
            return Err(StreamError::CapacityExhausted);
        }

        // Loop until an unused ID is found (handles wraparound and avoids collisions)
        loop {
            let candidate = self.next_id.fetch_add(1, Ordering::Relaxed);
            let id = if candidate == 0 {
                // If wraparound hit 0, bump past 0 so ID is at least 1
                self.next_id.fetch_add(1, Ordering::Relaxed);
                1
            } else {
                candidate
            };

            if let Some(local_id) = LocalId::from_u32(id) {
                if let Entry::Vacant(e) = map.entry(local_id) {
                    let entry = Arc::new(StreamEntry::new(local_id, remote_id, handle));
                    e.insert(entry);
                    return Ok(local_id);
                }
            }
        }
    }

    /// Sets or updates the `remote_id` for an existing local stream.
    pub fn set_remote_id(&self, local_id: LocalId, remote_id: RemoteId) -> Result<(), StreamError> {
        let map = self.streams.read().expect("lock poisoned");
        if let Some(entry) = map.get(&local_id) {
            entry.set_remote_id(remote_id);
            Ok(())
        } else {
            Err(StreamError::NotFound)
        }
    }

    /// Looks up a stream by `local_id`.
    pub fn get(&self, local_id: LocalId) -> Option<Arc<StreamEntry<H>>> {
        let map = self.streams.read().expect("lock poisoned");
        map.get(&local_id).cloned()
    }

    /// Looks up a stream by `local_id` and optional `remote_id`.
    ///
    /// Following C++ ADB `find_local_socket(local_id, peer_id)`:
    /// - If `remote_id.is_none()`, checks only `local_id`.
    /// - If `remote_id.is_some()`, checks that `entry.remote_id() == remote_id`.
    pub fn find(
        &self,
        local_id: LocalId,
        remote_id: Option<RemoteId>,
    ) -> Option<Arc<StreamEntry<H>>> {
        let map = self.streams.read().expect("lock poisoned");
        let entry = map.get(&local_id)?;
        match remote_id {
            None => Some(Arc::clone(entry)),
            Some(rid) if entry.remote_id() == Some(rid) => Some(Arc::clone(entry)),
            _ => None,
        }
    }

    /// Finds a stream by its associated `remote_id`.
    pub fn find_by_remote(&self, remote_id: RemoteId) -> Option<Arc<StreamEntry<H>>> {
        let map = self.streams.read().expect("lock poisoned");
        map.values()
            .find(|entry| entry.remote_id() == Some(remote_id))
            .cloned()
    }

    /// Removes and reclaims a stream by its `local_id`.
    pub fn remove(&self, local_id: LocalId) -> Option<Arc<StreamEntry<H>>> {
        let mut map = self.streams.write().expect("lock poisoned");
        map.remove(&local_id)
    }

    /// Returns the number of currently active streams.
    pub fn count(&self) -> usize {
        self.streams.read().expect("lock poisoned").len()
    }

    /// Returns true if there are no active streams.
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Clears all streams from the registry and returns them.
    pub fn clear(&self) -> Vec<Arc<StreamEntry<H>>> {
        let mut map = self.streams.write().expect("lock poisoned");
        map.drain().map(|(_, v)| v).collect()
    }
}
