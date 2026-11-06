use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use adb_proto::{AckBytes, Command, LocalId, RemoteId};
use adb_transport::{Packet, Transport, TransportError};

use crate::stream::{StreamRegistry, StreamState, StreamWriteRequest};

/// Handle passed with incoming data for acknowledging receipt via A_OKAY.
#[derive(Clone, Debug)]
pub struct StreamAcker {
    local_id: LocalId,
    cmd_tx: mpsc::Sender<RouterCommand>,
}

impl StreamAcker {
    /// Returns the local stream ID.
    pub fn local_id(&self) -> LocalId {
        self.local_id
    }

    /// Sends an A_OKAY acknowledgment back to the remote peer.
    pub async fn send_okay(&self) -> Result<(), RouterError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.cmd_tx
            .send(RouterCommand::AckInbound(self.local_id, resp_tx))
            .await
            .map_err(|_| RouterError::Stream("router command channel closed".to_string()))?;

        resp_rx
            .await
            .map_err(|_| RouterError::Stream("router response channel dropped".to_string()))?
    }
}

/// Messages dispatched to active stream channels.
#[derive(Debug, Clone)]
pub enum StreamMessage {
    /// Payload data received via an `A_WRTE` packet, along with an acker to send A_OKAY back to peer.
    Data(Vec<u8>, StreamAcker),
    /// Stream connection confirmation or flow control update via an `A_OKAY` packet.
    Ready {
        remote_id: RemoteId,
        ack_bytes: Option<AckBytes>,
    },
    /// Stream closure via an `A_CLSE` packet or transport termination.
    Close,
}

pub type StreamSender = mpsc::UnboundedSender<StreamMessage>;
pub type StreamReceiver = mpsc::UnboundedReceiver<StreamMessage>;

/// Incoming service request from an `A_OPEN` packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    /// Peer's stream ID.
    pub remote_id: RemoteId,
    /// Null-stripped service destination string (e.g. "shell:ls").
    pub destination: String,
    /// Send buffer bytes for delayed ack (if any).
    pub send_bytes: u32,
}

/// Decision for an incoming `A_OPEN` request.
#[derive(Debug)]
pub enum OpenDecision {
    /// Accept connection and provide stream message sender.
    Accept(StreamSender),
    /// Reject connection, prompting `A_CLSE` reply.
    Reject,
}

pub type OpenHandler = Box<dyn FnMut(OpenRequest) -> OpenDecision + Send + 'static>;

/// Errors that can occur during packet router operations.
#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("stream error: {0}")]
    Stream(String),
    #[error("protocol violation: {0}")]
    ProtocolViolation(String),
}

enum RouterCommand {
    Write(StreamWriteRequest),
    AckInbound(LocalId, oneshot::Sender<Result<(), RouterError>>),
}

/// Handle allowing asynchronous writing to a multiplexed stream with OKAY flow control backpressure.
#[derive(Clone)]
pub struct StreamWriter {
    local_id: LocalId,
    cmd_tx: mpsc::Sender<RouterCommand>,
}

impl StreamWriter {
    /// Returns the local stream ID.
    pub fn local_id(&self) -> LocalId {
        self.local_id
    }

    /// Asynchronously writes payload to the remote peer, awaiting an A_OKAY acknowledgment before completing.
    pub async fn write(&self, data: Vec<u8>) -> Result<(), RouterError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        let req = StreamWriteRequest {
            local_id: self.local_id,
            payload: data,
            response_tx: resp_tx,
        };

        self.cmd_tx
            .send(RouterCommand::Write(req))
            .await
            .map_err(|_| RouterError::Stream("router command channel closed".to_string()))?;

        resp_rx
            .await
            .map_err(|_| RouterError::Stream("router response channel dropped".to_string()))?
    }
}

/// Main async packet dispatch loop routing `OPEN`, `WRTE`, `OKAY`, and `CLSE` packets.
pub struct PacketRouter<T: Transport> {
    transport: T,
    registry: Arc<StreamRegistry<StreamSender>>,
    open_handler: Option<OpenHandler>,
    skip_checksum: bool,
    cmd_tx: mpsc::Sender<RouterCommand>,
    cmd_rx: mpsc::Receiver<RouterCommand>,
}

