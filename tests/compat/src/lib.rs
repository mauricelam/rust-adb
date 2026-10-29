pub mod known_failures;
pub mod normalizer;
pub use known_failures::*;
pub use normalizer::*;

use std::env;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Status of a smart-socket response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartSocketStatus {
    Okay,
    Fail,
}

/// Parsed response from an ADB smart-socket request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartSocketResponse {
    pub status: SmartSocketStatus,
    /// 4-byte status header (e.g. `b"OKAY"` or `b"FAIL"`)
    pub status_bytes: [u8; 4],
    /// 4-char hex length prefix string if present in response framing.
    pub len_prefix: Option<String>,
    /// Response payload bytes (or error reason string bytes if status is FAIL).
    pub payload: Vec<u8>,
    /// Complete raw bytes received from the socket.
    pub raw_bytes: Vec<u8>,
}

impl SmartSocketResponse {
    pub fn is_okay(&self) -> bool {
        self.status == SmartSocketStatus::Okay
    }

    pub fn is_fail(&self) -> bool {
        self.status == SmartSocketStatus::Fail
    }

    pub fn payload_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.payload)
    }

    pub fn fail_reason(&self) -> Option<&str> {
        if self.is_fail() {
            self.payload_str().ok()
        } else {
            None
        }
    }
}

/// Configuration for the compatibility test harness.
#[derive(Debug, Clone)]
pub struct Config {
    pub client_path: PathBuf,
    pub server_bin_path: PathBuf,
    pub server_port: Option<u16>,
}

impl Config {
    /// Loads configuration from environment variables with defaults:
    /// - `ADB_CLIENT`: defaults to `ADB_REFERENCE` or default workspace binary.
    /// - `ADB_SERVER_BIN`: defaults to `ADB_REFERENCE` or default workspace binary.
    /// - `ADB_SERVER_PORT`: optional port override.
    pub fn from_env() -> Self {
        let reference = env::var("ADB_REFERENCE")
            .ok()
            .map(PathBuf::from)
            .or_else(default_reference_binary);

        let client_path = env::var("ADB_CLIENT")
            .ok()
            .map(PathBuf::from)
            .or_else(|| reference.clone())
            .expect("ADB_CLIENT or ADB_REFERENCE environment variable must be set (or reference binary present in binaries/)");

        let server_bin_path = env::var("ADB_SERVER_BIN")
            .ok()
            .map(PathBuf::from)
            .or_else(|| reference.clone())
            .expect("ADB_SERVER_BIN or ADB_REFERENCE environment variable must be set (or reference binary present in binaries/)");

        let server_port = env::var("ADB_SERVER_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok());

        Config {
            client_path,
            server_bin_path,
            server_port,
        }
    }
}

/// Locate reference binary relative to workspace root if ADB_REFERENCE is not explicitly set.
fn default_reference_binary() -> Option<PathBuf> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    let root = PathBuf::from(manifest_dir).join("../..");

    let path = if cfg!(target_os = "windows") {
        root.join("binaries/windows/adb.exe")
    } else if cfg!(target_os = "macos") {
        root.join("binaries/mac/adb")
    } else {
        root.join("binaries/linux/adb")
    };

    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Output captured from running an ADB command.
#[derive(Debug, Clone)]
pub struct TestOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub status: ExitStatus,
}

impl TestOutput {
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

/// Managed ADB test server running in an isolated environment.
pub struct TestServer {
    pub port: u16,
    pub home_dir: TempDir,
    child: Option<Child>,
    config: Config,
}

impl TestServer {
    /// Starts a test server using `Config::from_env()`.
    pub fn start() -> Self {
        Self::start_with_config(Config::from_env())
    }

    /// Starts a test server with the given `Config`.
    pub fn start_with_config(config: Config) -> Self {
        let home_dir =
            tempfile::tempdir().expect("Failed to create temporary directory for test home");

        let mut attempts = 0;
        loop {
            attempts += 1;
            let port = match config.server_port {
                Some(p) => p,
                None => get_free_port().expect("Failed to find free TCP port for ADB server"),
            };

            let mut cmd = Command::new(&config.server_bin_path);
            cmd.args(["-P", &port.to_string(), "server", "nodaemon"]);
            set_isolated_env(&mut cmd, home_dir.path(), port);

            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(_) if config.server_port.is_none() && attempts < 10 => continue,
                Err(e) => panic!("Failed to start ADB server process: {e}"),
            };

            let start_time = Instant::now();
            let timeout = get_cmd_timeout();
            let mut ready = false;

            while start_time.elapsed() < timeout {
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    ready = true;
                    break;
                }
                if let Ok(Some(status)) = child.try_wait() {
                    if config.server_port.is_none() && attempts < 10 {
                        // Port collision or premature exit, retry with a new free port
                        break;
                    }
                    panic!("ADB server process exited unexpectedly with status: {status}");
                }
                thread::sleep(Duration::from_millis(50));
            }

