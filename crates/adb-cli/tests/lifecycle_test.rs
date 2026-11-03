use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

fn get_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    listener.local_addr().expect("local addr").port()
}

fn adb_binary() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("current test binary");
    path.pop(); // remove test exe name
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("adb");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    path
}

#[tokio::test]
async fn test_second_instance_start_fails_when_locked() {
    let port = get_free_port();
    let adb_bin = adb_binary();

    // Start first instance in foreground nodaemon mode
    let mut child1 = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "server", "nodaemon"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server 1");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Try starting a second instance on the same port
    let output2 = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "server", "nodaemon"])
        .output()
        .expect("run server 2");

    assert!(
        !output2.status.success(),
        "Second server instance should fail to start on an occupied port"
    );

    let stderr2 = String::from_utf8_lossy(&output2.stderr);
    assert!(
        stderr2.contains("Address already in use") || stderr2.contains("could not install"),
        "stderr should mention address already in use or could not install: got '{stderr2}'"
    );

    // Clean up child1
    let _ = child1.kill();
    let _ = child1.wait();
}

#[tokio::test]
async fn test_graceful_kill_releases_socket_lock() {
    let port = get_free_port();
    let adb_bin = adb_binary();

    // Start instance in nodaemon mode
    let mut child = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "server", "nodaemon"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send kill-server command
    let kill_output = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "kill-server"])
        .output()
        .expect("run kill-server");

    assert!(kill_output.status.success(), "kill-server should succeed");

    // Server process should exit cleanly
    let exit_status = child.wait().expect("wait for child");
    assert!(exit_status.success(), "Server should exit cleanly on kill");

    // Immediately another server should be able to bind to the port
    let mut child2 = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "server", "nodaemon"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server 2");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let kill2 = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "kill-server"])
        .output()
        .expect("run kill-server 2");

    assert!(kill2.status.success());
    let _ = child2.wait();
}

#[cfg(unix)]
#[tokio::test]
async fn test_sigint_clean_shutdown() {
    let port = get_free_port();
    let adb_bin = adb_binary();

    let mut child = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "server", "nodaemon"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    tokio::time::sleep(Duration::from_millis(500)).await;

    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }

    let status = child.wait().expect("wait for child");
    assert!(status.success(), "Server should exit cleanly upon SIGINT");
}

#[tokio::test]
async fn test_server_process_version_mismatch_clean_exit() {
    let port = get_free_port();
    let adb_bin = adb_binary();

    let mut child = Command::new(&adb_bin)
        .args(["-P", &port.to_string(), "server", "nodaemon"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Connect with a client sending an incompatible version in host:version
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to server");
    stream
        .write_all(b"0011host:version:0028")
        .await
        .expect("send mismatch version request");

    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await.expect("read response");
    let resp = String::from_utf8_lossy(&buf[..n]);
    assert!(
        resp.starts_with("FAIL"),
        "Server should respond with FAIL for version mismatch, got: {resp}"
    );

    // Server process should exit cleanly
    let status = child.wait().expect("wait for child process exit");
    assert!(
        status.success(),
        "Server process should exit cleanly with status 0 on version mismatch"
    );
}
