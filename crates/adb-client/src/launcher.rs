use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// Errors that can occur when launching an ADB server process.
#[derive(Debug)]
pub enum LaunchError {
    Io(std::io::Error),
    Unsupported(String),
    ServerExited(String),
    TimedOut,
    Other(String),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Io(err) => write!(f, "IO error: {err}"),
            LaunchError::Unsupported(msg) => write!(f, "Unsupported: {msg}"),
            LaunchError::ServerExited(msg) => write!(f, "Server exited: {msg}"),
            LaunchError::TimedOut => write!(f, "Timed out waiting for server acknowledgment"),
            LaunchError::Other(msg) => write!(f, "{msg}"),
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

/// Trait for launching an ADB server process.
pub trait ServerLauncher: Send + Sync {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>>;
}

/// Server launcher that executes `std::env::current_exe()`.
#[derive(Debug, Default, Clone)]
pub struct SelfExecLauncher;

impl ServerLauncher for SelfExecLauncher {
    fn launch<'a>(
        &'a self,
        spec: &'a str,
        one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move {
            let exe = std::env::current_exe().map_err(LaunchError::Io)?;
            launch_binary(&exe, spec, one_device).await
        })
    }
}

/// Server launcher that executes a specific binary path.
#[derive(Debug, Clone)]
pub struct ExternalBinaryLauncher {
    path: PathBuf,
}

impl ExternalBinaryLauncher {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
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

#[cfg(unix)]
async fn launch_binary(
    path: &Path,
    spec: &str,
    one_device: Option<&str>,
) -> Result<(), LaunchError> {
    use std::os::unix::process::CommandExt;

    // Create pipe for acknowledgment from the launched server.
    // Ports `launch_server` pipe handling from C++ `original/adb.cpp` lines 1060-1120.
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(LaunchError::Io(std::io::Error::last_os_error()));
    }
    let pipe_read = fds[0];
    let pipe_write = fds[1];

    let mut cmd = std::process::Command::new(path);
    cmd.arg("-L").arg(spec);
    cmd.arg("fork-server");
    cmd.arg("server");
    cmd.arg("--reply-fd").arg(pipe_write.to_string());
    if let Some(dev) = one_device {
        cmd.arg("--one-device").arg(dev);
    }

    cmd.stdin(std::process::Stdio::null());

    unsafe {
        cmd.pre_exec(move || {
            // Detach process into new session and process group so daemon survives parent exiting.
            libc::setsid();
            // Clear FD_CLOEXEC on pipe_write handle so it stays open across exec.
            let flags = libc::fcntl(pipe_write, libc::F_GETFD);
            if flags != -1 {
                libc::fcntl(pipe_write, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }

    let _child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            unsafe {
                libc::close(pipe_read);
                libc::close(pipe_write);
            }
            return Err(LaunchError::Io(err));
        }
    };

    // Close parent's copy of pipe_write so child is sole write handle holder.
    unsafe {
        libc::close(pipe_write);
    }

    // Wait for "OK\n" message (3 bytes) from child process.
    let read_res = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 3];
        let mut total = 0;
        while total < 3 {
            let n = unsafe {
                libc::read(
                    pipe_read,
                    buf[total..].as_mut_ptr() as *mut libc::c_void,
                    3 - total,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                unsafe { libc::close(pipe_read) };
                return Err(LaunchError::Io(err));
            }
            if n == 0 {
                unsafe { libc::close(pipe_read) };
                return Err(LaunchError::ServerExited(
                    "server process exited without acknowledging startup".into(),
                ));
            }
            total += n as usize;
        }
        unsafe { libc::close(pipe_read) };
        if &buf == b"OK\n" {
            Ok(())
        } else {
            Err(LaunchError::Other(format!(
                "unexpected server reply: {:?}",
                String::from_utf8_lossy(&buf)
            )))
        }
    })
    .await;

    match read_res {
        Ok(res) => res,
        Err(e) => Err(LaunchError::Other(e.to_string())),
    }
}

#[cfg(not(unix))]
async fn launch_binary(
    _path: &Path,
    _spec: &str,
    _one_device: Option<&str>,
) -> Result<(), LaunchError> {
    Err(LaunchError::Unsupported(
        "server launching on Windows is not supported yet".to_string(),
    ))
}
