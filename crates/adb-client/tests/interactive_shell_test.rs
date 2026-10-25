#![cfg(unix)]

use adb_client::{
    AdbClient, DefaultSessionFactory, LaunchError, ServerAddress, ServerLauncher, ShellOptions,
    TransportSelector,
};
use compat::TestServer;
use std::env;
use std::future::Future;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

static STEP_MARKERS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn record_step(step: &str) {
    if let Ok(mut markers) = STEP_MARKERS.lock() {
        markers.push(step.to_string());
    }
    if env::var_os("ADB_PTY_DEBUG").is_some() {
        eprintln!("[STEP MARKER] {step}");
    }
}

fn get_mock_adbd_bin() -> PathBuf {
    if let Ok(bin) = env::var("CARGO_BIN_EXE_mock-adbd") {
        return PathBuf::from(bin);
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let root = PathBuf::from(manifest_dir).join("../..");
    let bin_path = root.join("target").join("debug").join("mock-adbd");
    if !bin_path.exists() {
        let status = Command::new("cargo")
            .args(["build", "-p", "mock-adbd"])
            .status()
            .expect("Failed to build mock-adbd binary");
        assert!(status.success(), "Failed to build mock-adbd");
    }
    bin_path
}

struct MockAdbdProcess {
    child: Child,
    pub addr: String,
}

impl MockAdbdProcess {
    fn start(extra_args: &[&str]) -> Self {
        let bin = get_mock_adbd_bin();
        let mut cmd = Command::new(&bin);
        cmd.arg("--listen")
            .arg("127.0.0.1:0")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for arg in extra_args {
            cmd.arg(arg);
        }

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("Failed to spawn mock-adbd binary at {:?}: {}", bin, e));

        let stdout = child.stdout.take().expect("Failed to open stdout");
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("Failed to read first line from mock-adbd stdout");
        let addr = first_line.trim().to_string();

        MockAdbdProcess { child, addr }
    }
}

impl Drop for MockAdbdProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct DummyLauncher;

impl ServerLauncher for DummyLauncher {
    fn launch<'a>(
        &'a self,
        _spec: &'a str,
        _one_device: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<(), LaunchError>> + Send + 'a>> {
        Box::pin(async move {
            Err(LaunchError::Unsupported(
                "DummyLauncher cannot start daemon".to_string(),
            ))
        })
    }
}

fn open_pty() -> (OwnedFd, OwnedFd) {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let res = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(res, 0, "openpty failed");
    unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) }
}

fn set_window_size(fd: &OwnedFd, rows: u16, cols: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let res_set = unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCSWINSZ, &ws) };
    assert_eq!(res_set, 0, "TIOCSWINSZ failed");
}