            if ready {
                return TestServer {
                    port,
                    home_dir,
                    child: Some(child),
                    config,
                };
            }

            #[cfg(unix)]
            {
                let pgid = child.id() as i32;
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
            let _ = child.kill();
            let _ = child.wait();

            if config.server_port.is_some() || attempts >= 10 {
                panic!(
                    "Timed out after {:?} waiting for ADB server to listen on port {port}",
                    timeout
                );
            }
        }
    }

    /// Executes the ADB client binary against this server instance.
    pub fn run_client(&self, args: &[&str]) -> TestOutput {
        let port_str = self.port.to_string();
        let mut full_args = vec!["-P", port_str.as_str()];
        full_args.extend_from_slice(args);

        let home_str = self.home_dir.path().to_string_lossy().to_string();
        let socket_str = format!("tcp:localhost:{}", self.port);

        let envs = [
            ("HOME", home_str.as_str()),
            ("USERPROFILE", home_str.as_str()),
            ("ANDROID_ADB_HOME", home_str.as_str()),
            ("ANDROID_USER_HOME", home_str.as_str()),
            ("ADB_MDNS_AUTO_CONNECT", "0"),
            ("ADB_MDNS_OPENSCREEN", "0"),
            ("ANDROID_ADB_SERVER_PORT", port_str.as_str()),
            ("ADB_SERVER_SOCKET", socket_str.as_str()),
        ];

        run_command_with_timeout(
            &self.config.client_path,
            &full_args,
            &envs,
            get_cmd_timeout(),
        )
    }

