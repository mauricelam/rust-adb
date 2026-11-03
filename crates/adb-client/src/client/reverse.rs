use super::error::ClientError;
use super::events::Outcome;
use super::forward::{forward_dest_is_featured, forward_targets_are_valid};
use super::AdbClient;
use crate::session::SessionError;
use crate::transport::TransportSelector;

impl AdbClient {
    /// Lists all reverse socket connections from device (`reverse:list-forward`).
    ///
    /// Ports `reverse --list` handling from `original/client/commandline.cpp`.
    pub async fn reverse_list(&self, selector: &TransportSelector) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service("reverse:list-forward").await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(format!("{payload}\n"))
        }
        .await;

        Outcome {
            result: match res {
                Ok(out) => Ok(out),
                Err(SessionError::Fail(reason)) => Err(ClientError::QueryFailed(reason)),
                Err(err) => Err(err.into()),
            },
            events: outcome.events,
        }
    }

    /// Adds a reverse socket connection (`reverse:forward:[norebind:]remote;local`).
    ///
    /// Ports `reverse` and `reverse --no-rebind` handling from `original/client/commandline.cpp`.
    pub async fn reverse_add(
        &self,
        selector: &TransportSelector,
        remote: &str,
        local: &str,
        no_rebind: bool,
    ) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        if let Err(msg) = forward_targets_are_valid(remote, local) {
            return Outcome {
                result: Err(ClientError::QueryFailed(msg)),
                events: outcome.events,
            };
        }

        if local.starts_with("dev-raw:") {
            let features_outcome = self
                .features(selector, &adb_proto::FeatureOptions::default())
                .await;
            let features_str = match features_outcome.result {
                Ok(f) => f,
                Err(err) => {
                    return Outcome {
                        result: Err(err),
                        events: outcome.events,
                    };
                }
            };
            let features = adb_proto::Features::parse(&features_str.replace('\n', ","));
            if let Err(msg) = forward_dest_is_featured(local, &features) {
                return Outcome {
                    result: Err(ClientError::QueryFailed(msg)),
                    events: outcome.events,
                };
            }
        }

        let cmd = if no_rebind {
            format!("reverse:forward:norebind:{remote};{local}")
        } else {
            format!("reverse:forward:{remote};{local}")
        };

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;

            let resolved_port = match session.read_string().await {
                Ok(port) if !port.is_empty() => format!("{port}\n"),
                _ => String::new(),
            };

            let _ = session.read_to_close().await;

            Ok(resolved_port)
        }
        .await;

        Outcome {
            result: match res {
                Ok(out) => Ok(out),
                Err(SessionError::Fail(reason)) => Err(ClientError::QueryFailed(reason)),
                Err(err) => Err(err.into()),
            },
            events: outcome.events,
        }
    }

    /// Removes a specific reverse socket connection (`reverse:killforward:<remote>`).
    ///
    /// Ports `reverse --remove` handling from `original/client/commandline.cpp`.
    pub async fn reverse_remove(
        &self,
        selector: &TransportSelector,
        remote: &str,
    ) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let cmd = format!("reverse:killforward:{remote}");
        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;
            let _ = session.read_to_close().await;
            Ok(String::new())
        }
        .await;

        Outcome {
            result: match res {
                Ok(out) => Ok(out),
                Err(SessionError::Fail(reason)) => Err(ClientError::QueryFailed(reason)),
                Err(err) => Err(err.into()),
            },
            events: outcome.events,
        }
    }

    /// Removes all reverse socket connections (`reverse:killforward-all`).
    ///
    /// Ports `reverse --remove-all` handling from `original/client/commandline.cpp`.
    pub async fn reverse_remove_all(&self, selector: &TransportSelector) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service("reverse:killforward-all").await?;
            session.read_status().await?;
            let _ = session.read_to_close().await;
            Ok(String::new())
        }
        .await;

        Outcome {
            result: match res {
                Ok(out) => Ok(out),
                Err(SessionError::Fail(reason)) => Err(ClientError::QueryFailed(reason)),
                Err(err) => Err(err.into()),
            },
            events: outcome.events,
        }
    }
}
