use crate::server_address::ServerAddress;
use crate::session::{Session, SessionError};
use crate::transport::{format_host_command, TransportSelector};
use adb_proto::{ShellPacket, ShellPacketId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Escapes an argument string by enclosing it in single quotes and replacing any single quote
/// with `'\''`.
///
/// Ports `escape_arg` from `original/adb_utils.cpp`.
pub fn escape_arg(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Options for non-interactive device shell execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellOptions {
    /// -n: don't read from stdin (send close-stdin immediately in shell v2).
    pub no_stdin: bool,
    /// -x: disable remote exit codes and stdout/stderr separation (force v1 protocol).
    pub disable_shell_protocol: bool,
    /// -T: disable PTY allocation (force raw mode).
    pub no_pty: bool,
    /// Terminal type passed as `TERM=<value>` parameter in shell v2 protocol.
    pub term: Option<String>,
}

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

/// Expected ADB server version matching C++ `ADB_SERVER_VERSION` in `original/adb.h`.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Events reported by client operations (e.g. version checks, server startup, kill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    ServerVersionMismatch { server: u32, client: u32 },
    DaemonNotRunning { spec: String },
    CannotStartRemote,
    CannotConnectDaemon { spec: String, cause: String },
    Error(String),
    FailedToStartDaemon,
    DaemonStarted,
}

impl std::fmt::Display for ClientEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientEvent::ServerVersionMismatch { server, client } => {
                write!(
                    f,
                    "adb server version ({server}) doesn't match this client ({client}); killing..."
                )
            }
            ClientEvent::DaemonNotRunning { spec } => {
                write!(f, "* daemon not running; starting now at {spec}")
            }
            ClientEvent::CannotStartRemote => {
                write!(f, "* cannot start server on remote host")
            }
            ClientEvent::CannotConnectDaemon { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            ClientEvent::Error(msg) => {
                if msg.starts_with("error:") || msg.starts_with("cannot connect") {
                    write!(f, "{msg}")
                } else {
                    write!(f, "error: {msg}")
                }
            }
            ClientEvent::FailedToStartDaemon => {
                write!(f, "* failed to start daemon")
            }
            ClientEvent::DaemonStarted => {
                write!(f, "* daemon started successfully")
            }
        }
    }
}

/// Result of an `AdbClient` operation paired with events emitted during version check or daemon management.
#[derive(Debug)]
pub struct Outcome<T> {
    pub result: Result<T, ClientError>,
    pub events: Vec<ClientEvent>,
}

/// Errors that can occur during high-level client operations.
#[derive(Debug, Clone)]
pub enum ClientError {
    Session(SessionError),
    CannotConnect { spec: String, cause: String },
    CannotConnectDaemon,
    CannotParseVersionString(String),
    KillFailed(String),
    FailedToCheckVersion(String),
    QueryFailed(String),
    InvalidUsage(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Session(err) => {
                let msg = err.to_string();
                if msg.starts_with("error:")
                    || msg.starts_with("cannot connect")
                    || msg.starts_with("protocol fault")
                    || msg.starts_with("bad service")
                    || msg.starts_with("write failure")
                {
                    write!(f, "{msg}")
                } else {
                    write!(f, "error: {msg}")
                }
            }
            ClientError::CannotConnect { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            ClientError::CannotConnectDaemon => {
                write!(f, "cannot connect to daemon")
            }
            ClientError::CannotParseVersionString(ver) => {
                write!(f, "cannot parse version string: {ver}")
            }
            ClientError::KillFailed(msg) => write!(f, "{msg}"),
            ClientError::FailedToCheckVersion(err) => {
                write!(f, "failed to check server version: {err}")
            }
            ClientError::QueryFailed(msg) => {
                if msg.starts_with("error:") {
                    write!(f, "{msg}")
                } else {
                    write!(f, "error: {msg}")
                }
            }
            ClientError::InvalidUsage(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Session(err) => Some(err),
            _ => None,
        }
    }
}

impl From<SessionError> for ClientError {
    fn from(err: SessionError) -> Self {
        ClientError::Session(err)
    }
}

/// Errors that can occur during ADB server process launching.
#[derive(Debug)]
pub enum LaunchError {
    Io(std::io::Error),
    FailedToReadAck(String),
    Unsupported(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Io(err) => write!(f, "IO error: {err}"),
            LaunchError::FailedToReadAck(msg) => write!(f, "{msg}"),
            LaunchError::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LaunchError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for LaunchError {
    fn from(err: std::io::Error) -> Self {
        LaunchError::Io(err)
    }
}

/// Trait for creating sessions to an ADB server address.
pub trait SessionFactory: Send + Sync {
    fn connect<'a>(
        &'a self,
        addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<Session, SessionError>> + Send + 'a>>;
}

/// Default implementation of `SessionFactory` using `Session::connect`.
pub struct DefaultSessionFactory;

impl SessionFactory for DefaultSessionFactory {
    fn connect<'a>(
        &'a self,
        addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<Session, SessionError>> + Send + 'a>> {
        Box::pin(async move { Session::connect(addr).await })
    }
}

/// Trait for launching an ADB server process.
pub trait ServerLauncher: Send + Sync {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>>;
}

/// Launches the server process by executing `std::env::current_exe()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SelfExecLauncher;

impl ServerLauncher for SelfExecLauncher {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move {
            let exe = std::env::current_exe()?;
            launch_binary(&exe, spec, one_device).await
        })
    }
}

