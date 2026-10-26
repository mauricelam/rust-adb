//! Argument parsing and CLI helpers for `adb`.

use adb_client::{AdbClient, ClientError, Outcome, TrackDevicesMode, TransportSelector};
use std::pin::Pin;

/// Parses arguments for `adb devices` and executes `client.devices(long)`.
pub async fn parse_and_exec_devices(client: &AdbClient, args: &[&str]) -> Outcome<String> {
    let long = match args {
        [] => false,
        ["-l"] => true,
        _ => {
            return Outcome {
                result: Err(ClientError::InvalidUsage("adb devices [-l]".to_string())),
                events: Vec::new(),
            };
        }
    };
    client.devices(long).await
}

/// Parses arguments for `adb connect` and executes `client.connect(target)`.
pub async fn parse_and_exec_connect(client: &AdbClient, args: &[&str]) -> Outcome<String> {
    match args {
        [target] => client.connect(target).await,
        _ => Outcome {
            result: Err(ClientError::InvalidUsage(
                "usage: adb connect HOST[:PORT]".to_string(),
            )),
            events: Vec::new(),
        },
    }
}

/// Parses arguments for `adb disconnect` and executes `client.disconnect(target)`.
pub async fn parse_and_exec_disconnect(client: &AdbClient, args: &[&str]) -> Outcome<String> {
    match args {
        [] => client.disconnect(None).await,
        [target] => client.disconnect(Some(target)).await,
        _ => Outcome {
            result: Err(ClientError::InvalidUsage(
                "usage: adb disconnect [HOST[:PORT]]".to_string(),
            )),
            events: Vec::new(),
        },
    }
}

/// Parses arguments for `adb track-devices` and executes `client.track_devices(mode)`.
pub async fn parse_and_exec_track_devices(
    client: &AdbClient,
    args: &[&str],
) -> Outcome<Pin<Box<dyn tokio_stream::Stream<Item = Result<bytes::Bytes, ClientError>> + Send>>> {
    let mode = match args {
        [] => TrackDevicesMode::Normal,
        ["-l"] => TrackDevicesMode::Long,
        ["--proto-text"] => TrackDevicesMode::ProtoText,
        ["--proto-binary"] => TrackDevicesMode::ProtoBinary,
        _ => {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb track-devices [-l][--proto-text][--proto-binary]".to_string(),
                )),
                events: Vec::new(),
            };
        }
    };
    client.track_devices(mode).await
}

/// Parses arguments for `adb forward` and executes appropriate forward method on `client`.
pub async fn parse_and_exec_forward(
    client: &AdbClient,
    selector: &TransportSelector,
    args: &[&str],
) -> Outcome<String> {
    if args.is_empty() {
        return Outcome {
            result: Err(ClientError::InvalidUsage(
                "forward requires an argument".to_string(),
            )),
            events: Vec::new(),
        };
    }

    match args[0] {
        "--list" => {
            if args.len() != 1 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--list doesn't take any arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_list(selector).await
        }
        "--remove-all" => {
            if args.len() != 1 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--remove-all doesn't take any arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_remove_all(selector).await
        }
        "--remove" => {
            if args.len() != 2 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--remove requires an argument".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_remove(selector, args[1]).await
        }
        "--no-rebind" => {
            if args.len() != 3 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "--no-rebind takes two arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_add(selector, args[1], args[2], true).await
        }
        local => {
            if args.len() != 2 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "forward takes two arguments".to_string(),
                    )),
                    events: Vec::new(),
                };
            }
            client.forward_add(selector, local, args[1], false).await
        }
    }
}