impl<T: Transport> PacketRouter<T> {
    /// Creates a new `PacketRouter` with the provided transport and stream registry.
    pub fn new(transport: T, registry: Arc<StreamRegistry<StreamSender>>) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        Self {
            transport,
            registry,
            open_handler: None,
            skip_checksum: true,
            cmd_tx,
            cmd_rx,
        }
    }

    /// Creates a `StreamWriter` for the given `local_id`.
    pub fn create_stream_writer(&self, local_id: LocalId) -> Result<StreamWriter, RouterError> {
        if self.registry.get(local_id).is_none() {
            return Err(RouterError::Stream(format!("stream {local_id} not found")));
        }
        Ok(StreamWriter {
            local_id,
            cmd_tx: self.cmd_tx.clone(),
        })
    }

    /// Sends an `A_OKAY` packet acknowledging received data on the given stream, clearing inbound write-in-flight state.
    pub async fn send_okay(&mut self, local_id: LocalId) -> Result<(), RouterError> {
        let entry = self
            .registry
            .get(local_id)
            .ok_or_else(|| RouterError::Stream(format!("stream {local_id} not found")))?;

        let remote_id = entry.with_state_mut(|state| match state {
            StreamState::Open {
                remote_id, inbound, ..
            } => {
                inbound.awaiting_ack = false;
                Ok(*remote_id)
            }
            StreamState::Opening { .. } => Err(RouterError::Stream(
                "remote_id not yet established".to_string(),
            )),
            StreamState::Closing => Err(RouterError::Stream(format!("stream {local_id} closed"))),
        })?;

        let okay_packet = Packet::new(
            Command::Okay,
            local_id.get(),
            remote_id.get(),
            Vec::new(),
            self.skip_checksum,
        );
        self.transport.write_packet(&okay_packet).await?;
        Ok(())
    }

    /// Sets whether packets sent over the transport should skip payload checksums.
    pub fn set_skip_checksum(&mut self, skip_checksum: bool) {
        self.skip_checksum = skip_checksum;
    }

    /// Registers a handler for incoming `A_OPEN` requests.
    pub fn with_open_handler<F>(mut self, handler: F) -> Self
    where
        F: FnMut(OpenRequest) -> OpenDecision + Send + 'static,
    {
        self.open_handler = Some(Box::new(handler));
        self
    }

    /// Runs the main packet routing dispatch loop until transport EOF or error.
    ///
    /// Reads packets from the underlying transport and routes:
    /// - `A_OPEN`: delegates to `open_handler` if present; if accepted, allocates a local ID and
    ///   sends `A_OKAY`; otherwise replies with `A_CLSE(0, remote_id)`.
    /// - `A_WRTE`: routes payload to target stream `local_id = arg1`, validating `remote_id = arg0`.
    ///   If unmapped/closed, replies with `A_CLSE(arg1, arg0)` without crashing.
    /// - `A_OKAY`: updates peer `remote_id = arg0` for `local_id = arg1` and routes `Ready`.
    ///   If stream is unmapped, replies with `A_CLSE(arg1, arg0)`.
    /// - `A_CLSE`: notifies matching stream of closure and removes it from registry.
    ///   Unmapped CLSE packets are ignored per protocol parity.
    ///
    /// On loop exit (EOF or transport error), all registered streams are sent `Close` and cleared.
    pub async fn run(mut self) -> Result<(), RouterError> {
        let result = self.run_loop().await;

        // Cleanup: close all active streams in registry on exit and notify handles
        let active_streams = self.registry.clear();
        for entry in active_streams {
            if let Some(h) = entry.handle() {
                let _ = h.send(StreamMessage::Close);
            }
        }

        result
    }

    async fn run_loop(&mut self) -> Result<(), RouterError> {
        loop {
            tokio::select! {
                read_res = self.transport.read_packet() => {
                    let packet = match read_res {
                        Ok(pkt) => pkt,
                        Err(TransportError::Io(err))
                            if err.kind() == std::io::ErrorKind::UnexpectedEof
                                || err.kind() == std::io::ErrorKind::ConnectionReset
                                || err.kind() == std::io::ErrorKind::BrokenPipe =>
                        {
                            break Ok(());
                        }
                        Err(err) => return Err(RouterError::Transport(err)),
                    };

                    match packet.header.command {
                        Command::Open => {
                            self.handle_open(packet).await?;
                        }
                        Command::Wrte => {
                            self.handle_wrte(packet).await?;
                        }
                        Command::Okay => {
                            self.handle_okay(packet).await?;
                        }
                        Command::Clse => {
                            self.handle_clse(packet).await;
                        }
                        _ => {
                            // Ignore non-stream multiplexing packets (e.g. unexpected commands)
                        }
                    }
                }
                cmd_opt = self.cmd_rx.recv() => {
                    match cmd_opt {
                        Some(RouterCommand::Write(req)) => {
                            self.handle_write_request(req).await?;
                        }
                        Some(RouterCommand::AckInbound(local_id, resp_tx)) => {
                            let res = self.send_okay(local_id).await;
                            let _ = resp_tx.send(res);
                        }
                        None => {
                            // All command handles dropped
                        }
                    }
                }
            }
        }
    }

    async fn handle_write_request(&mut self, req: StreamWriteRequest) -> Result<(), RouterError> {
        let local_id = req.local_id;
        let entry = match self.registry.get(local_id) {
            Some(e) => e,
            None => {
                let _ = req
                    .response_tx
                    .send(Err(RouterError::Stream("stream closed".to_string())));
                return Ok(());
            }
        };

        let to_send = entry.with_state_mut(|state| match state {
            StreamState::Opening { .. } => {
                let _ = req.response_tx.send(Err(RouterError::Stream(
                    "remote_id not yet established".to_string(),
                )));
                None
            }
            StreamState::Open {
                remote_id,
                outbound,
                ..
            } => {
                if outbound.in_flight.is_some() {
                    outbound.queued.push_back(req);
                    None
                } else {
                    Some((*remote_id, req))
                }
            }
            StreamState::Closing => {
                let _ = req
                    .response_tx
                    .send(Err(RouterError::Stream("stream closed".to_string())));
                None
            }
        });

        if let Some((remote_id, req)) = to_send {
            self.send_stream_write(local_id, remote_id, req).await?;
        }

        Ok(())
    }

    async fn send_stream_write(
        &mut self,
        local_id: LocalId,
        remote_id: RemoteId,
        req: StreamWriteRequest,
    ) -> Result<(), RouterError> {
        let wrte_packet = Packet::new(
            Command::Wrte,
            local_id.get(),
            remote_id.get(),
            req.payload,
            self.skip_checksum,
        );

        if let Err(err) = self.transport.write_packet(&wrte_packet).await {
            let _ = req.response_tx.send(Err(RouterError::Transport(err)));
            return Ok(());
        }

        if let Some(entry) = self.registry.get(local_id) {
            entry.with_state_mut(|state| {
                if let StreamState::Open { outbound, .. } = state {
                    outbound.in_flight = Some(req.response_tx);
                } else {
                    let _ = req
                        .response_tx
                        .send(Err(RouterError::Stream("stream closed".to_string())));
                }
            });
        }

        Ok(())
    }

    async fn handle_open(&mut self, packet: Packet) -> Result<(), RouterError> {
        let Some(remote_id) = RemoteId::from_u32(packet.header.arg0) else {
            return Ok(());
        };
        let send_bytes = packet.header.arg1;

        let destination = {
            let mut payload = packet.payload;
            while payload.last() == Some(&0) {
                payload.pop();
            }
            String::from_utf8_lossy(&payload).into_owned()
        };

        let req = OpenRequest {
            remote_id,
            destination,
            send_bytes,
        };

        let decision = if let Some(ref mut handler) = self.open_handler {
            handler(req)
        } else {
            OpenDecision::Reject
        };

        match decision {
            OpenDecision::Accept(sender) => {
                match self.registry.allocate_with_remote(remote_id, sender) {
                    Ok(local_id) => {
                        let okay_packet = Packet::new(
                            Command::Okay,
                            local_id.get(),
                            remote_id.get(),
                            Vec::new(),
                            self.skip_checksum,
                        );
                        self.transport.write_packet(&okay_packet).await?;
                    }
                    Err(_) => {
                        self.send_close(None, remote_id).await?;
                    }
                }
            }
            OpenDecision::Reject => {
                self.send_close(None, remote_id).await?;
            }
        }

        Ok(())
    }

    async fn handle_wrte(&mut self, packet: Packet) -> Result<(), RouterError> {
        let (Some(remote_id), Some(local_id)) = (
            RemoteId::from_u32(packet.header.arg0),
            LocalId::from_u32(packet.header.arg1),
        ) else {
            return Ok(());
        };

        if let Some(entry) = self.registry.find(local_id, Some(remote_id)) {
            let res = entry.with_state_mut(|state| match state {
                StreamState::Open {
                    remote_id: rid,
                    inbound,
                    ..
                } if *rid == remote_id => {
                    if inbound.awaiting_ack {
                        Err(RouterError::ProtocolViolation(format!(
                            "consecutive WRTE without OKAY on stream local_id={local_id}, remote_id={remote_id}"
                        )))
                    } else {
                        inbound.awaiting_ack = true;
                        Ok(())
                    }
                }
                _ => Ok(()),
            });

            res?;

            let acker = StreamAcker {
                local_id,
                cmd_tx: self.cmd_tx.clone(),
            };

            let send_res = match entry.handle() {
                Some(h) => h.send(StreamMessage::Data(packet.payload, acker)),
                None => Err(tokio::sync::mpsc::error::SendError(StreamMessage::Close)),
            };
            if send_res.is_err() {
                // Stream receiver dropped, remove from registry and reply CLSE
                self.registry.remove(local_id);
                self.send_close(Some(local_id), remote_id).await?;
            }
        } else {
            // Unmapped stream target: trigger immediate CLSE reply without crashing transport
            self.send_close(Some(local_id), remote_id).await?;
        }

        Ok(())
    }

    async fn handle_okay(&mut self, packet: Packet) -> Result<(), RouterError> {
        let (Some(remote_id), Some(local_id)) = (
            RemoteId::from_u32(packet.header.arg0),
            LocalId::from_u32(packet.header.arg1),
        ) else {
            return Ok(());
        };

        if let Some(entry) = self.registry.find(local_id, None) {
            // If remote_id was not yet set, update it
            if entry.remote_id().is_none() {
                entry.set_remote_id(remote_id);
            }

            let ack_bytes = if packet.payload.len() == 4 {
                Some(AckBytes::from_le_bytes(
                    packet.payload[..4].try_into().expect("slice length 4"),
                ))
            } else {
                None
            };

            let next_to_send = entry.with_state_mut(|state| {
                if let StreamState::Open { outbound, .. } = state {
                    if let Some(resp_tx) = outbound.in_flight.take() {
                        let _ = resp_tx.send(Ok(()));
                    }
                    outbound.queued.pop_front()
                } else {
                    None
                }
            });

            if let Some(next_req) = next_to_send {
                self.send_stream_write(local_id, remote_id, next_req)
                    .await?;
            }

            let send_res = match entry.handle() {
                Some(h) => h.send(StreamMessage::Ready {
                    remote_id,
                    ack_bytes,
                }),
                None => Err(tokio::sync::mpsc::error::SendError(StreamMessage::Close)),
            };
            if send_res.is_err() {
                self.registry.remove(local_id);
                self.send_close(Some(local_id), remote_id).await?;
            }
        } else {
            // Unmapped stream target on OKAY: send CLSE back to device (C++ send_close(p->msg.arg1, p->msg.arg0, t))
            self.send_close(Some(local_id), remote_id).await?;
        }

        Ok(())
    }

    async fn handle_clse(&mut self, packet: Packet) {
        let Some(local_id) = LocalId::from_u32(packet.header.arg1) else {
            return;
        };
        let remote_id = RemoteId::from_u32(packet.header.arg0);

        // C++: find_local_socket(p->msg.arg1, p->msg.arg0)
        // Note: arg0 may be 0 for failed OPEN
        if let Some(entry) = self.registry.find(local_id, remote_id) {
            if let Some(h) = entry.handle() {
                let _ = h.send(StreamMessage::Close);
            }
            // Removing the stream cleans up and drops the stream state (cancelling in-flight/queued writes)
            self.registry.remove(local_id);
        }
        // Per protocol parity, unmapped CLSE messages are ignored without responding
    }

    async fn send_close(
        &mut self,
        local_id: Option<LocalId>,
        remote_id: RemoteId,
    ) -> Result<(), RouterError> {
        let clse_packet = Packet::new(
            Command::Clse,
            local_id.map_or(0, |id| id.get()),
            remote_id.get(),
            Vec::new(),
            self.skip_checksum,
        );
        self.transport.write_packet(&clse_packet).await?;
        Ok(())
    }
}
