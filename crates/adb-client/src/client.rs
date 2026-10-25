use crate::server_address::ServerAddress;
use crate::session::{Session, SessionConfig, SessionError};
use crate::shell_interactive::PtyMode;
use crate::transport::{format_host_command, TransportSelector};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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

/// Escapes a command argument for the shell following standard ADB single-quote escaping rules.
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

/// Options for device shell execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOptions {
    /// -n: Close stdin immediately / do not forward stdin.
    pub close_stdin: bool,
    /// -x: Disable Shell v2 protocol and force historical Shell v1 protocol.
    pub disable_shell_protocol: bool,
    /// -T: Disable remote PTY allocation (force raw mode).
    pub no_pty: bool,
    /// -t / -tt / -T PTY mode selection.
    pub pty_mode: PtyMode,
    /// -e: Escape character (e.g. '~', None if 'none' or disabled).
    pub escape_char: Option<char>,
    /// Value for `TERM` environment variable (e.g. `xterm-256color`).
    pub term: Option<String>,
}

impl Default for ShellOptions {
    fn default() -> Self {
        Self {
            close_stdin: false,
            disable_shell_protocol: false,
            no_pty: false,
            pty_mode: PtyMode::Auto,
            escape_char: Some('~'),
            term: None,
        }
    }
}

/// Expected ADB server version matching C++ `ADB_SERVER_VERSION` in `original/adb.h`.
pub const ADB_SERVER_VERSION: u32 = 41;

/// Configuration for launching an ADB server process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchConfig {
    pub ack_timeout: std::time::Duration,
}

impl Default for LaunchConfig {
    fn default() -> Self {
        Self {
            ack_timeout: std::time::Duration::from_secs(15),
        }
    }
}

/// Classification of client errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientErrorKind {
    Server,
    Connection,
    Protocol,
    Usage,
    Version,
    Launch,
    Timeout,
}

/// Events reported by client operations (e.g. version checks, server startup, kill).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    ServerVersionMismatch { server: u32, client: u32 },
    DaemonNotRunning { spec: String },
    CannotStartRemote,
    CannotConnectDaemon { spec: String, cause: String },
    ErrorLine(String),
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
            ClientEvent::ErrorLine(msg) => {
                write!(f, "{msg}")
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
    CannotConnectDaemon { source: Option<LaunchError> },
    CannotParseVersionString(String),
    KillFailed(String),
    FailedToCheckVersion(String),
    QueryFailed(String),
    InvalidUsage(String),
    Timeout,
}