/// Launches the server process by executing a specific binary path.
#[derive(Debug, Clone)]
pub struct ExternalBinaryLauncher {
    path: std::path::PathBuf,
}

impl ExternalBinaryLauncher {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl ServerLauncher for ExternalBinaryLauncher {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move { launch_binary(&self.path, spec, one_device).await })
    }
}

/// Spawns a background ADB server binary and awaits the reply acknowledgment.
///
/// Cites C++ `launch_server` in `original/adb.cpp` and `adb_server_main` in `original/client/main.cpp`.
#[cfg(unix)]
async fn launch_binary(
    path: &std::path::Path,
    spec: &str,
    one_device: Option<&str>,
) -> Result<(), LaunchError> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use tokio::io::AsyncReadExt;

    // Set up a pipe so the child process can acknowledge readiness ("OK\n").
    // See original/adb.cpp: launch_server()
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

    let write_fd_raw = write_fd.as_raw_fd();

    let mut cmd = tokio::process::Command::new(path);
    cmd.arg("-L")
        .arg(spec)
        .arg("fork-server")
        .arg("server")
        .arg("--reply-fd")
        .arg(write_fd_raw.to_string());

    if let Some(device) = one_device {
        cmd.arg("--one-device").arg(device);
    }

    unsafe {
        cmd.pre_exec(move || {
            // Detach process into its own session and process group.
            // See original/adb.cpp: launch_server()
            libc::setsid();

            // Clear FD_CLOEXEC on write_fd so it is inherited by exec.
            // See original/adb.cpp: launch_server() where fcntl(pipe_write.get(), F_SETFD, 0) is called.
            let flags = libc::fcntl(write_fd_raw, libc::F_GETFD);
            if flags != -1 {
                libc::fcntl(write_fd_raw, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;

    // Explicitly drop write_fd in parent so pipe closure signals EOF if child terminates early.
    drop(write_fd);

    let read_file = std::fs::File::from(read_fd);
    let mut async_read = tokio::fs::File::from_std(read_file);

    let mut buf = [0u8; 3];
    let read_res = async_read.read_exact(&mut buf).await;

    match read_res {
        Ok(3) if &buf == b"OK\n" => Ok(()),
        _ => {
            let _ = child.kill().await;
            Err(LaunchError::FailedToReadAck(
                "could not read ok from ADB Server".to_string(),
            ))
        }
    }
}

#[cfg(not(unix))]
async fn launch_binary(
    _path: &std::path::Path,
    _spec: &str,
    _one_device: Option<&str>,
) -> Result<(), LaunchError> {
    Err(LaunchError::Unsupported(
        "Windows server launching is not supported yet".to_string(),
    ))
}

/// Kills the running ADB server at `addr`.
///
/// Ports `adb_kill_server` from C++ `original/client/adb_client.cpp`.
pub async fn kill_server(
    addr: &ServerAddress,
    session_factory: &dyn SessionFactory,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    let mut session = match session_factory.connect(addr).await {
        Ok(s) => s,
        Err(err) => {
            let cause = match &err {
                SessionError::CannotConnect { cause, .. } => cause.clone(),
                _ => err.to_string(),
            };
            events.push(ClientEvent::CannotConnectDaemon {
                spec: addr.to_spec(),
                cause,
            });
            return Ok(());
        }
    };

    if let Err(err) = session.send_service("host:kill").await {
        let msg = format!("write failure during connection: {err}");
        events.push(ClientEvent::Error(msg.clone()));
        return Err(ClientError::KillFailed(msg));
    }

    match session.read_status().await {
        Ok(()) => {
            let _ = session.read_to_close().await;
            Ok(())
        }
        Err(SessionError::Fail(reason)) => {
            events.push(ClientEvent::Error(reason.clone()));
            Err(ClientError::KillFailed(reason))
        }
        Err(_) => {
            let msg = "failed to read response from server".to_string();
            events.push(ClientEvent::Error(msg.clone()));
            Err(ClientError::KillFailed(msg))
        }
    }
}

/// Launches a local ADB server for `addr`.
///
/// Ports `launch_server` invocation from `adb_connect` in C++ `original/client/adb_client.cpp`.
pub async fn start_server(
    addr: &ServerAddress,
    launcher: &dyn ServerLauncher,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    start_server_with_one_device(addr, None, launcher, events).await
}

/// Launches a local ADB server for `addr` with an optional `one_device` requirement.
pub async fn start_server_with_one_device(
    addr: &ServerAddress,
    one_device: Option<&str>,
    launcher: &dyn ServerLauncher,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    match launcher.launch(&addr.to_spec(), one_device).await {
        Ok(()) => {
            events.push(ClientEvent::DaemonStarted);
            Ok(())
        }
        Err(_err) => {
            events.push(ClientEvent::FailedToStartDaemon);
            Err(ClientError::CannotConnectDaemon)
        }
    }
}

/// Checks that the running server version matches `ADB_SERVER_VERSION`.
///
/// Ports `__adb_check_server_version` and `adb_connect` from C++ `original/client/adb_client.cpp`.
pub async fn check_server_version(
    addr: &ServerAddress,
    session_factory: &dyn SessionFactory,
    launcher: &dyn ServerLauncher,
    events: &mut Vec<ClientEvent>,
) -> Result<(), ClientError> {
    let connect_res = session_factory.connect(addr).await;
    let local = addr.is_local();

    let mut session = match connect_res {
        Err(SessionError::CannotConnect { cause, .. }) => {
            if !local {
                events.push(ClientEvent::CannotStartRemote);
                return Err(SessionError::CannotConnect {
                    spec: addr.to_spec(),
                    cause,
                }
                .into());
            } else {
                events.push(ClientEvent::DaemonNotRunning {
                    spec: addr.to_spec(),
                });
                return start_server(addr, launcher, events).await;
            }
        }
        Err(err) => return Err(err.into()),
        Ok(s) => s,
    };

    if let Err(err) = session.send_service("host:version").await {
        return Err(err.into());
    }

    let version = match session.read_status().await {
        Ok(()) => {
            let version_string = session.read_string().await?;
            let _ = session.read_to_close().await;

            match u32::from_str_radix(&version_string, 16) {
                Ok(v) => v,
                Err(_) => {
                    return Err(ClientError::CannotParseVersionString(version_string));
                }
            }
        }
        Err(SessionError::Fail(reason)) if reason == "unknown host service" => 0,
        Err(err) => return Err(err.into()),
    };

    if version != ADB_SERVER_VERSION {
        events.push(ClientEvent::ServerVersionMismatch {
            server: version,
            client: ADB_SERVER_VERSION,
        });

        kill_server(addr, session_factory, events).await?;
        return start_server(addr, launcher, events).await;
    }

    Ok(())
}

/// Builder for constructing an `AdbClient`.
pub struct AdbClientBuilder {
    addr: ServerAddress,
    session_factory: Arc<dyn SessionFactory>,
    launcher: Arc<dyn ServerLauncher>,
}

impl AdbClientBuilder {
    pub fn new(addr: ServerAddress) -> Self {
        Self {
            addr,
            session_factory: Arc::new(DefaultSessionFactory),
            launcher: Arc::new(SelfExecLauncher),
        }
    }

    pub fn session_factory(mut self, factory: impl SessionFactory + 'static) -> Self {
        self.session_factory = Arc::new(factory);
        self
    }

    pub fn launcher(mut self, launcher: impl ServerLauncher + 'static) -> Self {
        self.launcher = Arc::new(launcher);
        self
    }

    pub fn build(self) -> AdbClient {
        AdbClient {
            addr: self.addr,
            session_factory: self.session_factory,
            launcher: self.launcher,
            version_checked: tokio::sync::OnceCell::new(),
        }
    }
}

struct VersionCheckState {
    result: Result<(), ClientError>,
    events: std::sync::Mutex<Vec<ClientEvent>>,
}

/// Represents an ADB client instance associated with a server address.
pub struct AdbClient {
    addr: ServerAddress,
    session_factory: Arc<dyn SessionFactory>,
    launcher: Arc<dyn ServerLauncher>,
    version_checked: tokio::sync::OnceCell<VersionCheckState>,
}

impl AdbClient {
    /// Returns a builder for configuring and creating an `AdbClient`.
    pub fn builder(addr: ServerAddress) -> AdbClientBuilder {
        AdbClientBuilder::new(addr)
    }

    /// Creates a new `AdbClient` targeting `addr` with default session factory and launcher.
    pub fn new(addr: ServerAddress) -> Self {
        Self::builder(addr).build()
    }

    /// Returns a reference to the server address.
    pub fn addr(&self) -> &ServerAddress {
        &self.addr
    }

    /// Ensures the server is running and version matches before executing host operations.
    async fn ensure_server(&self) -> Outcome<()> {
        let state = self
            .version_checked
            .get_or_init(|| async {
                let mut check_events = Vec::new();
                let res = check_server_version(
                    &self.addr,
                    self.session_factory.as_ref(),
                    self.launcher.as_ref(),
                    &mut check_events,
                )
                .await;
                VersionCheckState {
                    result: res,
                    events: std::sync::Mutex::new(check_events),
                }
            })
            .await;

        let events = {
            let mut guard = state.events.lock().unwrap();
            std::mem::take(&mut *guard)
        };

        Outcome {
            result: state.result.clone(),
            events,
        }
    }

    /// Checks the server version once per `AdbClient` instance.
    pub async fn check_server_version(&self) -> Outcome<()> {
        self.ensure_server().await
    }

    /// Kills the ADB server for this client instance.
    pub async fn kill_server(&self) -> Outcome<()> {
        let mut events = Vec::new();
        let result = kill_server(&self.addr, self.session_factory.as_ref(), &mut events).await;
        Outcome { result, events }
    }

    /// Queries connected devices list (`host:devices` or `host:devices-l`).
    ///
    /// Ports `devices` command handling in `original/client/commandline.cpp`.
    /// Checks server version first, then prepends `"List of devices attached\n"`.
    pub async fn devices(&self, long: bool) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let service = if long {
            "host:devices-l"
        } else {
            "host:devices"
        };

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(format!("List of devices attached\n{payload}\n"))
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Validates arguments for `devices` and executes query.
    pub async fn devices_with_args(&self, args: &[&str]) -> Outcome<String> {
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
        self.devices(long).await
    }

    /// Executes a general host query command formatted with transport selector.
    ///
    /// Ports `adb_query_command(format_host_command(...))` from `commandline.cpp`.
    pub async fn query_command(
        &self,
        selector: &TransportSelector,
        command: &str,
    ) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let service = format_host_command(command, selector);
        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(format!("{payload}\n"))
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Queries the target device state (`get-state`).
    pub async fn get_state(&self, selector: &TransportSelector) -> Outcome<String> {
        self.query_command(selector, "get-state").await
    }

    /// Queries the target device serial number (`get-serialno`).
    pub async fn get_serialno(&self, selector: &TransportSelector) -> Outcome<String> {
        self.query_command(selector, "get-serialno").await
    }

    /// Queries the target device path (`get-devpath`).
    pub async fn get_devpath(&self, selector: &TransportSelector) -> Outcome<String> {
        self.query_command(selector, "get-devpath").await
    }

    /// Queries the target device transport ID (`transport-id`).
    ///
    /// Asks `host:features` with a forced device transport switch, returning the decimal transport ID.
    /// Ports `transport-id` handling from `original/client/commandline.cpp`.
    pub async fn transport_id(&self, selector: &TransportSelector) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            let transport_id = session.select_transport(selector).await?.unwrap_or(0);
            session.send_service("host:features").await?;
            session.read_status().await?;
            Ok(format!("{transport_id}\n"))
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Queries target device features and filters against client supported features (`features`).
    ///
    /// Ports `features` handling from `original/client/commandline.cpp`.
    pub async fn features(&self, selector: &TransportSelector) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let service = format_host_command("features", selector);
        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;

            let device_features = adb_proto::Features::parse(&payload);
            let client_features = adb_proto::supported_features();

            let mut output = String::new();
            for feature in device_features.items() {
                if client_features.contains(feature) {
                    output.push_str(feature.as_str());
                    output.push('\n');
                }
            }
            Ok(output)
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Queries server host features (`host-features`).
    ///
    /// Ports `host-features` handling from `original/client/commandline.cpp`.
    pub async fn host_features(&self) -> Outcome<String> {
        self.query_command(&TransportSelector::Any, "host-features")
            .await
    }

    /// Connects to a device via TCP/IP (`host:connect:<target>`).
    ///
    /// Ports `connect` command handling from `original/client/commandline.cpp`.
    /// Returns reply text on success. Returns error on failure or query refusal.
    pub async fn connect(&self, target: &str) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let service = format!("host:connect:{target}");
        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(payload)
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Validates arguments for `connect` and executes command.
    pub async fn connect_with_args(&self, args: &[&str]) -> Outcome<String> {
        match args {
            [target] => self.connect(target).await,
            _ => Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb connect HOST[:PORT]".to_string(),
                )),
                events: Vec::new(),
            },
        }
    }

    /// Disconnects from a TCP/IP device (`host:disconnect:<target or empty>`).
    ///
    /// Ports `disconnect` command handling from `original/client/commandline.cpp`.
    /// If `target` is `None`, disconnects all TCP/IP devices.
    pub async fn disconnect(&self, target: Option<&str>) -> Outcome<String> {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let service = format!("host:disconnect:{}", target.unwrap_or(""));
        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(payload)
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Validates arguments for `disconnect` and executes command.
    pub async fn disconnect_with_args(&self, args: &[&str]) -> Outcome<String> {
        match args {
            [] => self.disconnect(None).await,
            [target] => self.disconnect(Some(target)).await,
            _ => Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb disconnect [HOST[:PORT]]".to_string(),
                )),
                events: Vec::new(),
            },
        }
    }

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

    /// Validates arguments for `track-devices` and returns tracking stream.
    pub async fn track_devices_with_args(
        &self,
        args: &[&str],
    ) -> Outcome<Pin<Box<dyn tokio_stream::Stream<Item = Result<bytes::Bytes, ClientError>> + Send>>>
    {
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
        self.track_devices(mode).await
    }

    /// Executes a single command on a device via shell protocol in non-interactive mode.
    ///
    /// Ports `adb_shell` and `RemoteShell` from C++ `original/client/commandline.cpp`.
    /// Reads stdout and stderr bytes, streams stdin data if provided (unless `options.no_stdin` is true),
    /// and returns the command exit code.
    pub async fn shell<R, W1, W2>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        options: &ShellOptions,
        stdin: Option<R>,
        stdout: Option<W1>,
        stderr: Option<W2>,
    ) -> Outcome<i32>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W1: AsyncWrite + Unpin + Send,
        W2: AsyncWrite + Unpin + Send,
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        // 1. Query device features to check if shell_v2 (kFeatureShell2) is supported
        let features_outcome = self.features(selector).await;
        let features_str = match features_outcome.result {
            Ok(s) => s,
            Err(e) => {
                return Outcome {
                    result: Err(e),
                    events: outcome.events,
                }
            }
        };

        let device_supports_v2 = features_str.lines().any(|l| l == "shell_v2");
        let use_shell_protocol = device_supports_v2 && !options.disable_shell_protocol;

        let mut is_raw = options.no_pty;
        if use_shell_protocol {
            // In non-interactive mode with v2, request raw shell type
            is_raw = true;
        }

        // 2. Refusal checks for raw requests without shell_v2 support
        if !use_shell_protocol && is_raw {
            if options.disable_shell_protocol {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "error: -x only supports allocating a pty".to_string(),
                    )),
                    events: outcome.events,
                };
            }
            if !device_supports_v2 {
                return Outcome {
                    result: Err(ClientError::InvalidUsage(
                        "error: device only supports allocating a pty".to_string(),
                    )),
                    events: outcome.events,
                };
            }
        }

        let command_str = args.join(" ");

        // 3. Format service string
        let service_string = if use_shell_protocol {
            let mut v2_args = vec!["v2"];
            let term_arg;
            if let Some(term) = &options.term {
                term_arg = format!("TERM={term}");
                v2_args.push(&term_arg);
            }
            if is_raw {
                v2_args.push("raw");
            } else {
                v2_args.push("pty");
            }
            format!("shell,{}:{}", v2_args.join(","), command_str)
        } else {
            format!("shell:{command_str}")
        };

        if !use_shell_protocol && service_string.len() > adb_proto::MAX_PAYLOAD_V1 {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "error: shell command too long".to_string(),
                )),
                events: outcome.events,
            };
        }

        // 4. Connect to device service
        let session_res = Session::connect_and_switch(&self.addr, selector, &service_string, false).await;
        let session = match session_res {
            Ok(s) => s,
            Err(e) => {
                return Outcome {
                    result: Err(e.into()),
                    events: outcome.events,
                }
            }
        };

        let res = if use_shell_protocol {
            Self::run_shell_v2(session, options.no_stdin, stdin, stdout, stderr).await
        } else {
            Self::run_shell_v1(session, options.no_stdin, stdin, stdout).await
        };

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    async fn run_shell_v2<R, W1, W2>(
        session: Session,
        no_stdin: bool,
        stdin: Option<R>,
        mut stdout: Option<W1>,
        mut stderr: Option<W2>,
    ) -> Result<i32, ClientError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W1: AsyncWrite + Unpin + Send,
        W2: AsyncWrite + Unpin + Send,
    {
        let stream = session.into_inner();
        let (reader, mut writer) = stream.into_split();

        let writer_future = async move {
            if no_stdin || stdin.is_none() {
                let pkt = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
                let _ = writer.write_all(&pkt.encode_to_vec()).await;
                let _ = writer.flush().await;
            } else if let Some(mut input) = stdin {
                let mut buf = [0u8; 16384];
                loop {
                    match input.read(&mut buf).await {
                        Ok(0) => {
                            let pkt = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
                            let _ = writer.write_all(&pkt.encode_to_vec()).await;
                            let _ = writer.flush().await;
                            break;
                        }
                        Ok(n) => {
                            let pkt = ShellPacket::new(ShellPacketId::Stdin, buf[..n].to_vec());
                            if writer.write_all(&pkt.encode_to_vec()).await.is_err() {
                                break;
                            }
                            if writer.flush().await.is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let pkt = ShellPacket::new(ShellPacketId::CloseStdin, vec![]);
                            let _ = writer.write_all(&pkt.encode_to_vec()).await;
                            let _ = writer.flush().await;
                            break;
                        }
                    }
                }
            }
        };

        let reader_future = async move {
            let mut reader = tokio::io::BufReader::new(reader);
            let mut exit_code = 255i32;
            let mut write_error = false;
            let mut header_buf = [0u8; ShellPacket::HEADER_SIZE];

            loop {
                match reader.read_exact(&mut header_buf).await {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                        break;
                    }
                    Err(e) => {
                        return Err(ClientError::Session(SessionError::ReadStatusFailed(e)));
                    }
                }

                let id_byte = header_buf[0];
                let packet_id = ShellPacketId::try_from(id_byte).map_err(|id| {
                    ClientError::QueryFailed(format!("unknown shell packet id: {id}"))
                })?;

                let mut len_bytes = [0u8; 4];
                len_bytes.copy_from_slice(&header_buf[1..5]);
                let payload_len = u32::from_le_bytes(len_bytes) as usize;

                if payload_len > adb_proto::MAX_PAYLOAD {
                    return Err(ClientError::QueryFailed(format!(
                        "shell packet length exceeds max: {payload_len}"
                    )));
                }

                let mut payload = vec![0u8; payload_len];
                if payload_len > 0 {
                    if let Err(e) = reader.read_exact(&mut payload).await {
                        return Err(ClientError::Session(SessionError::ReadStatusFailed(e)));
                    }
                }

                match packet_id {
                    ShellPacketId::Stdout => {
                        if let Some(ref mut out) = stdout {
                            if out.write_all(&payload).await.is_err() || out.flush().await.is_err()
                            {
                                write_error = true;
                                break;
                            }
                        }
                    }
                    ShellPacketId::Stderr => {
                        if let Some(ref mut err) = stderr {
                            if err.write_all(&payload).await.is_err() || err.flush().await.is_err()
                            {
                                write_error = true;
                                break;
                            }
                        }
                    }
                    ShellPacketId::Exit => {
                        if !payload.is_empty() {
                            exit_code = payload[0] as i32;
                        }
                        break;
                    }
                    _ => {}
                }
            }

            if write_error {
                return Ok(128 + 13);
            }

            Ok(exit_code)
        };

        let (reader_res, _) = tokio::join!(reader_future, writer_future);
        reader_res
    }

    async fn run_shell_v1<R, W>(
        session: Session,
        no_stdin: bool,
        stdin: Option<R>,
        mut stdout: Option<W>,
    ) -> Result<i32, ClientError>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send,
    {
        let stream = session.into_inner();
        let (mut reader, mut writer) = stream.into_split();

        let writer_future = async move {
            if !no_stdin {
                if let Some(mut input) = stdin {
                    let _ = tokio::io::copy(&mut input, &mut writer).await;
                }
            }
        };

        let reader_future = async move {
            if let Some(ref mut out) = stdout {
                let mut buf = [0u8; 16384];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            if out.write_all(&buf[..n]).await.is_err()
                                || out.flush().await.is_err()
                            {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                        Err(e) => {
                            return Err(ClientError::Session(SessionError::ReadStatusFailed(e)))
                        }
                    }
                }
            } else {
                let mut buf = [0u8; 16384];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            }
            Ok(0)
        };

        tokio::pin!(writer_future);
        tokio::pin!(reader_future);

        tokio::select! {
            res = &mut reader_future => res,
            _ = &mut writer_future => {
                reader_future.await
            }
        }
    }

    /// Runs a command on the device with stdin piped to the device (`exec-in`).
    ///
    /// Ports `exec-in` from C++ `original/client/commandline.cpp`.
    /// Takes stdin reader and streams data until EOF, returning status 0.
    pub async fn exec_in<R>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        mut stdin: R,
    ) -> Outcome<i32>
    where
        R: AsyncRead + Unpin + Send,
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        if args.is_empty() {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb exec-in command".to_string(),
                )),
                events: outcome.events,
            };
        }

        let cmd_head = args[0];
        let mut cmd_parts = vec![cmd_head.to_string()];
        for arg in &args[1..] {
            cmd_parts.push(escape_arg(arg));
        }
        let service_string = format!("exec:{}", cmd_parts.join(" "));

        let session_res = Session::connect_and_switch(&self.addr, selector, &service_string, false).await;
        let mut session = match session_res {
            Ok(s) => s,
            Err(e) => {
                return Outcome {
                    result: Err(e.into()),
                    events: outcome.events,
                }
            }
        };

        let copy_res = tokio::io::copy(&mut stdin, session.stream_mut()).await;
        let res = match copy_res {
            Ok(_) => Ok(0),
            Err(e) => Err(ClientError::Session(SessionError::WriteFailure(e))),
        };

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Runs a command on the device with stdout piped from the device (`exec-out`).
    ///
    /// Ports `exec-out` from C++ `original/client/commandline.cpp`.
    /// Takes stdout writer and streams data until EOF, returning status 0.
    pub async fn exec_out<W>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        mut stdout: W,
    ) -> Outcome<i32>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        if args.is_empty() {
            return Outcome {
                result: Err(ClientError::InvalidUsage(
                    "usage: adb exec-out command".to_string(),
                )),
                events: outcome.events,
            };
        }

        let cmd_head = args[0];
        let mut cmd_parts = vec![cmd_head.to_string()];
        for arg in &args[1..] {
            cmd_parts.push(escape_arg(arg));
        }
        let service_string = format!("exec:{}", cmd_parts.join(" "));

        let session_res = Session::connect_and_switch(&self.addr, selector, &service_string, false).await;
        let mut session = match session_res {
            Ok(s) => s,
            Err(e) => {
                return Outcome {
                    result: Err(e.into()),
                    events: outcome.events,
                }
            }
        };

        let copy_res = tokio::io::copy(session.stream_mut(), &mut stdout).await;
        let res = match copy_res {
            Ok(_) => Ok(0),
            Err(e) => Err(ClientError::Session(SessionError::ReadStatusFailed(e))),
        };

        Outcome {
            result: res,
            events: outcome.events,
        }
    }
}
