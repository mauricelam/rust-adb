use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

use adb_proto::Command;
use adb_transport::{Packet, Transport, TransportError};

use crate::stream::StreamRegistry;

/// Handle passed with incoming data for acknowledging receipt via A_OKAY.
#[derive(Clone, Debug)]
pub struct StreamAcker {
    local_id: u32,
    cmd_tx: mpsc::Sender<RouterCommand>,
}

impl StreamAcker {
    /// Returns the local stream ID.
    pub fn local_id(&self) -> u32 {
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
        remote_id: u32,
        ack_bytes: Option<i32>,
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
    pub remote_id: u32,
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

struct StreamWriteRequest {
    local_id: u32,
    payload: Vec<u8>,
    response_tx: oneshot::Sender<Result<(), RouterError>>,
}

enum RouterCommand {
    Write(StreamWriteRequest),
    AckInbound(u32, oneshot::Sender<Result<(), RouterError>>),
}

/// Handle allowing asynchronous writing to a multiplexed stream with OKAY flow control backpressure.
#[derive(Clone)]
pub struct StreamWriter {
    local_id: u32,
    cmd_tx: mpsc::Sender<RouterCommand>,
}

impl StreamWriter {
    /// Returns the local stream ID.
    pub fn local_id(&self) -> u32 {
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

    /// Sends an A_OKAY acknowledgment back to the remote peer for inbound data received on this stream.
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

/// Main async packet dispatch loop routing `OPEN`, `WRTE`, `OKAY`, and `CLSE` packets.
pub struct PacketRouter<T: Transport> {
    transport: T,
    registry: Arc<StreamRegistry<StreamSender>>,
    open_handler: Option<OpenHandler>,
    skip_checksum: bool,
    cmd_tx: mpsc::Sender<RouterCommand>,
    cmd_rx: mpsc::Receiver<RouterCommand>,
    outgoing_in_flight: HashMap<u32, oneshot::Sender<Result<(), RouterError>>>,
    outgoing_queued: HashMap<u32, VecDeque<StreamWriteRequest>>,
    inbound_in_flight: HashSet<u32>,
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
            outgoing_in_flight: HashMap::new(),
            outgoing_queued: HashMap::new(),
            inbound_in_flight: HashSet::new(),
        }
    }

    /// Creates a `StreamWriter` for the given `local_id`.
    pub fn create_stream_writer(&self, local_id: u32) -> Result<StreamWriter, RouterError> {
        if self.registry.get(local_id).is_none() {
            return Err(RouterError::Stream(format!("stream {local_id} not found")));
        }
        Ok(StreamWriter {
            local_id,
            cmd_tx: self.cmd_tx.clone(),
        })
    }

    /// Sends an `A_OKAY` packet acknowledging received data on the given stream, clearing inbound write-in-flight state.
    pub async fn send_okay(&mut self, local_id: u32) -> Result<(), RouterError> {
        let entry = self
            .registry
            .get(local_id)
            .ok_or_else(|| RouterError::Stream(format!("stream {local_id} not found")))?;
        let remote_id = entry
            .remote_id()
            .ok_or_else(|| RouterError::Stream("remote_id not yet established".to_string()))?;

        self.inbound_in_flight.remove(&local_id);

        let okay_packet = Packet::new(
            Command::Okay,
            local_id,
            remote_id,
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

        // Cleanup: close all active streams in registry on exit
        let active_streams = self.registry.clear();
        for entry in active_streams {
            let _ = entry.handle().send(StreamMessage::Close);
        }

        for (_, resp_tx) in self.outgoing_in_flight.drain() {
            let _ = resp_tx.send(Err(RouterError::Stream("connection closed".to_string())));
        }
        for (_, mut queue) in self.outgoing_queued.drain() {
            while let Some(req) = queue.pop_front() {
                let _ = req
                    .response_tx
                    .send(Err(RouterError::Stream("connection closed".to_string())));
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

        if entry.remote_id().is_none() {
            let _ = req.response_tx.send(Err(RouterError::Stream(
                "remote_id not yet established".to_string(),
            )));
            return Ok(());
        }

        if self.outgoing_in_flight.contains_key(&local_id) {
            self.outgoing_queued
                .entry(local_id)
                .or_default()
                .push_back(req);
            return Ok(());
        }

        self.send_stream_write(req).await
    }

    async fn send_stream_write(&mut self, req: StreamWriteRequest) -> Result<(), RouterError> {
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

        let remote_id = match entry.remote_id() {
            Some(rid) => rid,
            None => {
                let _ = req.response_tx.send(Err(RouterError::Stream(
                    "remote_id not yet established".to_string(),
                )));
                return Ok(());
            }
        };

        let wrte_packet = Packet::new(
            Command::Wrte,
            local_id,
            remote_id,
            req.payload,
            self.skip_checksum,
        );

        if let Err(err) = self.transport.write_packet(&wrte_packet).await {
            let _ = req.response_tx.send(Err(RouterError::Transport(err)));
            return Ok(());
        }

        self.outgoing_in_flight.insert(local_id, req.response_tx);
        Ok(())
    }

    async fn handle_open(&mut self, packet: Packet) -> Result<(), RouterError> {
        let remote_id = packet.header.arg0;
        let send_bytes = packet.header.arg1;

        if remote_id == 0 {
            return Ok(());
        }

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
                            local_id,
                            remote_id,
                            Vec::new(),
                            self.skip_checksum,
                        );
                        self.transport.write_packet(&okay_packet).await?;
                    }
                    Err(_) => {
                        self.send_close(0, remote_id).await?;
                    }
                }
            }
            OpenDecision::Reject => {
                self.send_close(0, remote_id).await?;
            }
        }

        Ok(())
    }