impl ClientError {
    /// Returns the classification category of this client error.
    pub fn kind(&self) -> ClientErrorKind {
        match self {
            ClientError::Session(err) => match err {
                SessionError::CannotConnect { .. } => ClientErrorKind::Connection,
                SessionError::Fail(_) => ClientErrorKind::Server,
                SessionError::Timeout => ClientErrorKind::Timeout,
                SessionError::BadServiceLength(_)
                | SessionError::WriteFailure(_)
                | SessionError::ReadTransportIdFailed
                | SessionError::ReadStatusFailed(_)
                | SessionError::ReadStatusLengthFailed(_)
                | SessionError::ReadStatusMessageFailed(_)
                | SessionError::InvalidStatus(_) => ClientErrorKind::Protocol,
            },
            ClientError::CannotConnect { .. } => ClientErrorKind::Connection,
            ClientError::CannotConnectDaemon { source } => {
                if let Some(LaunchError::Timeout(_)) = source {
                    ClientErrorKind::Timeout
                } else if source.is_some() {
                    ClientErrorKind::Launch
                } else {
                    ClientErrorKind::Connection
                }
            }
            ClientError::CannotParseVersionString(_) => ClientErrorKind::Version,
            ClientError::KillFailed(_) => ClientErrorKind::Server,
            ClientError::FailedToCheckVersion(_) => ClientErrorKind::Server,
            ClientError::QueryFailed(_) => ClientErrorKind::Server,
            ClientError::InvalidUsage(_) => ClientErrorKind::Usage,
            ClientError::Timeout => ClientErrorKind::Timeout,
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Session(err) => write!(f, "{err}"),
            ClientError::CannotConnect { spec, cause } => {
                write!(f, "cannot connect to daemon at {spec}: {cause}")
            }
            ClientError::CannotConnectDaemon { .. } => {
                write!(f, "cannot connect to daemon")
            }
            ClientError::CannotParseVersionString(ver) => {
                write!(f, "cannot parse version string: {ver}")
            }
            ClientError::KillFailed(msg) => write!(f, "{msg}"),
            ClientError::FailedToCheckVersion(err) => {
                write!(f, "failed to check server version: {err}")
            }
            ClientError::QueryFailed(msg) => write!(f, "{msg}"),
            ClientError::InvalidUsage(msg) => write!(f, "{msg}"),
            ClientError::Timeout => write!(f, "timeout"),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ClientError::Session(err) => Some(err),
            ClientError::CannotConnectDaemon { source: Some(err) } => Some(err),
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
    Timeout(String),
}

impl Clone for LaunchError {
    fn clone(&self) -> Self {
        match self {
            LaunchError::Io(err) => {
                LaunchError::Io(std::io::Error::new(err.kind(), err.to_string()))
            }
            LaunchError::FailedToReadAck(msg) => LaunchError::FailedToReadAck(msg.clone()),
            LaunchError::Unsupported(msg) => LaunchError::Unsupported(msg.clone()),
            LaunchError::Timeout(msg) => LaunchError::Timeout(msg.clone()),
        }
    }
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Io(err) => write!(f, "IO error: {err}"),
            LaunchError::FailedToReadAck(msg) => write!(f, "{msg}"),
            LaunchError::Unsupported(msg) => write!(f, "{msg}"),
            LaunchError::Timeout(msg) => write!(f, "{msg}"),
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

/// Default implementation of `SessionFactory` using `Session::connect_with_config`.
pub struct DefaultSessionFactory {
    config: SessionConfig,
}

impl DefaultSessionFactory {
    pub fn new() -> Self {
        Self {
            config: SessionConfig::default(),
        }
    }

    pub fn with_config(config: SessionConfig) -> Self {
        Self { config }
    }
}

impl Default for DefaultSessionFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionFactory for DefaultSessionFactory {
    fn connect<'a>(
        &'a self,
        addr: &'a ServerAddress,
    ) -> Pin<Box<dyn Future<Output = Result<Session, SessionError>> + Send + 'a>> {
        let config = self.config;
        Box::pin(async move { Session::connect_with_config(addr, config).await })
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
pub struct SelfExecLauncher {
    config: LaunchConfig,
}

impl SelfExecLauncher {
    pub fn new() -> Self {
        Self {
            config: LaunchConfig::default(),
        }
    }

    pub fn with_config(config: LaunchConfig) -> Self {
        Self { config }
    }
}

impl ServerLauncher for SelfExecLauncher {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        let config = self.config;
        Box::pin(async move {
            let exe = std::env::current_exe()?;
            launch_binary(&exe, spec, one_device, config).await
        })
    }
}

/// Launches the server process by executing a specific binary path.
#[derive(Debug, Clone)]
pub struct ExternalBinaryLauncher {
    path: std::path::PathBuf,
    config: LaunchConfig,
}

impl ExternalBinaryLauncher {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            config: LaunchConfig::default(),
        }
    }

    pub fn with_config(path: impl Into<std::path::PathBuf>, config: LaunchConfig) -> Self {
        Self {
            path: path.into(),
            config,
        }
    }
}