    /// Sends a smart-socket request formatted as `4-hex-digit-len + service_name`,
    /// then reads the status (`OKAY` or `FAIL`), the 4-hex-digit length prefix,
    /// and the payload bytes.
    pub fn send_smart_socket_request(&self, service: &str) -> std::io::Result<SmartSocketResponse> {
        let mut stream = connect_with_retry(self.port, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let request = format!("{:04x}{}", service.len(), service);
        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        let mut raw_bytes = Vec::new();

        let mut status_bytes = [0u8; 4];
        stream.read_exact(&mut status_bytes)?;
        raw_bytes.extend_from_slice(&status_bytes);

        let status = match &status_bytes {
            b"OKAY" => SmartSocketStatus::Okay,
            b"FAIL" => SmartSocketStatus::Fail,
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "Unexpected smart socket status: {:?}",
                        String::from_utf8_lossy(&status_bytes)
                    ),
                ))
            }
        };

        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes)?;
        raw_bytes.extend_from_slice(&len_bytes);

        let len_str = String::from_utf8_lossy(&len_bytes).to_string();
        let payload_len = usize::from_str_radix(&len_str, 16)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let mut payload = vec![0u8; payload_len];
        if payload_len > 0 {
            stream.read_exact(&mut payload)?;
            raw_bytes.extend_from_slice(&payload);
        }

        Ok(SmartSocketResponse {
            status,
            status_bytes,
            len_prefix: Some(len_str),
            payload,
            raw_bytes,
        })
    }

    /// Sends a smart-socket request and reads all raw output returned by the server until connection closes.
    pub fn send_smart_socket_request_raw(&self, service: &str) -> std::io::Result<Vec<u8>> {
        let mut stream = connect_with_retry(self.port, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let request = format!("{:04x}{}", service.len(), service);
        stream.write_all(request.as_bytes())?;
        stream.flush()?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        Ok(response)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            #[cfg(unix)]
            {
                let pgid = child.id() as i32;
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Runs the ADB client command directly using `Config::from_env()` in an isolated environment.
pub fn run_client(args: &[&str]) -> TestOutput {
    run_client_with_config(&Config::from_env(), args)
}

/// Runs the ADB client command with a custom `Config` in an isolated environment.
pub fn run_client_with_config(config: &Config, args: &[&str]) -> TestOutput {
    let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory for test home");
    let port = config.server_port.unwrap_or(0);

    let mut full_args = Vec::new();
    let port_str;
    if port != 0 {
        port_str = port.to_string();
        full_args.push("-P");
        full_args.push(port_str.as_str());
    }
    full_args.extend_from_slice(args);

    let home_str = temp_dir.path().to_string_lossy().to_string();
    let mut envs = vec![
        ("HOME", home_str.as_str()),
        ("USERPROFILE", home_str.as_str()),
        ("ANDROID_ADB_HOME", home_str.as_str()),
        ("ANDROID_USER_HOME", home_str.as_str()),
        ("ADB_MDNS_AUTO_CONNECT", "0"),
        ("ADB_MDNS_OPENSCREEN", "0"),
    ];

    let port_env_str;
    let socket_env_str;
    if port != 0 {
        port_env_str = port.to_string();
        socket_env_str = format!("tcp:localhost:{port}");
        envs.push(("ANDROID_ADB_SERVER_PORT", port_env_str.as_str()));
        envs.push(("ADB_SERVER_SOCKET", socket_env_str.as_str()));
    }

    run_command_with_timeout(&config.client_path, &full_args, &envs, get_cmd_timeout())
}

pub fn get_cmd_timeout() -> Duration {
    if let Ok(val) = env::var("COMPAT_CMD_TIMEOUT_SECS") {
        match val.parse::<u64>() {
            Ok(secs) => Duration::from_secs(secs),
            Err(err) => panic!("Invalid COMPAT_CMD_TIMEOUT_SECS value '{val}': {err}"),
        }
    } else {
        Duration::from_secs(60)
    }
}

/// Managed ADB server environment isolated from host environment and other test instances.
pub struct IsolatedServer {
    pub port: u16,
    pub home_dir: TempDir,
    pub config: Config,
}

impl IsolatedServer {
    /// Creates a new isolated server environment using `Config::from_env()`.
    pub fn new() -> Self {
        Self::with_config(Config::from_env())
    }

    /// Creates a new isolated server environment with the given `Config`.
    pub fn with_config(config: Config) -> Self {
        let home_dir =
            tempfile::tempdir().expect("Failed to create temporary directory for test home");
        let port = match config.server_port {
            Some(p) => p,
            None => get_free_port().expect("Failed to find free TCP port for ADB server"),
        };
        IsolatedServer {
            port,
            home_dir,
            config,
        }
    }

    /// Starts the server on an isolated free port, retrying with a new port if port binding fails.
    pub fn start_server(&mut self) -> TestOutput {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let output = self.run_client(&["start-server"]);
            if output.success() {
                return output;
            }
            if attempts < 10 {
                if let Ok(new_port) = get_free_port() {
                    self.port = new_port;
                    continue;
                }
            }
            return output;
        }
    }

    /// Executes the ADB client binary against this isolated server environment.
    pub fn run_client(&self, args: &[&str]) -> TestOutput {
        let port_str = self.port.to_string();
        let mut full_args = vec!["-P", port_str.as_str()];
        full_args.extend_from_slice(args);

        let home_str = self.home_dir.path().to_string_lossy().to_string();
        let socket_str = format!("tcp:localhost:{}", self.port);
        let server_bin_str = self.config.server_bin_path.to_string_lossy().to_string();

        let envs = [
            ("HOME", home_str.as_str()),
            ("USERPROFILE", home_str.as_str()),
            ("ANDROID_ADB_HOME", home_str.as_str()),
            ("ANDROID_USER_HOME", home_str.as_str()),
            ("ADB_MDNS_AUTO_CONNECT", "0"),
            ("ADB_MDNS_OPENSCREEN", "0"),
            ("ANDROID_ADB_SERVER_PORT", port_str.as_str()),
            ("ADB_SERVER_SOCKET", socket_str.as_str()),
            ("ADB_SERVER_BIN", server_bin_str.as_str()),
        ];

        run_command_with_timeout(
            &self.config.client_path,
            &full_args,
            &envs,
            get_cmd_timeout(),
        )
    }
}

impl Default for IsolatedServer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for IsolatedServer {
    fn drop(&mut self) {
        if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
            let _ = self.run_client(&["kill-server"]);
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                if std::net::TcpStream::connect(("127.0.0.1", self.port)).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// Executes an arbitrary command with arguments, environment variables, and timeout.
pub fn run_command_with_timeout(
    program: impl AsRef<Path>,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout: Duration,
) -> TestOutput {
    let mut cmd = Command::new(program.as_ref());
    cmd.env_remove("ANDROID_SERIAL");
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }

    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    let mut child = cmd.spawn().expect("Failed to execute command");

    let stdout = child.stdout.take().expect("Failed to open stdout");
    let stderr = child.stderr.take().expect("Failed to open stderr");

    let stdout_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let stdout_clone = stdout_buf.clone();
    let stdout_thread = thread::spawn(move || {
        let mut reader = stdout;
        let mut buf = [0u8; 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Ok(mut lock) = stdout_clone.lock() {
                lock.extend_from_slice(&buf[..n]);
            }
        }
    });

    let stderr_clone = stderr_buf.clone();
    let stderr_thread = thread::spawn(move || {
        let mut reader = stderr;
        let mut buf = [0u8; 1024];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Ok(mut lock) = stderr_clone.lock() {
                lock.extend_from_slice(&buf[..n]);
            }
        }
    });

    let start = Instant::now();

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() >= timeout {
                    #[cfg(unix)]
                    {
                        let pgid = child.id() as i32;
                        unsafe {
                            libc::kill(-pgid, libc::SIGKILL);
                        }
                    }
                    let _ = child.kill();
                    let _ = child.wait();

                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();

                    let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
                    let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();

                    panic!(
                        "Command timed out after {:?}.\nArguments: {:?}\nCaptured stdout:\n{}\nCaptured stderr:\n{}",
                        timeout, args, out_str, err_str
                    );
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("Error waiting for child process: {e}"),
        }
    };

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let stdout = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
    let stderr = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();

    TestOutput {
        stdout,
        stderr,
        exit_code: status.code(),
        status,
    }
}

/// Returns whether a process with the given PID is currently running.
#[cfg(unix)]
pub fn is_process_running(pid: u32) -> bool {
    let ret = unsafe { libc::kill(pid as i32, 0) };
    if ret == 0 {
        true
    } else {
        let err = std::io::Error::last_os_error();
        err.raw_os_error() != Some(libc::ESRCH)
    }
}

/// Returns whether a process group with the given PGID is currently running.
#[cfg(unix)]
pub fn is_process_group_running(pgid: u32) -> bool {
    let ret = unsafe { libc::kill(-(pgid as i32), 0) };
    if ret == 0 {
        true
    } else {
        let err = std::io::Error::last_os_error();
        err.raw_os_error() != Some(libc::ESRCH)
    }
}

/// Returns whether a process with the given PID is currently running.
#[cfg(windows)]
pub fn is_process_running(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0u32;
        let res = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        res != 0 && exit_code == (STILL_ACTIVE as u32)
    }
}

