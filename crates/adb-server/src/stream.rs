use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::oneshot;

use adb_proto::{LocalId, RemoteId};

use crate::router::RouterError;

/// Errors that can occur during stream registry operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    #[error("stream not found")]
    NotFound,
    #[error("stream registry capacity exhausted")]
    CapacityExhausted,
}

/// A request to write payload data to a logical stream.
pub struct StreamWriteRequest {
    pub local_id: LocalId,
    pub payload: Vec<u8>,
    pub response_tx: oneshot::Sender<Result<(), RouterError>>,
}

impl std::fmt::Debug for StreamWriteRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamWriteRequest")
            .field("local_id", &self.local_id)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}

/// State of inbound data receiving on an open logical stream.
#[derive(Debug, Default)]
pub struct InboundState {
    /// Whether an inbound A_WRTE packet has arrived and is awaiting an A_OKAY acknowledgment.
    pub awaiting_ack: bool,
}

/// State of outbound data transmission on an open logical stream.
#[derive(Default)]
pub struct OutboundState {
    /// In-flight write response sender awaiting an A_OKAY acknowledgment from the peer.
    pub in_flight: Option<oneshot::Sender<Result<(), RouterError>>>,
    /// Queue of pending outbound writes awaiting peer A_OKAY.
    pub queued: VecDeque<StreamWriteRequest>,
}

impl std::fmt::Debug for OutboundState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundState")
            .field("has_in_flight", &self.in_flight.is_some())
            .field("queued_len", &self.queued.len())
            .finish()
    }
}

/// Explicit lifecycle state of a logical stream.
pub enum StreamState<H> {
    /// Stream is being opened (e.g. host sent OPEN or awaiting remote peer OKAY).
    Opening { handle: H },
    /// Stream is fully established with known peer RemoteId.
    Open {
        remote_id: RemoteId,
        handle: H,
        inbound: InboundState,
        outbound: OutboundState,
    },
    /// Stream is closing or closed.
    Closing,
}

impl<H> std::fmt::Debug for StreamState<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Opening { .. } => write!(f, "StreamState::Opening"),
            Self::Open {
                remote_id,
                inbound,
                outbound,
                ..
            } => f
                .debug_struct("StreamState::Open")
                .field("remote_id", remote_id)
                .field("inbound", inbound)
                .field("outbound", outbound)
                .finish(),
            Self::Closing => write!(f, "StreamState::Closing"),
        }
    }
}

/// A registered stream entry holding the local_id and the explicit `StreamState<H>`.
pub struct StreamEntry<H> {
    local_id: LocalId,
    state: RwLock<StreamState<H>>,
}

impl<H> std::fmt::Debug for StreamEntry<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamEntry")
            .field("local_id", &self.local_id)
            .field("state", &*self.state.read().expect("lock poisoned"))
            .finish()
    }
}

impl<H: Clone> StreamEntry<H> {
    /// Creates a new `StreamEntry` in Opening or Open state depending on `remote_id`.
    pub fn new(local_id: LocalId, remote_id: Option<RemoteId>, handle: H) -> Self {
        let state = match remote_id {
            Some(rid) => StreamState::Open {
                remote_id: rid,
                handle,
                inbound: InboundState::default(),
                outbound: OutboundState::default(),
            },
            None => StreamState::Opening { handle },
        };
        Self {
            local_id,
            state: RwLock::new(state),
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
        match &*self.state.read().expect("lock poisoned") {
            StreamState::Open { remote_id, .. } => Some(*remote_id),
            _ => None,
        }
    }

    /// Sets or updates the remote peer's stream ID, transitioning `Opening` to `Open`.
    pub fn set_remote_id(&self, remote_id: RemoteId) {
        let mut state_guard = self.state.write().expect("lock poisoned");
        let current = std::mem::replace(&mut *state_guard, StreamState::Closing);
        *state_guard = match current {
            StreamState::Opening { handle } => StreamState::Open {
                remote_id,
                handle,
                inbound: InboundState::default(),
                outbound: OutboundState::default(),
            },
            StreamState::Open {
                handle,
                inbound,
                outbound,
                ..
            } => StreamState::Open {
                remote_id,
                handle,
                inbound,
                outbound,
            },
            StreamState::Closing => StreamState::Closing,
        };
    }

    /// Returns a clone of the inner stream handle, if not closed.
    pub fn handle(&self) -> Option<H> {
        match &*self.state.read().expect("lock poisoned") {
            StreamState::Opening { handle } => Some(handle.clone()),
            StreamState::Open { handle, .. } => Some(handle.clone()),
            StreamState::Closing => None,
        }
    }

    /// Executes an operation with a reference to the stream state.
    pub fn with_state<R>(&self, f: impl FnOnce(&StreamState<H>) -> R) -> R {
        let guard = self.state.read().expect("lock poisoned");
        f(&*guard)
    }

    /// Executes an operation with a mutable reference to the stream state.
    pub fn with_state_mut<R>(&self, f: impl FnOnce(&mut StreamState<H>) -> R) -> R {
        let mut guard = self.state.write().expect("lock poisoned");
        f(&mut *guard)
    }

    /// Closes this stream entry and cleans up any pending outbound write requests.
    pub fn close(&self) {
        let mut guard = self.state.write().expect("lock poisoned");
        let prev = std::mem::replace(&mut *guard, StreamState::Closing);
        if let StreamState::Open { mut outbound, .. } = prev {
            if let Some(resp_tx) = outbound.in_flight.take() {
                let _ = resp_tx.send(Err(RouterError::Stream("connection closed".to_string())));
            }
            while let Some(req) = outbound.queued.pop_front() {
                let _ = req
                    .response_tx
                    .send(Err(RouterError::Stream("connection closed".to_string())));
            }
        }
    }
}

/// Concurrency-safe registry for multiplexed logical ADB streams over a physical transport.
pub struct StreamRegistry<H> {
    next_id: AtomicU32,
    streams: RwLock<HashMap<LocalId, Arc<StreamEntry<H>>>>,
}

impl<H> Default for StreamRegistry<H> {
    fn default() -> Self {
        Self {
            next_id: AtomicU32::new(1),
            streams: RwLock::new(HashMap::new()),
        }
    }
}

impl<H> StreamRegistry<H> {
    /// Creates a new empty `StreamRegistry`.
    pub fn new() -> Self {
        Self::default()
    }
}

impl<H: Clone> StreamRegistry<H> {
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
        let entry = map.remove(&local_id);
        if let Some(ref e) = entry {
            e.close();
        }
        entry
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
        let drained: Vec<_> = map.drain().map(|(_, v)| v).collect();
        drained
    }
}