impl ServerLauncher for ExternalBinaryLauncher {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        let config = self.config;
        Box::pin(async move { launch_binary(&self.path, spec, one_device, config).await })
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
    config: LaunchConfig,
) -> Result<(), LaunchError> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::process::Stdio;
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
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .arg("-L")
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
    let read_res = tokio::time::timeout(config.ack_timeout, async_read.read_exact(&mut buf)).await;

    match read_res {
        Ok(Ok(3)) if &buf == b"OK\n" => Ok(()),
        Ok(_) => {
            let _ = child.kill().await;
            Err(LaunchError::FailedToReadAck(
                "could not read ok from ADB Server".to_string(),
            ))
        }
        Err(_) => {
            let _ = child.kill().await;
            Err(LaunchError::Timeout(
                "could not read ok from ADB Server: timed out".to_string(),
            ))
        }
    }
}

#[cfg(not(unix))]
async fn launch_binary(
    _path: &std::path::Path,
    _spec: &str,
    _one_device: Option<&str>,
    _config: LaunchConfig,
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
        events.push(ClientEvent::ErrorLine(format!("error: {msg}")));
        return Err(ClientError::KillFailed(msg));
    }

    match session.read_status().await {
        Ok(()) => {
            let _ = session.read_to_close().await;
            Ok(())
        }
        Err(SessionError::Fail(reason)) => {
            events.push(ClientEvent::ErrorLine(format!("error: {reason}")));
            Err(ClientError::KillFailed(reason))
        }
        Err(_) => {
            let msg = "failed to read response from server".to_string();
            events.push(ClientEvent::ErrorLine(format!("error: {msg}")));
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
        Err(err) => {
            events.push(ClientEvent::FailedToStartDaemon);
            Err(ClientError::CannotConnectDaemon { source: Some(err) })
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
    session_factory: Option<Arc<dyn SessionFactory>>,
    launcher: Option<Arc<dyn ServerLauncher>>,
    session_config: SessionConfig,
    launch_config: LaunchConfig,
}

impl AdbClientBuilder {
    pub fn new(addr: ServerAddress) -> Self {
        Self {
            addr,
            session_factory: None,
            launcher: None,
            session_config: SessionConfig::default(),
            launch_config: LaunchConfig::default(),
        }
    }

    pub fn session_factory(mut self, factory: impl SessionFactory + 'static) -> Self {
        self.session_factory = Some(Arc::new(factory));
        self
    }

    pub fn launcher(mut self, launcher: impl ServerLauncher + 'static) -> Self {
        self.launcher = Some(Arc::new(launcher));
        self
    }

    pub fn session_config(mut self, config: SessionConfig) -> Self {
        self.session_config = config;
        self
    }

    pub fn launch_config(mut self, config: LaunchConfig) -> Self {
        self.launch_config = config;
        self
    }

    pub fn build(self) -> AdbClient {
        let session_factory = self
            .session_factory
            .unwrap_or_else(|| Arc::new(DefaultSessionFactory::with_config(self.session_config)));
        let launcher = self
            .launcher
            .unwrap_or_else(|| Arc::new(SelfExecLauncher::with_config(self.launch_config)));

        AdbClient {
            addr: self.addr,
            session_factory,
            launcher,
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

    /// Returns a reference to the session factory.
    pub fn session_factory(&self) -> &Arc<dyn SessionFactory> {
        &self.session_factory
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
            let burst = std::env::var("ADB_BURST_MODE").is_ok_and(|v| v == "1");
            let options = adb_proto::FeatureOptions { delayed_ack: burst };
            let client_features = adb_proto::client_supported_features(&options);

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

    /// Runs a command on the device using Shell v1 or v2 protocol.
    ///
    /// Ports `adb_shell` and `RemoteShell` from `original/client/commandline.cpp`.
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
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W1: tokio::io::AsyncWrite + Unpin + Send,
        W2: tokio::io::AsyncWrite + Unpin + Send,
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let features_outcome = self.features(selector).await;
        let features_str = match features_outcome.result {
            Ok(f) => f,
            Err(err) => {
                return Outcome {
                    result: Err(err),
                    events: outcome.events,
                };
            }
        };

        let device_features = adb_proto::Features::parse(&features_str.replace('\n', ","));
        let device_supports_shell2 = device_features.contains(&adb_proto::Feature::Shell2);

        let command = args.join(" ");
        let is_interactive = command.is_empty();

        let use_shell_protocol = device_supports_shell2 && !options.disable_shell_protocol;

        if !use_shell_protocol {
            let service_string = format!("shell:{command}");
            if service_string.len() > adb_proto::limits::MAX_PAYLOAD_V1 {
                return Outcome {
                    result: Err(ClientError::QueryFailed(
                        "error: shell command too long".to_string(),
                    )),
                    events: outcome.events,
                };
            }
        }

        let stdin_isatty = stdin
            .as_ref()
            .and_then(|r| crate::shell_interactive::find_tty_fd(r))
            .is_some();

        let pty_dec = match crate::shell_interactive::determine_pty_allocation(
            options,
            is_interactive,
            stdin_isatty,
            device_supports_shell2,
        ) {
            Ok(dec) => dec,
            Err(err) => {
                return Outcome {
                    result: Err(err),
                    events: outcome.events,
                };
            }
        };

        let service_string = if pty_dec.use_shell_protocol {
            let mut v2_args = vec!["v2".to_string()];
            if let Some(term) = &options.term {
                v2_args.push(format!("TERM={term}"));
            }
            if !pty_dec.type_arg.is_empty() {
                v2_args.push(pty_dec.type_arg.to_string());
            }
            format!("shell,{}:{}", v2_args.join(","), command)
        } else {
            format!("shell:{command}")
        };

        let res = self
            .run_shell_session(
                selector,
                &service_string,
                &pty_dec,
                options,
                stdin,
                stdout,
                stderr,
            )
            .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_shell_session<R, W1, W2>(
        &self,
        selector: &TransportSelector,
        service_string: &str,
        pty_dec: &crate::shell_interactive::PtyDecision,
        options: &ShellOptions,
        stdin: Option<R>,
        mut stdout_writer: Option<W1>,
        mut stderr_writer: Option<W2>,
    ) -> Result<i32, ClientError>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
        W1: tokio::io::AsyncWrite + Unpin + Send,
        W2: tokio::io::AsyncWrite + Unpin + Send,
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        if let Some(warning) = &pty_dec.warning {
            if let Some(ref mut w) = stderr_writer {
                let _ = w.write_all(warning.as_bytes()).await;
                let _ = w.flush().await;
            } else {
                eprint!("{warning}");
            }
        }

        let stdin_tty_fd = stdin
            .as_ref()
            .and_then(|r| crate::shell_interactive::find_tty_fd(r));

        let _raw_guard = if pty_dec.raw_stdin {
            stdin_tty_fd.and_then(crate::shell_interactive::RawTerminalGuard::new)
        } else {
            None
        };

        let mut session = self.session_factory.connect(&self.addr).await?;
        session.select_transport(selector).await?;
        session.send_service(service_string).await?;
        session.read_status().await?;

        let stream = session.into_inner();
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        let mut stdin_reader = stdin;
        let mut stdin_eof = stdin_reader.is_none() || options.close_stdin;

        let mut read_buf = [0u8; 8192];
        let mut stdin_buf = [0u8; 8192];

        if pty_dec.use_shell_protocol {
            let mut decoder = adb_proto::ShellDecoder::new();
            let mut exit_code: Option<i32> = None;
            let mut escape_sm =
                crate::shell_interactive::EscapeStateMachine::new(if pty_dec.raw_stdin {
                    options.escape_char
                } else {
                    None
                });

            // Send initial window size if available and requesting PTY
            if pty_dec.type_arg == "pty" {
                if let Some(fd) = stdin_tty_fd {
                    if let Some((r, c, x, y)) = crate::shell_interactive::get_window_size(fd) {
                        let payload = crate::shell_interactive::format_window_size(r, c, x, y);
                        let winsz_pkt = adb_proto::ShellPacket::new(
                            adb_proto::ShellPacketId::WindowSizeChange,
                            payload,
                        );
                        if let Ok(encoded) = winsz_pkt.encode() {
                            let _ = write_half.write_all(&encoded).await;
                            let _ = write_half.flush().await;
                        }
                    }
                }
            }

            // SIGWINCH watcher channel
            #[cfg(unix)]
            let (winch_tx, mut winch_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
            #[cfg(unix)]
            let sigwinch_task = if pty_dec.type_arg == "pty" {
                stdin_tty_fd.map(|fd| {
                    tokio::spawn(async move {
                        if let Ok(mut sig) = tokio::signal::unix::signal(
                            tokio::signal::unix::SignalKind::window_change(),
                        ) {
                            while sig.recv().await.is_some() {
                                if let Some((r, c, x, y)) =
                                    crate::shell_interactive::get_window_size(fd)
                                {
                                    let payload =
                                        crate::shell_interactive::format_window_size(r, c, x, y);
                                    if winch_tx.send(payload).is_err() {
                                        break;
                                    }
                                }
                            }
                        }
                    })
                })
            } else {
                None
            };

            if stdin_reader.is_none() || options.close_stdin {
                let close_pkt =
                    adb_proto::ShellPacket::new(adb_proto::ShellPacketId::CloseStdin, vec![]);
                if let Ok(encoded) = close_pkt.encode() {
                    let _ = write_half.write_all(&encoded).await;
                    let _ = write_half.flush().await;
                }
                stdin_eof = true;
            }

            loop {
                tokio::select! {
                    n_res = read_half.read(&mut read_buf) => {
                        let n = n_res.map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;
                        if n == 0 {
                            break;
                        }
                        decoder.feed(&read_buf[..n]);

                        while let Ok(Some(pkt)) = decoder.pop_packet() {
                            match pkt.id {
                                adb_proto::ShellPacketId::Stdout => {
                                    if let Some(ref mut w) = stdout_writer {
                                        if w.write_all(&pkt.data).await.is_err() || w.flush().await.is_err() {
                                            exit_code = Some(128 + 13);
                                            break;
                                        }
                                    }
                                }
                                adb_proto::ShellPacketId::Stderr => {
                                    if let Some(ref mut w) = stderr_writer {
                                        if w.write_all(&pkt.data).await.is_err() || w.flush().await.is_err() {
                                            exit_code = Some(128 + 13);
                                            break;
                                        }
                                    }
                                }
                                adb_proto::ShellPacketId::Exit => {
                                    if let Ok(code) = pkt.parse_exit_code() {
                                        exit_code = Some(code as i32);
                                    }
                                }
                                _ => {}
                            }
                        }

                        if exit_code == Some(128 + 13) {
                            break;
                        }
                    }
                    winch_payload = async {
                        #[cfg(unix)]
                        {
                            winch_rx.recv().await
                        }
                        #[cfg(not(unix))]
                        {
                            std::future::pending::<Option<Vec<u8>>>().await
                        }
                    } => {
                        if let Some(payload) = winch_payload {
                            let winsz_pkt = adb_proto::ShellPacket::new(
                                adb_proto::ShellPacketId::WindowSizeChange,
                                payload,
                            );
                            if let Ok(encoded) = winsz_pkt.encode() {
                                let _ = write_half.write_all(&encoded).await;
                                let _ = write_half.flush().await;
                            }
                        }
                    }
                    n_res = async {
                        if let Some(ref mut r) = stdin_reader {
                            r.read(&mut stdin_buf).await
                        } else {
                            std::future::pending().await
                        }
                    }, if !stdin_eof => {
                        match n_res {
                            Ok(0) => {
                                stdin_eof = true;
                                let close_pkt = adb_proto::ShellPacket::new(
                                    adb_proto::ShellPacketId::CloseStdin,
                                    vec![],
                                );
                                if let Ok(encoded) = close_pkt.encode() {
                                    let _ = write_half.write_all(&encoded).await;
                                    let _ = write_half.flush().await;
                                }
                            }
                            Ok(n) => {
                                let esc_res = escape_sm.process_chunk(&stdin_buf[..n]);
                                match esc_res {
                                    crate::shell_interactive::EscapeResult::Output(out_bytes) => {
                                        if !out_bytes.is_empty() {
                                            let stdin_pkt = adb_proto::ShellPacket::new(
                                                adb_proto::ShellPacketId::Stdin,
                                                out_bytes,
                                            );
                                            if let Ok(encoded) = stdin_pkt.encode() {
                                                if write_half.write_all(&encoded).await.is_err() || write_half.flush().await.is_err() {
                                                    stdin_eof = true;
                                                }
                                            }
                                        }
                                    }
                                    crate::shell_interactive::EscapeResult::Disconnect => {
                                        const DISCONNECTED_MSG: &[u8] = b"\r\n[ disconnected ]\r\n";
                                        if let Some(ref mut w) = stderr_writer {
                                            let _ = w.write_all(DISCONNECTED_MSG).await;
                                            let _ = w.flush().await;
                                        } else {
                                            eprint!("\r\n[ disconnected ]\r\n");
                                        }
                                        #[cfg(unix)]
                                        if let Some(task) = sigwinch_task {
                                            task.abort();
                                        }
                                        #[cfg(unix)]
                                        drop(_raw_guard);
                                        return Ok(0);
                                    }
                                }
                            }
                            Err(_) => {
                                stdin_eof = true;
                            }
                        }
                    }
                }
            }

            #[cfg(unix)]
            if let Some(task) = sigwinch_task {
                task.abort();
            }

            Ok(exit_code.unwrap_or(255))
        } else {
            loop {
                tokio::select! {
                    n_res = read_half.read(&mut read_buf) => {
                        let n = n_res.map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;
                        if n == 0 {
                            break;
                        }
                        if let Some(ref mut w) = stdout_writer {
                            if w.write_all(&read_buf[..n]).await.is_err() || w.flush().await.is_err() {
                                break;
                            }
                        }
                    }
                    n_res = async {
                        if let Some(ref mut r) = stdin_reader {
                            r.read(&mut stdin_buf).await
                        } else {
                            std::future::pending().await
                        }
                    }, if !stdin_eof => {
                        match n_res {
                            Ok(0) => {
                                stdin_eof = true;
                            }
                            Ok(n) => {
                                if write_half.write_all(&stdin_buf[..n]).await.is_err() || write_half.flush().await.is_err() {
                                    stdin_eof = true;
                                }
                            }
                            Err(_) => {
                                stdin_eof = true;
                            }
                        }
                    }
                }
            }

            Ok(0)
        }
    }

    /// Runs a command on the device with stdin piped to the remote command (`exec-in`).
    pub async fn exec_in<R>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        mut stdin: R,
    ) -> Outcome<i32>
    where
        R: tokio::io::AsyncRead + Unpin + Send,
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

        let mut cmd = format!("exec:{}", args[0]);
        for arg in &args[1..] {
            cmd.push(' ');
            cmd.push_str(&escape_arg(arg));
        }

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;

            let mut stream = session.into_inner();
            tokio::io::copy(&mut stdin, &mut stream)
                .await
                .map_err(|e| ClientError::Session(SessionError::WriteFailure(e)))?;

            Ok(0)
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }

    /// Runs a command on the device and streams stdout back (`exec-out`).
    pub async fn exec_out<W>(
        &self,
        selector: &TransportSelector,
        args: &[&str],
        mut stdout: W,
    ) -> Outcome<i32>
    where
        W: tokio::io::AsyncWrite + Unpin + Send,
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

        let mut cmd = format!("exec:{}", args[0]);
        for arg in &args[1..] {
            cmd.push(' ');
            cmd.push_str(&escape_arg(arg));
        }

        let res = async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.select_transport(selector).await?;
            session.send_service(&cmd).await?;
            session.read_status().await?;

            let mut stream = session.into_inner();
            tokio::io::copy(&mut stream, &mut stdout)
                .await
                .map_err(|e| ClientError::Session(SessionError::ReadStatusFailed(e)))?;

            Ok(0)
        }
        .await;

        Outcome {
            result: res,
            events: outcome.events,
        }
    }
}
