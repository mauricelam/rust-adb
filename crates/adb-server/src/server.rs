use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use adb_proto::MAX_PAYLOAD;

use adb_transport::DEFAULT_ADB_LOCAL_TRANSPORT_PORT;

use crate::registry::{DeviceEntry, DeviceRegistry, TransportId};
use crate::tracker::{format_devices_raw, TrackerFormat};
use crate::transport_type::TransportType;

/// ADB server internal protocol version.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Default ADB server port.
pub const DEFAULT_SERVER_PORT: u16 = 5037;

/// Target transport selection criteria parsed from host service prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportTarget {
    Any,
    Usb,
    Local,
    Serial(String),
    TransportId(TransportId),
}

/// Parsed smart-socket request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartSocketRequest {
    /// Host request, e.g. `host:version`, `host:transport:XYZ`, `host-serial:XYZ:shell:ls`.
    Host {
        target: TransportTarget,
        service: String,
    },
    /// Direct device service request on an established/selected transport, e.g. `123456789012` or `shell:ls`.
    Device(String),
}

/// Formats an `OKAY` reply without payload.
pub fn encode_okay() -> &'static [u8] {
    b"OKAY"
}

/// Formats an `OKAY` reply with a 4-hex-digit length-prefixed payload string.
pub fn encode_okay_with_payload(payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 4 + payload.len());
    out.extend_from_slice(b"OKAY");
    let hex_len = format!("{:04x}", payload.len());
    out.extend_from_slice(hex_len.as_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

/// Formats a `FAIL` reply with a 4-hex-digit length-prefixed error message.
pub fn encode_fail(reason: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + 4 + reason.len());
    out.extend_from_slice(b"FAIL");
    let hex_len = format!("{:04x}", reason.len());
    out.extend_from_slice(hex_len.as_bytes());
    out.extend_from_slice(reason.as_bytes());
    out
}

/// Parses a smart socket service request string into a [`SmartSocketRequest`].
///
/// Follows C++ smart socket dispatch logic in `original/sockets.cpp` (`smart_socket_enqueue`).
pub fn parse_smart_socket_request(request: &str) -> SmartSocketRequest {
    if let Some(rest) = request.strip_prefix("host-serial:") {
        if let Some((serial, cmd)) = parse_host_serial_service(rest) {
            SmartSocketRequest::Host {
                target: TransportTarget::Serial(serial),
                service: cmd,
            }
        } else {
            // Fallback if parsing fails
            SmartSocketRequest::Host {
                target: TransportTarget::Any,
                service: rest.to_string(),
            }
        }
    } else if let Some(rest) = request.strip_prefix("host-transport-id:") {
        if let Some(colon_pos) = rest.find(':') {
            let id_str = &rest[..colon_pos];
            let cmd = &rest[colon_pos + 1..];
            if let Ok(id) = id_str.parse::<u64>() {
                SmartSocketRequest::Host {
                    target: TransportTarget::TransportId(id),
                    service: cmd.to_string(),
                }
            } else {
                SmartSocketRequest::Host {
                    target: TransportTarget::Any,
                    service: cmd.to_string(),
                }
            }
        } else {
            SmartSocketRequest::Host {
                target: TransportTarget::Any,
                service: rest.to_string(),
            }
        }
    } else if let Some(cmd) = request.strip_prefix("host-usb:") {
        SmartSocketRequest::Host {
            target: TransportTarget::Usb,
            service: cmd.to_string(),
        }
    } else if let Some(cmd) = request.strip_prefix("host-local:") {
        SmartSocketRequest::Host {
            target: TransportTarget::Local,
            service: cmd.to_string(),
        }
    } else if let Some(cmd) = request.strip_prefix("host:") {
        SmartSocketRequest::Host {
            target: TransportTarget::Any,
            service: cmd.to_string(),
        }
    } else {
        SmartSocketRequest::Device(request.to_string())
    }
}

