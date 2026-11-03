//! Forward listener management and local socket proxying.
//!
//! Ports `original/adb_listeners.cpp` and `original/adb_listeners.h`.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::registry::DeviceEntry;

/// Error or status codes for `install_listener`.
///
/// Matches `InstallStatus` enum from `original/adb_listeners.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallStatus {
    Ok = 0,
    InternalError = -1,
    CannotBind = -2,
    CannotRebind = -3,
    ListenerNotFound = -4,
}

/// Installation flags.
pub const INSTALL_LISTENER_NO_REBIND: i32 = 1 << 0;

/// Represents an active forward listener.
struct ActiveListener {
    local_name: String,
    connect_to: String,
    transport: Option<Arc<DeviceEntry>>,
    shutdown_tx: watch::Sender<bool>,
}

/// Thread-safe manager for forward listeners.
pub struct ListenerManager {
    listeners: RwLock<HashMap<String, ActiveListener>>,
}

impl Default for ListenerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ListenerManager {
    /// Creates a new `ListenerManager`.
    pub fn new() -> Self {
        Self {
            listeners: RwLock::new(HashMap::new()),
        }
    }

    /// Formats all active listeners into the standard string format:
    /// `<serial> <local-name> <connect-to>\n`
    ///
    /// Matches `format_listeners()` in `original/adb_listeners.cpp`.
    pub fn format_listeners(&self) -> String {
        let listeners = self.listeners.read();
        let mut result = String::new();
        // C++ ADB iterates listener list
        for l in listeners.values() {
            let serial = match &l.transport {
                Some(t) if !t.serial.is_empty() => t.serial.as_str(),
                _ => "(reverse)",
            };
            result.push_str(&format!("{} {} {}\n", serial, l.local_name, l.connect_to));
        }
        result
    }

    /// Removes a specific listener by local name.
    ///
    /// Matches `remove_listener` in `original/adb_listeners.cpp`.
    pub fn remove_listener(&self, local_name: &str) -> InstallStatus {
        let mut listeners = self.listeners.write();
        if let Some(removed) = listeners.remove(local_name) {
            let _ = removed.shutdown_tx.send(true);
            InstallStatus::Ok
        } else {
            InstallStatus::ListenerNotFound
        }
    }

    /// Removes all listeners.
    ///
    /// Matches `remove_all_listeners` in `original/adb_listeners.cpp`.
    pub fn remove_all_listeners(&self) {
        let mut listeners = self.listeners.write();
        for (_, l) in listeners.drain() {
            let _ = l.shutdown_tx.send(true);
        }
    }

    /// Removes all listeners associated with a given transport.
    pub fn remove_transport_listeners(&self, transport: &Arc<DeviceEntry>) {
        let mut listeners = self.listeners.write();
        let mut to_remove = Vec::new();
        for (name, l) in listeners.iter() {
            if let Some(ref t) = l.transport {
                if t.transport_id == transport.transport_id {
                    to_remove.push(name.clone());
                }
            }
        }
        for name in to_remove {
            if let Some(l) = listeners.remove(&name) {
                let _ = l.shutdown_tx.send(true);
            }
        }
    }

