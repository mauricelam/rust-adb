use super::error::ClientError;
use super::events::Outcome;
use super::AdbClient;
use crate::session::SessionError;
use crate::transport::TransportSelector;

/// Validates forward/reverse targets.
///
/// Ports `forward_targets_are_valid` from C++ `original/adb_utils.cpp`.
pub fn forward_targets_are_valid(source: &str, dest: &str) -> Result<(), String> {
    if let Some(port_str) = source.strip_prefix("tcp:") {
        match port_str.parse::<i32>() {
            Ok(port) if port >= 0 => {}
            _ => return Err(format!("Invalid source port: '{port_str}'")),
        }
    }

    if let Some(port_str) = dest.strip_prefix("tcp:") {
        match port_str.parse::<i32>() {
            Ok(port) if port > 0 => {}
            _ => return Err(format!("Invalid destination port: '{port_str}'")),
        }
    }

    Ok(())
}

/// Checks if destination target is supported by device feature set.
///
/// Ports `forward_dest_is_featured` from C++ `original/client/commandline.cpp`.
pub fn forward_dest_is_featured(dest: &str, features: &adb_proto::Features) -> Result<(), String> {
    if dest.starts_with("dev-raw:") && !features.contains(&adb_proto::Feature::DevRaw) {
        return Err("dev-raw is not supported by the device".to_string());
    }
    Ok(())
}

impl AdbClient {
    /// Lists all forward socket connections (`host:list-forward`).
    ///
    /// Ports `forward --list` handling from `original/client/commandline.cpp`.
    pub async fn forward_list(&self, _selector: &TransportSelector) -> Outcome<String> {
        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service("host:list-forward").await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(format!("{payload}\n"))
        })
        .await
    }

    /// Adds a forward socket connection.
    ///
    /// Ports `forward` and `forward --no-rebind` handling from `original/client/commandline.cpp`.
    pub async fn forward_add(
        &self,
        selector: &TransportSelector,
        local: &str,
        remote: &str,
        no_rebind: bool,
    ) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        if let Err(msg) = forward_targets_are_valid(local, remote) {
            return Outcome {
                result: Err(ClientError::QueryFailed(msg)),
                events: outcome.events,
            };
        }

        if remote.starts_with("dev-raw:") {
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
            if let Err(msg) = forward_dest_is_featured(remote, &features) {
                return Outcome {
                    result: Err(ClientError::QueryFailed(msg)),
                    events: outcome.events,
                };
            }
        }

        let cmd = if no_rebind {
            format!("host:forward:norebind:{local};{remote}")
        } else {
            format!("host:forward:{local};{remote}")
        };

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;
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

    /// Removes a specific forward socket connection (`host:killforward:<local>`).
    ///
    /// Ports `forward --remove` handling from `original/client/commandline.cpp`.
    pub async fn forward_remove(
        &self,
        selector: &TransportSelector,
        local: &str,
    ) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let cmd = format!("host:killforward:{local}");
        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;
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

    /// Removes all forward socket connections (`host:killforward-all`).
    ///
    /// Ports `forward --remove-all` handling from `original/client/commandline.cpp`.
    pub async fn forward_remove_all(&self, selector: &TransportSelector) -> Outcome<String> {
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
            session.send_service("host:killforward-all").await?;
            session.read_status().await?;
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