fn parse_host_serial_service(rest: &str) -> Option<(String, String)> {
    // Matches internal::parse_host_service in original/sockets.cpp:
    // Handles prefixes like usb:, product:, model:, device:, localfilesystem:,
    // tcp:, udp:, [ipv6]:port, or plain <serial>:<cmd>.
    if rest.is_empty() {
        return None;
    }

    // Check known prefixes
    let prefixes = ["usb:", "product:", "model:", "device:", "localfilesystem:"];
    for prefix in &prefixes {
        if let Some(remainder) = rest.strip_prefix(prefix) {
            let colon = remainder.find(':')?;
            let serial = &rest[..prefix.len() + colon];
            let cmd = &remainder[colon + 1..];
            return Some((serial.to_string(), cmd.to_string()));
        }
    }

    // Look for colon
    let colon = rest.find(':')?;
    let serial = &rest[..colon];
    let remainder = &rest[colon + 1..];

    // Check if remainder starts with a port followed by colon
    if let Some(next_colon) = remainder.find(':') {
        let port_part = &remainder[..next_colon];
        if !port_part.is_empty() && port_part.chars().all(|c| c.is_ascii_digit()) {
            let full_serial = format!("{serial}:{port_part}");
            let cmd = &remainder[next_colon + 1..];
            return Some((full_serial, cmd.to_string()));
        }
    }

    Some((serial.to_string(), remainder.to_string()))
}

/// Errors that can occur in the smart-socket server listener.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("could not install *smartsocket* listener: {0}")]
    CannotBind(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("server shutdown")]
    Shutdown,
}

use crate::listener::{InstallStatus, ListenerManager, INSTALL_LISTENER_NO_REBIND};

