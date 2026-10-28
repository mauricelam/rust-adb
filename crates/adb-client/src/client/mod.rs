pub mod device_tracker;
pub mod error;
pub mod events;
pub mod forward;
pub mod shell;

use crate::server_address::ServerAddress;
use crate::session::{Session, SessionConfig, SessionError};
use crate::transport::{format_host_command, TransportSelector};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use device_tracker::TrackDevicesMode;
pub use error::{ClientError, ClientErrorKind, LaunchError};
pub use events::{ClientEvent, Outcome};
pub use forward::{forward_dest_is_featured, forward_targets_are_valid};
pub use shell::{escape_arg, ShellOptions};

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

#[cfg(windows)]
async fn launch_binary(
    path: &std::path::Path,
    spec: &str,
    one_device: Option<&str>,
    config: LaunchConfig,
) -> Result<(), LaunchError> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let (read_handle, write_handle) = {
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1, // TRUE
        };

        let mut read_pipe: HANDLE = std::ptr::null_mut();
        let mut write_pipe: HANDLE = std::ptr::null_mut();

        if unsafe { CreatePipe(&mut read_pipe, &mut write_pipe, &sa, 0) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        let read_handle = unsafe { OwnedHandle::from_raw_handle(read_pipe as _) };
        let write_handle = unsafe { OwnedHandle::from_raw_handle(write_pipe as _) };
        (read_handle, write_handle)
    };

    let write_handle_raw = write_handle.as_raw_handle() as usize;

    let mut cmd = tokio::process::Command::new(path);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .arg("-L")
        .arg(spec)
        .arg("fork-server")
        .arg("server")
        .arg("--reply-fd")
        .arg(write_handle_raw.to_string())
        .creation_flags(CREATE_NO_WINDOW);

    if let Some(device) = one_device {
        cmd.arg("--one-device").arg(device);
    }

    let mut child = cmd.spawn()?;

    // Explicitly drop write_handle in parent so pipe closure signals EOF if child terminates early.
    drop(write_handle);

    let read_file = std::fs::File::from(read_handle);
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

#[cfg(not(any(unix, windows)))]
async fn launch_binary(
    _path: &std::path::Path,
    _spec: &str,
    _one_device: Option<&str>,
    _config: LaunchConfig,
) -> Result<(), LaunchError> {
    Err(LaunchError::Unsupported(
        "Server launching is not supported on this platform".to_string(),
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
        events.push(ClientEvent::ErrorLine(msg.clone()));
        return Err(ClientError::KillFailed(msg));
    }

    match session.read_status().await {
        Ok(()) => {
            let _ = session.read_to_close().await;
            Ok(())
        }
        Err(SessionError::Fail(reason)) => {
            events.push(ClientEvent::ErrorLine(reason.clone()));
            Err(ClientError::KillFailed(reason))
        }
        Err(_) => {
            let msg = "failed to read response from server".to_string();
            events.push(ClientEvent::ErrorLine(msg.clone()));
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

/// Represents an ADB client instance associated with a server address.
pub struct AdbClient {
    addr: ServerAddress,
    session_factory: Arc<dyn SessionFactory>,
    launcher: Arc<dyn ServerLauncher>,
    version_checked: tokio::sync::OnceCell<()>,
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
    pub(crate) async fn ensure_server(&self) -> Outcome<()> {
        let mut check_events = Vec::new();
        let res = self
            .version_checked
            .get_or_try_init(|| async {
                check_server_version(
                    &self.addr,
                    self.session_factory.as_ref(),
                    self.launcher.as_ref(),
                    &mut check_events,
                )
                .await
            })
            .await;

        match res {
            Ok(_) => Outcome {
                result: Ok(()),
                events: check_events,
            },
            Err(err) => Outcome {
                result: Err(err.clone()),
                events: check_events,
            },
        }
    }

    /// Helper for executing an operation after ensuring the server is running.
    pub(crate) async fn with_server<T, Fut>(&self, op: Fut) -> Outcome<T>
    where
        Fut: Future<Output = Result<T, ClientError>>,
    {
        let outcome = self.ensure_server().await;
        if let Err(err) = outcome.result {
            return Outcome {
                result: Err(err),
                events: outcome.events,
            };
        }

        let res = op.await;

        Outcome {
            result: res,
            events: outcome.events,
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
        let service = if long {
            "host:devices-l"
        } else {
            "host:devices"
        };

        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(format!("List of devices attached\n{payload}\n"))
        })
        .await
    }

    /// Executes a general host query command formatted with transport selector.
    ///
    /// Ports `adb_query_command(format_host_command(...))` from `commandline.cpp`.
    pub async fn query_command(
        &self,
        selector: &TransportSelector,
        command: &str,
    ) -> Outcome<String> {
        let service = format_host_command(command, selector);
        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(format!("{payload}\n"))
        })
        .await
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
        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            let transport_id = session.select_transport(selector).await?.unwrap_or(0);
            session.send_service("host:features").await?;
            session.read_status().await?;
            Ok(format!("{transport_id}\n"))
        })
        .await
    }

    /// Queries target device features and filters against client supported features (`features`).
    ///
    /// Ports `features` handling from `original/client/commandline.cpp`.
    pub async fn features(
        &self,
        selector: &TransportSelector,
        options: &adb_proto::FeatureOptions,
    ) -> Outcome<String> {
        let service = format_host_command("features", selector);
        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;

            let device_features = adb_proto::Features::parse(&payload);
            let client_features = adb_proto::client_supported_features(options);

            let mut output = String::new();
            for feature in device_features.items() {
                if client_features.contains(feature) {
                    output.push_str(feature.as_str());
                    output.push('\n');
                }
            }
            Ok(output)
        })
        .await
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
        let service = format!("host:connect:{target}");
        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(payload)
        })
        .await
    }

    /// Disconnects from a TCP/IP device (`host:disconnect:<target or empty>`).
    ///
    /// Ports `disconnect` command handling from `original/client/commandline.cpp`.
    /// If `target` is `None`, disconnects all TCP/IP devices.
    pub async fn disconnect(&self, target: Option<&str>) -> Outcome<String> {
        let service = format!("host:disconnect:{}", target.unwrap_or(""));
        self.with_server(async {
            let mut session = self.session_factory.connect(&self.addr).await?;
            session.send_service(&service).await?;
            session.read_status().await?;
            let payload = session.read_string().await?;
            Ok(payload)
        })
        .await
    }
}