    async fn handle_wrte(&mut self, packet: Packet) -> Result<(), RouterError> {
        let remote_id = packet.header.arg0;
        let local_id = packet.header.arg1;

        if remote_id == 0 || local_id == 0 {
            return Ok(());
        }

        if let Some(entry) = self.registry.find(local_id, remote_id) {
            if self.inbound_in_flight.contains(&local_id) {
                return Err(RouterError::ProtocolViolation(format!(
                    "consecutive WRTE without OKAY on stream local_id={local_id}, remote_id={remote_id}"
                )));
            }

            self.inbound_in_flight.insert(local_id);

            let acker = StreamAcker {
                local_id,
                cmd_tx: self.cmd_tx.clone(),
            };

            if entry
                .handle()
                .send(StreamMessage::Data(packet.payload, acker))
                .is_err()
            {
                // Stream receiver dropped, remove from registry and reply CLSE
                self.registry.remove(local_id);
                self.inbound_in_flight.remove(&local_id);
                self.send_close(local_id, remote_id).await?;
            }
        } else {
            // Unmapped stream target: trigger immediate CLSE reply without crashing transport
            self.send_close(local_id, remote_id).await?;
        }

        Ok(())
    }

    async fn handle_okay(&mut self, packet: Packet) -> Result<(), RouterError> {
        let remote_id = packet.header.arg0;
        let local_id = packet.header.arg1;

        if remote_id == 0 || local_id == 0 {
            return Ok(());
        }

        if let Some(entry) = self.registry.find(local_id, 0) {
            // If remote_id was not yet set, update it
            if entry.remote_id().is_none() {
                let _ = entry.set_remote_id(remote_id);
            }

            let ack_bytes = if packet.payload.len() == 4 {
                Some(i32::from_le_bytes(
                    packet.payload[..4].try_into().expect("slice length 4"),
                ))
            } else {
                None
            };

            if let Some(resp_tx) = self.outgoing_in_flight.remove(&local_id) {
                let _ = resp_tx.send(Ok(()));
            }

            if let Some(queue) = self.outgoing_queued.get_mut(&local_id) {
                if let Some(next_req) = queue.pop_front() {
                    self.send_stream_write(next_req).await?;
                }
            }

            if entry
                .handle()
                .send(StreamMessage::Ready {
                    remote_id,
                    ack_bytes,
                })
                .is_err()
            {
                self.registry.remove(local_id);
                self.inbound_in_flight.remove(&local_id);
                self.send_close(local_id, remote_id).await?;
            }
        } else {
            // Unmapped stream target on OKAY: send CLSE back to device (C++ send_close(p->msg.arg1, p->msg.arg0, t))
            self.send_close(local_id, remote_id).await?;
        }

        Ok(())
    }

    async fn handle_clse(&mut self, packet: Packet) {
        let remote_id = packet.header.arg0;
        let local_id = packet.header.arg1;

        if local_id == 0 {
            return;
        }

        // C++: find_local_socket(p->msg.arg1, p->msg.arg0)
        // Note: arg0 may be 0 for failed OPEN
        if let Some(entry) = self.registry.find(local_id, remote_id) {
            let _ = entry.handle().send(StreamMessage::Close);
            self.registry.remove(local_id);
            self.inbound_in_flight.remove(&local_id);
            if let Some(resp_tx) = self.outgoing_in_flight.remove(&local_id) {
                let _ = resp_tx.send(Err(RouterError::Stream(
                    "stream closed by peer".to_string(),
                )));
            }
            if let Some(mut queue) = self.outgoing_queued.remove(&local_id) {
                while let Some(req) = queue.pop_front() {
                    let _ = req.response_tx.send(Err(RouterError::Stream(
                        "stream closed by peer".to_string(),
                    )));
                }
            }
        }
        // Per protocol parity, unmapped CLSE messages are ignored without responding
    }

    async fn send_close(&mut self, local_id: u32, remote_id: u32) -> Result<(), RouterError> {
        let clse_packet = Packet::new(
            Command::Clse,
            local_id,
            remote_id,
            Vec::new(),
            self.skip_checksum,
        );
        self.transport.write_packet(&clse_packet).await?;
        Ok(())
    }
}