fn get_termios(fd_raw: std::os::fd::RawFd) -> libc::termios {
    let mut termios = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::tcgetattr(fd_raw, &mut termios) };
    assert_eq!(res, 0, "tcgetattr failed");
    termios
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_unix_interactive_shell_pty_resize_disconnect() {
    env::set_var("ADB_PTY_DEBUG", "1");

    let watchdog_done = Arc::new(AtomicBool::new(false));
    let watchdog_done_clone = watchdog_done.clone();
    let test_pid = std::process::id();

    let _watchdog_handle = std::thread::spawn(move || {
        let timeout = Duration::from_secs(60);
        let poll_interval = Duration::from_millis(100);
        let start = std::time::Instant::now();

        while start.elapsed() < timeout {
            if watchdog_done_clone.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(poll_interval);
        }

        eprintln!("\n================ WATCHDOG TIMEOUT ==================");
        eprintln!("test_unix_interactive_shell_pty_resize_disconnect has been running for over 60 seconds!");
        eprintln!("Recorded step markers:");
        if let Ok(markers) = STEP_MARKERS.lock() {
            for (i, m) in markers.iter().enumerate() {
                eprintln!("  {}. {}", i + 1, m);
            }
        }

        #[cfg(target_os = "macos")]
        {
            eprintln!("\nSampling process stack using `sample`...");
            let _ = Command::new("sample")
                .args([test_pid.to_string(), "1".to_string()])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();
        }

        #[cfg(target_os = "linux")]
        {
            eprintln!("\nSampling process stack using `eu-stack`...");
            let _ = Command::new("eu-stack")
                .args(["-p", &test_pid.to_string()])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();
        }

        eprintln!("Aborting process via std::process::abort()");
        eprintln!("====================================================\n");
        std::process::abort();
    });

    record_step("1. Starting mock-adbd and TestServer");
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    record_step("2. Connecting TestServer to mock-adbd");
    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success(), "adb connect failed");

    record_step("3. Building AdbClient and opening PTY");
    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    let (master_fd, slave_fd) = open_pty();

    record_step("4. Setting initial window size (30x90)");
    set_window_size(&slave_fd, 30, 90);

    let slave_check_fd = slave_fd.try_clone().unwrap();
    let orig_termios = get_termios(slave_check_fd.as_raw_fd());

    let slave_file_in = tokio::fs::File::from(std::fs::File::from(slave_fd.try_clone().unwrap()));
    let slave_file_out = tokio::fs::File::from(std::fs::File::from(slave_fd));

    let serial = mock.addr.clone();

    record_step("5. Spawning client.shell task");
    let shell_task = tokio::spawn(async move {
        client
            .shell(
                &TransportSelector::Serial(serial),
                &[], // empty args = interactive
                &ShellOptions::default(),
                Some(slave_file_in),
                Some(slave_file_out),
                Option::<tokio::fs::File>::None,
            )
            .await
    });

    record_step("6. Sleeping 200ms for shell initialization and raw mode");
    tokio::time::sleep(Duration::from_millis(200)).await;

    record_step("7. Verifying raw terminal mode on slave PTY");
    let raw_termios = get_termios(slave_check_fd.as_raw_fd());
    assert_eq!(
        raw_termios.c_lflag & (libc::ICANON | libc::ECHO),
        0,
        "termios should be in raw mode"
    );

    record_step("8. Resizing window (45x110) and sending SIGWINCH");
    set_window_size(&master_fd, 45, 110);
    unsafe {
        libc::kill(libc::getpid(), libc::SIGWINCH);
    }

    record_step("9. Sleeping 200ms after SIGWINCH");
    tokio::time::sleep(Duration::from_millis(200)).await;

    record_step("10. Writing disconnect sequence '~.' to master PTY");
    let mut master_file: std::fs::File = master_fd.into();
    let mut master_reader = master_file.try_clone().unwrap();

    master_file.write_all(b"~.").unwrap();
    master_file.flush().unwrap();

    record_step("11. Awaiting shell_task completion");
    let outcome = shell_task.await.unwrap();
    let exit_code = outcome.result.expect("interactive shell failed");

    assert_eq!(exit_code, 0);

    record_step("12. Verifying restored termios mode");
    let restored_termios = get_termios(slave_check_fd.as_raw_fd());
    assert_eq!(
        restored_termios.c_lflag & libc::ICANON,
        orig_termios.c_lflag & libc::ICANON,
        "termios ICANON should be restored"
    );

    record_step("13. Reading master output and verifying WINSZ text");
    let mut read_buf = vec![0u8; 1024];
    let mut accumulated = Vec::new();
    unsafe {
        let flags = libc::fcntl(master_reader.as_raw_fd(), libc::F_GETFL);
        if flags != -1 {
            libc::fcntl(
                master_reader.as_raw_fd(),
                libc::F_SETFL,
                flags | libc::O_NONBLOCK,
            );
        }
    }
    while let Ok(n) = master_reader.read(&mut read_buf) {
        if n == 0 {
            break;
        }
        accumulated.extend_from_slice(&read_buf[..n]);
    }

    let out_str = String::from_utf8_lossy(&accumulated);
    assert!(
        out_str.contains("WINSZ 30x90,0x0"),
        "master output should contain initial window size, got: {out_str}"
    );
    assert!(
        out_str.contains("WINSZ 45x110,0x0"),
        "master output should contain resized window size, got: {out_str}"
    );

    record_step("14. Test completed successfully");
    watchdog_done.store(true, Ordering::Relaxed);
}
