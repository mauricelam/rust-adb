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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

static TEST_START_TIME: OnceLock<Instant> = OnceLock::new();
static STEP_MARKERS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn log_step(step_num: usize, what: &str) {
    let start = *TEST_START_TIME.get_or_init(Instant::now);
    let ms = start.elapsed().as_millis();
    let msg = format!("[STEP {step_num}] {what} +{ms}ms");
    if let Ok(mut guard) = STEP_MARKERS.lock() {
        guard.push(msg.clone());
    }
    eprintln!("{msg}");
    let _ = std::io::stderr().flush();
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

fn set_window_size(fd_raw: std::os::fd::RawFd, rows: u16, cols: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let res_set = unsafe { libc::ioctl(fd_raw, libc::TIOCSWINSZ, &ws) };
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
    TEST_START_TIME.set(Instant::now()).ok();
    if let Ok(mut guard) = STEP_MARKERS.lock() {
        guard.clear();
    }

    let test_done = Arc::new(AtomicBool::new(false));
    let test_done_watchdog = test_done.clone();

    let collected_bytes = Arc::new(Mutex::new(Vec::<u8>::new()));
    let watchdog_collected = collected_bytes.clone();
    let test_pid = std::process::id();

    let watchdog_thread = std::thread::spawn(move || {
        let timeout = Duration::from_secs(60);
        let poll_interval = Duration::from_millis(100);
        let start = Instant::now();

        while start.elapsed() < timeout {
            if test_done_watchdog.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(poll_interval);
        }

        eprintln!("\n================ WATCHDOG TIMEOUT (60s) ==================");
        eprintln!("test_unix_interactive_shell_pty_resize_disconnect timed out!");
        eprintln!("Step markers logged so far:");
        if let Ok(markers) = STEP_MARKERS.lock() {
            for m in markers.iter() {
                eprintln!("  {m}");
            }
        }
        if let Ok(bytes) = watchdog_collected.lock() {
            eprintln!("Bytes collected so far ({}) bytes:", bytes.len());
            eprintln!("{:?}", String::from_utf8_lossy(&bytes));
        }

        #[cfg(target_os = "macos")]
        {
            eprintln!("\nSampling process stack via `sample {} 3`...", test_pid);
            let _ = Command::new("sample")
                .args([test_pid.to_string(), "3".to_string()])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();
        }

        #[cfg(target_os = "linux")]
        {
            eprintln!("\nSampling process stack via `gdb`...");
            let _ = Command::new("gdb")
                .args([
                    "-p",
                    &test_pid.to_string(),
                    "-batch",
                    "-ex",
                    "thread apply all bt",
                ])
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();
        }

        eprintln!("Aborting process via std::process::abort()");
        eprintln!("==========================================================\n");
        let _ = std::io::stderr().flush();
        std::process::abort();
    });

    log_step(1, "server start");
    let mock = MockAdbdProcess::start(&[]);
    let server = TestServer::start();

    log_step(2, "connect");
    let connect_out = server.run_client(&["connect", &mock.addr]);
    assert!(connect_out.success(), "adb connect failed");

    log_step(3, "openpty");
    let (master_fd, slave_fd) = open_pty();

    let mut master_file = std::fs::File::from(master_fd);
    let mut master_writer = master_file
        .try_clone()
        .expect("Failed to clone master_file for writing");

    let reader_bytes = collected_bytes.clone();
    let reader_stop = Arc::new(AtomicBool::new(false));
    let reader_stop_flag = reader_stop.clone();

    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            if reader_stop_flag.load(Ordering::Relaxed) {
                break;
            }
            match master_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut guard) = reader_bytes.lock() {
                        guard.extend_from_slice(&buf[..n]);
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Initial window size
    set_window_size(slave_fd.as_raw_fd(), 30, 90);

    let slave_check_fd = slave_fd.try_clone().unwrap();
    let orig_termios = get_termios(slave_check_fd.as_raw_fd());

    let slave_file_in = tokio::fs::File::from(std::fs::File::from(slave_fd.try_clone().unwrap()));
    let slave_file_out = tokio::fs::File::from(std::fs::File::from(slave_fd));

    let serial = mock.addr.clone();

    let addr = ServerAddress {
        host: Some("127.0.0.1".into()),
        port: server.port,
    };

    let client = AdbClient::builder(addr)
        .session_factory(DefaultSessionFactory::new())
        .launcher(DummyLauncher)
        .build();

    log_step(4, "spawn shell task");
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

    // Give shell time to initialize raw mode and send initial window size
    tokio::time::sleep(Duration::from_millis(200)).await;

    log_step(5, "raw-mode check");
    let raw_termios = get_termios(slave_check_fd.as_raw_fd());
    assert_eq!(
        raw_termios.c_lflag & (libc::ICANON | libc::ECHO),
        0,
        "termios should be in raw mode"
    );

    log_step(6, "SIGWINCH");
    set_window_size(master_writer.as_raw_fd(), 45, 110);
    unsafe {
        libc::kill(libc::getpid(), libc::SIGWINCH);
    }

    tokio::time::sleep(Duration::from_millis(200)).await;

    log_step(7, "writing ~.");
    master_writer.write_all(b"~.").unwrap();
    master_writer.flush().unwrap();

    let outcome = shell_task.await.unwrap();
    log_step(8, "shell task returned");
    let exit_code = outcome.result.expect("interactive shell failed");

    assert_eq!(exit_code, 0);

    log_step(9, "termios restore check");
    let restored_termios = get_termios(slave_check_fd.as_raw_fd());
    assert_eq!(
        restored_termios.c_lflag & libc::ICANON,
        orig_termios.c_lflag & libc::ICANON,
        "termios ICANON should be restored"
    );

    // Close slave and stop reader thread
    drop(slave_check_fd);
    drop(master_writer);
    reader_stop.store(true, Ordering::Relaxed);

    let start_join = Instant::now();
    let mut reader_joined = false;
    while start_join.elapsed() < Duration::from_secs(10) {
        if reader_thread.is_finished() {
            let _ = reader_thread.join();
            reader_joined = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    if !reader_joined {
        let current = collected_bytes.lock().unwrap().clone();
        panic!(
            "Reader thread failed to join within 10s. Collected bytes: {:?}",
            String::from_utf8_lossy(&current)
        );
    }
    log_step(10, "reader joined");

    log_step(11, "final assertions");
    let accumulated = collected_bytes.lock().unwrap().clone();
    let out_str = String::from_utf8_lossy(&accumulated);
    assert!(
        out_str.contains("WINSZ 30x90,0x0"),
        "master output should contain initial window size, got: {out_str}"
    );
    assert!(
        out_str.contains("WINSZ 45x110,0x0"),
        "master output should contain resized window size, got: {out_str}"
    );

    test_done.store(true, Ordering::Relaxed);
    let _ = watchdog_thread.join();
}