fn set_isolated_env(cmd: &mut Command, home_path: &std::path::Path, port: u16) {
    cmd.env("HOME", home_path);
    cmd.env("USERPROFILE", home_path);
    cmd.env("ANDROID_ADB_HOME", home_path);
    cmd.env("ANDROID_USER_HOME", home_path);
    cmd.env("ADB_MDNS_AUTO_CONNECT", "0");
    cmd.env("ADB_MDNS_OPENSCREEN", "0");
    if port != 0 {
        cmd.env("ANDROID_ADB_SERVER_PORT", port.to_string());
        cmd.env("ADB_SERVER_SOCKET", format!("tcp:localhost:{port}"));
    }
}

fn connect_with_retry(port: u16, timeout: Duration) -> std::io::Result<std::net::TcpStream> {
    let start = Instant::now();
    loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                if start.elapsed() >= timeout {
                    return Err(err);
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn get_free_port() -> std::io::Result<u16> {
    static RECENT_PORTS: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    let mut listeners = Vec::new();
    loop {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let mut recent = RECENT_PORTS.lock().unwrap();
        if !recent.contains(&port) {
            recent.push(port);
            if recent.len() > 100 {
                recent.remove(0);
            }
            drop(recent);
            drop(listener);
            drop(listeners);
            thread::sleep(Duration::from_millis(20));
            return Ok(port);
        }
        listeners.push(listener);
    }
}