/// Smart-socket TCP server listener and request dispatcher.
pub struct SmartSocketServer {
    listener: TcpListener,
    registry: Arc<DeviceRegistry>,
    listener_manager: Arc<ListenerManager>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl SmartSocketServer {
    /// Binds to the specified socket address or string.
    pub async fn bind(addr: &str, registry: Arc<DeviceRegistry>) -> Result<Self, ServerError> {
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::AddrInUse {
                    return Err(ServerError::CannotBind(
                        "Address already in use".to_string(),
                    ));
                }
                return Err(ServerError::Io(e));
            }
        };
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Ok(Self {
            listener,
            registry,
            listener_manager: Arc::new(ListenerManager::new()),
            shutdown_tx,
            shutdown_rx,
        })
    }

    /// Binds using the default port (5037) or the value specified in `ANDROID_ADB_SERVER_PORT`.
    pub async fn bind_default(registry: Arc<DeviceRegistry>) -> Result<Self, ServerError> {
        let port = match std::env::var("ANDROID_ADB_SERVER_PORT") {
            Ok(val) => val.parse::<u16>().unwrap_or(DEFAULT_SERVER_PORT),
            Err(_) => DEFAULT_SERVER_PORT,
        };
        let addr = format!("127.0.0.1:{port}");
        Self::bind(&addr, registry).await
    }

    /// Returns the local socket address this listener is bound to.
    pub fn local_addr(&self) -> Result<SocketAddr, ServerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Returns a shutdown sender that can trigger server termination.
    pub fn shutdown_handle(&self) -> watch::Sender<bool> {
        self.shutdown_tx.clone()
    }

    /// Runs the server listener dispatch loop until shutdown is signaled or error occurs.
    pub async fn run(self) -> Result<(), ServerError> {
        let registry = self.registry;
        let listener_manager = self.listener_manager;
        let shutdown_tx = self.shutdown_tx;
        let mut shutdown_rx = self.shutdown_rx;

        loop {
            tokio::select! {
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
                accept_res = self.listener.accept() => {
                    match accept_res {
                        Ok((stream, _addr)) => {
                            let reg = Arc::clone(&registry);
                            let lm = Arc::clone(&listener_manager);
                            let s_tx = shutdown_tx.clone();
                            tokio::spawn(async move {
                                let _ = handle_client(stream, reg, lm, s_tx).await;
                            });
                        }
                        Err(err) => {
                            // Non-fatal accept error
                            if err.kind() == std::io::ErrorKind::Interrupted {
                                continue;
                            }
                            return Err(ServerError::Io(err));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Client connection session state.
struct ClientSession {
    stream: TcpStream,
    selected_device: Option<TransportId>,
}

impl ClientSession {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            selected_device: None,
        }
    }
}

async fn handle_client(
    stream: TcpStream,
    registry: Arc<DeviceRegistry>,
    listener_manager: Arc<ListenerManager>,
    shutdown_tx: watch::Sender<bool>,
) -> Result<(), std::io::Error> {
    let mut session = ClientSession::new(stream);

    loop {
        // Read 4-hex-digit length prefix
        let mut prefix_buf = [0u8; 4];
        let mut bytes_read = 0;

        while bytes_read < 4 {
            let n = session.stream.read(&mut prefix_buf[bytes_read..]).await?;
            if n == 0 {
                // Client closed socket
                return Ok(());
            }
            bytes_read += n;
        }

        // Validate hex length prefix
        let hex_str = match std::str::from_utf8(&prefix_buf) {
            Ok(s) => s,
            Err(_) => {
                session
                    .stream
                    .write_all(&encode_fail("invalid length prefix"))
                    .await?;
                session.stream.flush().await?;
                let _ = session.stream.shutdown().await;
                return Ok(());
            }
        };

        let length = match usize::from_str_radix(hex_str, 16) {
            Ok(len) if len > 0 && len <= MAX_PAYLOAD => len,
            Ok(0) => {
                session
                    .stream
                    .write_all(&encode_fail("invalid request length"))
                    .await?;
                session.stream.flush().await?;
                let _ = session.stream.shutdown().await;
                return Ok(());
            }
            Ok(_) => {
                session
                    .stream
                    .write_all(&encode_fail("request too long"))
                    .await?;
                session.stream.flush().await?;
                let _ = session.stream.shutdown().await;
                return Ok(());
            }
            Err(_) => {
                session
                    .stream
                    .write_all(&encode_fail("invalid hex length"))
                    .await?;
                session.stream.flush().await?;
                let _ = session.stream.shutdown().await;
                return Ok(());
            }
        };

        // Read payload of specified length
        let mut payload = vec![0u8; length];
        session.stream.read_exact(&mut payload).await?;

        let request_str = match std::str::from_utf8(&payload) {
            Ok(s) => s,
            Err(_) => {
                session
                    .stream
                    .write_all(&encode_fail("invalid utf-8"))
                    .await?;
                session.stream.flush().await?;
                return Ok(());
            }
        };

        let request = parse_smart_socket_request(request_str);

        match request {
            SmartSocketRequest::Host { target, service } => {
                let should_continue = handle_host_service(
                    &mut session,
                    &target,
                    &service,
                    &registry,
                    &listener_manager,
                    &shutdown_tx,
                )
                .await?;

                if !should_continue {
                    return Ok(());
                }
            }
            SmartSocketRequest::Device(device_service) => {
                handle_device_service(&mut session, &device_service).await?;
                return Ok(());
            }
        }
    }
}

async fn handle_host_service(
    session: &mut ClientSession,
    target: &TransportTarget,
    service: &str,
    registry: &Arc<DeviceRegistry>,
    listener_manager: &Arc<ListenerManager>,
    shutdown_tx: &watch::Sender<bool>,
) -> Result<bool, std::io::Error> {
    if service == "version" {
        let resp = format!("OKAY0004{:04x}", ADB_SERVER_VERSION);
        session.stream.write_all(resp.as_bytes()).await?;
        session.stream.flush().await?;
        return Ok(false);
    }

    if service == "kill" {
        session.stream.write_all(encode_okay()).await?;
        session.stream.flush().await?;
        let _ = shutdown_tx.send(true);
        return Ok(false);
    }

    if service == "devices" || service == "devices-l" {
        let format = if service == "devices-l" {
            TrackerFormat::LongText
        } else {
            TrackerFormat::ShortText
        };
        let devices = registry.list_devices();
        let payload_bytes = format_devices_raw(&devices, format);
        let payload_str = String::from_utf8_lossy(&payload_bytes);
        let resp = encode_okay_with_payload(&payload_str);
        session.stream.write_all(&resp).await?;
        session.stream.flush().await?;
        return Ok(false);
    }

    if let Some(address) = service.strip_prefix("disconnect:") {
        if address.is_empty() {
            // Kick all TCP devices
            let devices = registry.list_devices();
            for dev in devices {
                if dev.transport_type == TransportType::Local {
                    let _ = registry.unregister_by_id(dev.transport_id);
                }
            }
            let resp = encode_okay_with_payload("disconnected everything");
            session.stream.write_all(&resp).await?;
            session.stream.flush().await?;
            return Ok(false);
        }

        // Check if device matches directly by address
        if let Some(dev) = registry.get_by_serial(address) {
            let _ = registry.unregister_by_id(dev.transport_id);
            let resp = encode_okay_with_payload(&format!("disconnected {address}"));
            session.stream.write_all(&resp).await?;
            session.stream.flush().await?;
            return Ok(false);
        }

        // Parse net address to canonical serial
        let (serial, parse_err) = parse_net_address_canonical(address);
        if let Some(err) = parse_err {
            let resp = encode_fail(&format!("couldn't parse '{address}': {err}"));
            session.stream.write_all(&resp).await?;
            session.stream.flush().await?;
            return Ok(false);
        }

        if let Some(dev) = registry.get_by_serial(&serial) {
            let _ = registry.unregister_by_id(dev.transport_id);
            let resp = encode_okay_with_payload(&format!("disconnected {address}"));
            session.stream.write_all(&resp).await?;
            session.stream.flush().await?;
            return Ok(false);
        } else {
            let resp = encode_fail(&format!("no such device '{serial}'"));
            session.stream.write_all(&resp).await?;
            session.stream.flush().await?;
            return Ok(false);
        }
    }

    if let Some(host) = service.strip_prefix("connect:") {
        let response = handle_connect_service(host, registry).await;
        let resp = encode_okay_with_payload(&response);
        session.stream.write_all(&resp).await?;
        session.stream.flush().await?;
        return Ok(false);
    }

    // Transport selection handling (matching C++ handle_host_request):
    // e.g. transport, transport-any, transport-usb, transport-local, transport:<serial>, tport:...
    if service.starts_with("transport") || service.starts_with("tport:") {
        let (legacy, req_target) = if let Some(tport_spec) = service.strip_prefix("tport:") {
            if let Some(serial) = tport_spec.strip_prefix("serial:") {
                (false, TransportTarget::Serial(serial.to_string()))
            } else if tport_spec == "usb" {
                (false, TransportTarget::Usb)
            } else if tport_spec == "local" {
                (false, TransportTarget::Local)
            } else {
                (false, TransportTarget::Any)
            }
        } else if let Some(serial) = service.strip_prefix("transport:") {
            (true, TransportTarget::Serial(serial.to_string()))
        } else if let Some(id_str) = service.strip_prefix("transport-id:") {
            if let Ok(id) = id_str.parse::<u64>() {
                (true, TransportTarget::TransportId(id))
            } else {
                session
                    .stream
                    .write_all(&encode_fail("invalid transport id"))
                    .await?;
                session.stream.flush().await?;
                return Ok(false);
            }
        } else if service == "transport-usb" {
            (true, TransportTarget::Usb)
        } else if service == "transport-local" {
            (true, TransportTarget::Local)
        } else {
            (true, target.clone())
        };

        match acquire_one_transport(registry, &req_target, true) {
            Ok(dev) => {
                session.selected_device = Some(dev.transport_id);
                session.stream.write_all(encode_okay()).await?;
                if !legacy {
                    let id_bytes = dev.transport_id.to_le_bytes();
                    session.stream.write_all(&id_bytes).await?;
                }
                session.stream.flush().await?;
                return Ok(true);
            }
            Err(err_msg) => {
                session.stream.write_all(&encode_fail(&err_msg)).await?;
                session.stream.flush().await?;
                return Ok(false);
            }
        }
    }

    if service == "get-serialno" {
        let effective_target = if let Some(id) = session.selected_device {
            TransportTarget::TransportId(id)
        } else {
            target.clone()
        };
        match acquire_one_transport(registry, &effective_target, false) {
            Ok(t) => {
                let serial_resp = if !t.serial.is_empty() {
                    &t.serial
                } else {
                    "unknown"
                };
                let resp = encode_okay_with_payload(serial_resp);
                session.stream.write_all(&resp).await?;
                session.stream.flush().await?;
            }
            Err(err) => {
                session.stream.write_all(&encode_fail(&err)).await?;
                session.stream.flush().await?;
            }
        }
        return Ok(false);
    }

    if service == "get-devpath" {
        let effective_target = if let Some(id) = session.selected_device {
            TransportTarget::TransportId(id)
        } else {
            target.clone()
        };
        match acquire_one_transport(registry, &effective_target, false) {
            Ok(t) => {
                let devpath_resp = t.devpath.as_deref().unwrap_or("unknown");
                let resp = encode_okay_with_payload(devpath_resp);
                session.stream.write_all(&resp).await?;
                session.stream.flush().await?;
            }
            Err(err) => {
                session.stream.write_all(&encode_fail(&err)).await?;
                session.stream.flush().await?;
            }
        }
        return Ok(false);
    }

    if service == "get-state" {
        let effective_target = if let Some(id) = session.selected_device {
            TransportTarget::TransportId(id)
        } else {
            target.clone()
        };
        match acquire_one_transport(registry, &effective_target, false) {
            Ok(t) => {
                let resp = encode_okay_with_payload(t.state.as_str());
                session.stream.write_all(&resp).await?;
                session.stream.flush().await?;
            }
            Err(err) => {
                session.stream.write_all(&encode_fail(&err)).await?;
                session.stream.flush().await?;
            }
        }
        return Ok(false);
    }

    if service == "features" {
        let effective_target = if let Some(id) = session.selected_device {
            TransportTarget::TransportId(id)
        } else {
            target.clone()
        };
        match acquire_one_transport(registry, &effective_target, false) {
            Ok(t) => {
                let features_str = t.features.join(",");
                let resp = encode_okay_with_payload(&features_str);
                session.stream.write_all(&resp).await?;
                session.stream.flush().await?;
            }
            Err(err) => {
                session.stream.write_all(&encode_fail(&err)).await?;
                session.stream.flush().await?;
            }
        }
        return Ok(false);
    }

    // Forwarding request handling (matching C++ handle_forward_request in original/adb.cpp):
    // list-forward
    if service == "list-forward" {
        let listeners = listener_manager.format_listeners();
        let resp = encode_okay_with_payload(&listeners);
        session.stream.write_all(&resp).await?;
        session.stream.flush().await?;
        return Ok(false);
    }

    // killforward-all
    if service == "killforward-all" {
        listener_manager.remove_all_listeners();
        // C++: 1st OKAY is connect, 2nd OKAY is status
        let mut resp = Vec::new();
        resp.extend_from_slice(encode_okay());
        resp.extend_from_slice(encode_okay());
        session.stream.write_all(&resp).await?;
        session.stream.flush().await?;
        return Ok(false);
    }

    // forward: or killforward:
    if service.starts_with("forward:") || service.starts_with("killforward:") {
        let effective_target = if let Some(id) = session.selected_device {
            TransportTarget::TransportId(id)
        } else {
            target.clone()
        };

        let transport = match acquire_one_transport(registry, &effective_target, false) {
            Ok(t) => t,
            Err(err) => {
                session.stream.write_all(&encode_fail(&err)).await?;
                session.stream.flush().await?;
                return Ok(false);
            }
        };

        if let Some(local) = service.strip_prefix("killforward:") {
            if local.is_empty() || local.contains(';') {
                let err_msg = format!("bad killforward: {service}");
                session.stream.write_all(&encode_fail(&err_msg)).await?;
                session.stream.flush().await?;
                return Ok(false);
            }

            let status = listener_manager.remove_listener(local);
            if status == InstallStatus::Ok {
                let mut resp = Vec::new();
                resp.extend_from_slice(encode_okay());
                resp.extend_from_slice(encode_okay());
                session.stream.write_all(&resp).await?;
                session.stream.flush().await?;
            } else {
                let err_msg = format!("listener '{local}' not found");
                session.stream.write_all(&encode_fail(&err_msg)).await?;
                session.stream.flush().await?;
            }
            return Ok(false);
        }

        if let Some(forward_spec) = service.strip_prefix("forward:") {
            let (no_rebind, fwd_args) =
                if let Some(norebind_args) = forward_spec.strip_prefix("norebind:") {
                    (true, norebind_args)
                } else {
                    (false, forward_spec)
                };

            let pieces: Vec<&str> = fwd_args.split(';').collect();
            if pieces.len() != 2
                || pieces[0].is_empty()
                || pieces[1].is_empty()
                || pieces[1].starts_with('*')
            {
                let err_msg = format!("bad forward: {service}");
                session.stream.write_all(&encode_fail(&err_msg)).await?;
                session.stream.flush().await?;
                return Ok(false);
            }

            let mut flags = 0;
            if no_rebind {
                flags |= INSTALL_LISTENER_NO_REBIND;
            }

            let (status, resolved_port, err) = listener_manager
                .install_listener(pieces[0], pieces[1], Some(transport), flags)
                .await;

            if status == InstallStatus::Ok {
                let mut resp = Vec::new();
                resp.extend_from_slice(encode_okay());
                resp.extend_from_slice(encode_okay());
                if let Some(port) = resolved_port {
                    let port_str = format!("{port}");
                    let hex_len = format!("{:04x}", port_str.len());
                    resp.extend_from_slice(hex_len.as_bytes());
                    resp.extend_from_slice(port_str.as_bytes());
                }
                session.stream.write_all(&resp).await?;
                session.stream.flush().await?;
            } else {
                let msg = match status {
                    InstallStatus::Ok => "success (!)".to_string(),
                    InstallStatus::InternalError => "internal error".to_string(),
                    InstallStatus::CannotBind => format!("cannot bind listener: {err}"),
                    InstallStatus::CannotRebind => "cannot rebind existing socket".to_string(),
                    InstallStatus::ListenerNotFound => format!("listener '{service}' not found"),
                };
                session.stream.write_all(&encode_fail(&msg)).await?;
                session.stream.flush().await?;
            }
            return Ok(false);
        }
    }

    // If service is a target device service (e.g. shell, sync, exec, etc.)
    // routed via host prefix like `host-serial:XYZ:shell:...`
    if service.starts_with("shell")
        || service.starts_with("sync:")
        || service.starts_with("exec:")
        || service.starts_with("abb:")
        || service.starts_with("abb_exec:")
        || service.starts_with("remount:")
        || service.starts_with("dev:")
        || service.starts_with("dev-raw:")
    {
        let effective_target = if let Some(id) = session.selected_device {
            TransportTarget::TransportId(id)
        } else {
            target.clone()
        };
        let dev = match acquire_one_transport(registry, &effective_target, false) {
            Ok(d) => d,
            Err(err) => {
                session.stream.write_all(&encode_fail(&err)).await?;
                session.stream.flush().await?;
                return Ok(false);
            }
        };

        session.selected_device = Some(dev.transport_id);
        handle_device_service(session, service).await?;
        return Ok(false);
    }

    // Unrecognized or out-of-scope host service:
    let fail_msg = format!("unknown host service '{service}'");
    session.stream.write_all(&encode_fail(&fail_msg)).await?;
    session.stream.flush().await?;
    Ok(false)
}

/// Ports `acquire_one_transport` from `original/transport.cpp`.
pub fn acquire_one_transport(
    registry: &DeviceRegistry,
    target: &TransportTarget,
    accept_any_state: bool,
) -> Result<Arc<DeviceEntry>, String> {
    let mut default_error = match target {
        TransportTarget::TransportId(id) => format!("no device with transport id '{id}'"),
        TransportTarget::Serial(ref s) => format!("device '{s}' not found"),
        TransportTarget::Local => "no emulators found".to_string(),
        TransportTarget::Any => "no devices/emulators found".to_string(),
        TransportTarget::Usb => "no devices found".to_string(),
    };

    let all_devices = registry.list_devices();
    let mut result: Option<Arc<DeviceEntry>> = None;

    for t in all_devices {
        if t.state == crate::state::ConnectionState::NoPerm {
            default_error = "insufficient permissions for device".to_string();
            continue;
        }

        match target {
            TransportTarget::TransportId(id) => {
                if t.transport_id == *id {
                    result = Some(t);
                    break;
                }
            }
            TransportTarget::Serial(ref s) => {
                if t.matches_target(s) {
                    if result.is_some() {
                        return Err(format!("more than one device with serial {s}"));
                    }
                    result = Some(t);
                }
            }
            TransportTarget::Usb => {
                if t.transport_type == TransportType::Usb {
                    if result.is_some() {
                        return Err("more than one device".to_string());
                    }
                    result = Some(t);
                }
            }
            TransportTarget::Local => {
                if t.transport_type == TransportType::Local {
                    if result.is_some() {
                        return Err("more than one emulator".to_string());
                    }
                    result = Some(t);
                }
            }
            TransportTarget::Any => {
                if result.is_some() {
                    return Err("more than one device/emulator".to_string());
                }
                result = Some(t);
            }
        }
    }

    let dev = match result {
        Some(d) => d,
        None => return Err(default_error),
    };

    if !accept_any_state {
        match dev.state {
            crate::state::ConnectionState::Connecting => {
                return Err("device still connecting".to_string())
            }
            crate::state::ConnectionState::Authorizing => {
                return Err("device still authorizing".to_string())
            }
            crate::state::ConnectionState::Unauthorized => {
                let mut err =
                    "device unauthorized.\nThis adb server's $ADB_VENDOR_KEYS is ".to_string();
                if let Ok(keys) = std::env::var("ADB_VENDOR_KEYS") {
                    err.push_str(&keys);
                } else {
                    err.push_str("not set");
                }
                err.push_str("\nTry 'adb kill-server' if that seems wrong.\nOtherwise check for a confirmation dialog on your device.");
                return Err(err);
            }
            crate::state::ConnectionState::Offline => return Err("device offline".to_string()),
            _ => {}
        }
    }

    Ok(dev)
}

/// Parses network address to canonical serial and error matching C++ ParseNetAddress.
fn parse_net_address_canonical(address: &str) -> (String, Option<String>) {
    if address.starts_with("vsock:") || address.starts_with("localfilesystem:") {
        return (address.to_string(), None);
    }

    let colons = address.chars().filter(|&c| c == ':').count();
    let dots = address.chars().filter(|&c| c == '.').count();

    let (host, port_val) = if address.starts_with('[') {
        if let Some(end_bracket) = address.find("]:") {
            let h = &address[1..end_bracket];
            let p_str = &address[end_bracket + 2..];
            match p_str.parse::<u16>() {
                Ok(p) if p > 0 => (h, p),
                _ => {
                    return (
                        String::new(),
                        Some(format!("bad port number '{p_str}' in '{address}'")),
                    );
                }
            }
        } else {
            return (String::new(), Some(format!("bad IPv6 address '{address}'")));
        }
    } else if dots == 0 && (2..=7).contains(&colons) {
        (address, DEFAULT_ADB_LOCAL_TRANSPORT_PORT)
    } else if colons <= 1 {
        let parts: Vec<&str> = address.split(':').collect();
        let h = parts[0];
        if parts.len() > 1 {
            let p_str = parts[1];
            match p_str.parse::<u16>() {
                Ok(p) if p > 0 => (h, p),
                _ => {
                    return (
                        String::new(),
                        Some(format!("bad port number '{p_str}' in '{address}'")),
                    );
                }
            }
        } else {
            (h, DEFAULT_ADB_LOCAL_TRANSPORT_PORT)
        }
    } else {
        return (
            String::new(),
            Some(format!("could not parse network address '{address}'")),
        );
    };

    if host.is_empty() {
        return (String::new(), Some(format!("no host in '{address}'")));
    }

    let is_ipv6 = dots == 0 && (2..=7).contains(&colons);
    let canonical = if is_ipv6 {
        format!("[{host}]:{port_val}")
    } else {
        format!("{host}:{port_val}")
    };

    (canonical, None)
}

/// Ports `connect_service` from `original/services.cpp` and `original/client/transport_emulator.cpp`.
async fn handle_connect_service(host: &str, registry: &Arc<DeviceRegistry>) -> String {
    if host.is_empty() {
        return "empty address".to_string();
    }

    if let Some(port_spec) = host.strip_prefix("emu:") {
        let pieces: Vec<&str> = port_spec.split(',').collect();
        if pieces.len() != 2 {
            return format!("unable to parse '{port_spec}' as <console port>,<adb port>");
        }
        let console_port = pieces[0].parse::<i32>().unwrap_or(-1);
        let adb_port = pieces[1].parse::<i32>().unwrap_or(-1);
        if console_port <= 0 || adb_port <= 0 {
            return format!("Invalid port numbers: {port_spec}");
        }
        let serial = format!("emulator-{console_port}");
        if registry.get_by_serial(&serial).is_some() {
            return format!("Emulator already registered on port {adb_port}");
        }

        // Try loopback connection
        let target_addr = format!("127.0.0.1:{adb_port}");
        match tokio::net::TcpStream::connect(&target_addr).await {
            Ok(_stream) => {
                let _ = registry.register(&serial, TransportType::Local);
                format!("Connected to emulator on ports {console_port},{adb_port}")
            }
            Err(e) => {
                format!("Could not connect to emulator on ports {console_port},{adb_port}: failed to connect to '127.0.0.1:{adb_port}': {e}")
            }
        }
    } else {
        let (serial, parse_err) = parse_net_address_canonical(host);
        if let Some(err) = parse_err {
            return err;
        }

        if registry.get_by_serial(&serial).is_some() {
            return format!("already connected to {serial}");
        }

        match tokio::net::TcpStream::connect(&serial).await {
            Ok(_stream) => {
                let _ = registry.register(&serial, TransportType::Local);
                format!("connected to {serial}")
            }
            Err(e) => {
                format!("failed to connect to '{serial}': {e}")
            }
        }
    }
}

async fn handle_device_service(
    session: &mut ClientSession,
    _service: &str,
) -> Result<(), std::io::Error> {
    if session.selected_device.is_none() {
        // C++ ADB: SendFail(s->peer->fd, "device offline (no transport)");
        session
            .stream
            .write_all(&encode_fail("device offline (no transport)"))
            .await?;
        session.stream.flush().await?;
        return Ok(());
    }

    // Device is selected: route stream to device
    // (For task 5.6a router core, confirm routing with OKAY or stream connection)
    session.stream.write_all(encode_okay()).await?;
    session.stream.flush().await?;
    Ok(())
}
