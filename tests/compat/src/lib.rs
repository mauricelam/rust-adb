use std::env;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

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

        let port = match config.server_port {
            Some(p) => p,
            None => get_free_port().expect("Failed to find free TCP port for ADB server"),
        };

        let mut cmd = Command::new(&config.server_bin_path);
        cmd.args(["-P", &port.to_string(), "server", "nodaemon"]);
        set_isolated_env(&mut cmd, home_dir.path(), port);

        let mut child = cmd.spawn().expect("Failed to start ADB server process");

        // Wait up to 5 seconds for the server socket to accept connections
        let start_time = Instant::now();
        let timeout = Duration::from_secs(5);
        let mut ready = false;

        while start_time.elapsed() < timeout {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                ready = true;
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!("ADB server process exited unexpectedly with status: {status}");
            }
            thread::sleep(Duration::from_millis(50));
        }

        if !ready {
            let _ = child.kill();
            panic!("Timed out waiting for ADB server to listen on port {port}");
        }

        TestServer {
            port,
            home_dir,
            child: Some(child),
            config,
        }
    }

    /// Executes the ADB client binary against this server instance.
    pub fn run_client(&self, args: &[&str]) -> TestOutput {
        let mut cmd = Command::new(&self.config.client_path);
        cmd.arg("-P").arg(self.port.to_string());
        cmd.args(args);
        set_isolated_env(&mut cmd, self.home_dir.path(), self.port);

        let output = cmd.output().expect("Failed to execute ADB client command");

        TestOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code(),
            status: output.status,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Send graceful kill request
        let mut kill_cmd = Command::new(&self.config.client_path);
        kill_cmd.args(["-P", &self.port.to_string(), "kill-server"]);
        set_isolated_env(&mut kill_cmd, self.home_dir.path(), self.port);
        let _ = kill_cmd.status();

        if let Some(mut child) = self.child.take() {
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

    let mut cmd = Command::new(&config.client_path);
    if port != 0 {
        cmd.arg("-P").arg(port.to_string());
    }
    cmd.args(args);
    set_isolated_env(&mut cmd, temp_dir.path(), port);

    let output = cmd.output().expect("Failed to execute ADB client command");

    TestOutput {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code(),
        status: output.status,
    }
}

fn set_isolated_env(cmd: &mut Command, home_path: &std::path::Path, port: u16) {
    cmd.env("HOME", home_path);
    cmd.env("USERPROFILE", home_path);
    cmd.env("ANDROID_ADB_HOME", home_path);
    cmd.env("ANDROID_USER_HOME", home_path);
    if port != 0 {
        cmd.env("ANDROID_ADB_SERVER_PORT", port.to_string());
        cmd.env("ADB_SERVER_SOCKET", format!("tcp:localhost:{port}"));
    }
}

fn get_free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}