    /// Installs a new listener or rebinds an existing listener.
    ///
    /// Matches `install_listener` in `original/adb_listeners.cpp`.
    pub async fn install_listener(
        self: &Arc<Self>,
        local_name: &str,
        connect_to: &str,
        transport: Option<Arc<DeviceEntry>>,
        flags: i32,
    ) -> (InstallStatus, Option<u16>, String) {
        // Check if listener already exists
        {
            let mut listeners = self.listeners.write();
            if let Some(existing) = listeners.get_mut(local_name) {
                if flags & INSTALL_LISTENER_NO_REBIND != 0 {
                    return (
                        InstallStatus::CannotRebind,
                        None,
                        "cannot rebind existing socket".to_string(),
                    );
                }

                existing.connect_to = connect_to.to_string();
                existing.transport = transport;
                return (InstallStatus::Ok, None, String::new());
            }
        }

        // Parse and bind local listener
        // Currently supporting tcp:PORT (including tcp:0)
        let (listener, resolved_name, resolved_port) = match bind_local_spec(local_name).await {
            Ok(res) => res,
            Err(err) => {
                return (
                    InstallStatus::CannotBind,
                    None,
                    format!("cannot bind listener: {err}"),
                );
            }
        };

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Store active listener
        {
            let mut listeners = self.listeners.write();
            // Re-check in case another task bound same resolved_name concurrently
            if listeners.contains_key(&resolved_name) && (flags & INSTALL_LISTENER_NO_REBIND != 0) {
                return (
                    InstallStatus::CannotRebind,
                    None,
                    "cannot rebind existing socket".to_string(),
                );
            }

            listeners.insert(
                resolved_name.clone(),
                ActiveListener {
                    local_name: resolved_name.clone(),
                    connect_to: connect_to.to_string(),
                    transport,
                    shutdown_tx,
                },
            );
        }

        // Spawn accept & proxy loop
        let manager_weak = Arc::downgrade(self);
        let listen_key = resolved_name.clone();
        tokio::spawn(async move {
            run_listener_loop(listener, listen_key, manager_weak, shutdown_rx).await;
        });

        (InstallStatus::Ok, resolved_port, String::new())
    }
}

async fn bind_local_spec(local_name: &str) -> Result<(TcpListener, String, Option<u16>), String> {
    if let Some(port_str) = local_name.strip_prefix("tcp:") {
        let port: u16 = port_str
            .parse()
            .map_err(|e| format!("invalid port '{port_str}': {e}"))?;
        let bind_addr = format!("127.0.0.1:{port}");
        let listener = TcpListener::bind(&bind_addr)
            .await
            .map_err(|e| format!("{e}"))?;
        let actual_port = listener.local_addr().map_err(|e| format!("{e}"))?.port();

        if port == 0 {
            Ok((listener, format!("tcp:{actual_port}"), Some(actual_port)))
        } else {
            Ok((listener, local_name.to_string(), None))
        }
    } else {
        Err(format!("unsupported socket spec '{local_name}'"))
    }
}

async fn run_listener_loop(
    listener: TcpListener,
    local_name: String,
    manager_weak: std::sync::Weak<ListenerManager>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((stream, _addr)) => {
                        let manager = match manager_weak.upgrade() {
                            Some(m) => m,
                            None => break,
                        };

                        let (connect_to, transport) = {
                            let listeners = manager.listeners.read();
                            match listeners.get(&local_name) {
                                Some(l) => (l.connect_to.clone(), l.transport.clone()),
                                None => break,
                            }
                        };

                        tokio::spawn(async move {
                            proxy_forwarded_connection(stream, connect_to, transport).await;
                        });
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                }
            }
        }
    }
}

async fn proxy_forwarded_connection(
    client_stream: TcpStream,
    connect_to: String,
    _transport: Option<Arc<DeviceEntry>>,
) {
    // If destination is tcp:PORT, connect to target TCP service
    if let Some(port_str) = connect_to.strip_prefix("tcp:") {
        let Ok(port) = port_str.parse::<u16>() else {
            return;
        };

        let target_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let target_stream = match TcpStream::connect(target_addr).await {
            Ok(s) => s,
            Err(_) => return,
        };

        let (mut client_read, mut client_write) = client_stream.into_split();
        let (mut target_read, mut target_write) = target_stream.into_split();

        let client_to_target = async {
            let mut buf = [0u8; 8192];
            loop {
                let n = match client_read.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if target_write.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            let _ = target_write.shutdown().await;
        };

        let target_to_client = async {
            let mut buf = [0u8; 8192];
            loop {
                let n = match target_read.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if client_write.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            let _ = client_write.shutdown().await;
        };

        tokio::join!(client_to_target, target_to_client);
    }
}
