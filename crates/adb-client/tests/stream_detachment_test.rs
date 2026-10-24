#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
fn pid_file_path() -> std::path::PathBuf {
    std::env::temp_dir().join("stream_helper_fake_server.pid")
}

#[cfg(unix)]
#[test]
fn test_detached_server_streams() {
    let _ = fs::remove_file(pid_file_path());

    // Build the stream_helper example binary using cargo build --example stream_helper
    let build_status = Command::new("cargo")
        .args(["build", "--example", "stream_helper", "-p", "adb-client"])
        .status()
        .expect("Failed to run cargo build --example stream_helper");
    assert!(
        build_status.success(),
        "cargo build --example stream_helper failed"
    );

    let target_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/examples/stream_helper");

    let mut child = Command::new(&target_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn stream_helper");

    let stdout_pipe = child.stdout.take().unwrap();
    let stderr_pipe = child.stderr.take().unwrap();

    // Read stdout and stderr to EOF in separate threads with a short timeout
    let (tx_out, rx_out) = std::sync::mpsc::channel();
    let (tx_err, rx_err) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut p = stdout_pipe;
        let _ = p.read_to_end(&mut buf);
        tx_out.send(buf).unwrap();
    });

    std::thread::spawn(move || {
        use std::io::Read;
        let mut buf = Vec::new();
        let mut p = stderr_pipe;
        let _ = p.read_to_end(&mut buf);
        tx_err.send(buf).unwrap();
    });

    let _ = child.wait();

    // We expect both pipes to close within 5 seconds of child exiting.
    let out_res = rx_out.recv_timeout(Duration::from_secs(5));
    let err_res = rx_err.recv_timeout(Duration::from_secs(5));

    // Cleanup fake server process if PID file was created
    if let Ok(pid_str) = fs::read_to_string(pid_file_path()) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
        let _ = fs::remove_file(pid_file_path());
    }

    assert!(
        out_res.is_ok(),
        "stdout pipe did not close within 5 seconds after helper exited"
    );
    assert!(
        err_res.is_ok(),
        "stderr pipe did not close within 5 seconds after helper exited"
    );

    let stdout_bytes = out_res.unwrap();
    let stdout_str = String::from_utf8_lossy(&stdout_bytes);
    assert!(
        stdout_str.contains("helper finished"),
        "stdout missing helper finished: {stdout_str}"
    );
}
