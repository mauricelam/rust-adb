use std::pin::Pin;

use super::error::ClientError;
use super::events::Outcome;
use super::AdbClient;

/// Mode selector for `track-devices` service variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackDevicesMode {
    Normal,
    Long,
    ProtoText,
    ProtoBinary,
}

impl TrackDevicesMode {
    /// Returns the host service string associated with this mode.
    pub fn service_name(&self) -> &'static str {
        match self {
            TrackDevicesMode::Normal => "host:track-devices",
            TrackDevicesMode::Long => "host:track-devices-l",
            TrackDevicesMode::ProtoText => "host:track-devices-proto-text",
            TrackDevicesMode::ProtoBinary => "host:track-devices-proto-binary",
        }
    }
}

impl AdbClient {
    /// Tracks device state changes over a raw stream (`host:track-devices*`).
    ///
    /// Ports `track-devices` command handling from `original/client/commandline.cpp`.
    /// Returns a stream of raw byte chunks as sent by the server. Dropping the stream closes the socket connection.
    pub async fn track_devices(
        &self,
        mode: TrackDevicesMode,
    ) -> Outcome<Pin<Box<dyn tokio_stream::Stream<Item = Result<bytes::Bytes, ClientError>> + Send>>>
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let session_res = self.session_factory.connect(&self.addr).await;
        let mut session = match session_res {
            Ok(s) => s,
            Err(e) => {
                return Outcome {
                    result: Err(e.into()),
                    events: outcome.events,
                }
            }
        };

        if let Err(e) = session.send_service(mode.service_name()).await {
            return Outcome {
                result: Err(e.into()),
                events: outcome.events,
            };
        }

        if let Err(e) = session.read_status().await {
            return Outcome {
                result: Err(e.into()),
                events: outcome.events,
            };
        }

        let stream = async_stream::try_stream! {
            use crate::session::SessionError;
            let mut stream = session.into_inner();
            let mut buf = [0u8; 8192];
            loop {
                use tokio::io::AsyncReadExt;
                let n = stream
                    .read(&mut buf)
                    .await
                    .map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;
                if n == 0 {
                    break;
                }
                yield bytes::Bytes::copy_from_slice(&buf[..n]);
            }
        };

        Outcome {
            result: Ok(Box::pin(stream)),
            events: outcome.events,
        }
    }
}
