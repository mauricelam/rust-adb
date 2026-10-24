#[cfg(unix)]
use std::env;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
fn pid_file_path() -> PathBuf {
    env::temp_dir().join("stream_helper_fake_server.pid")
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|a| a.contains("--no-ack")) {
        thread::sleep(Duration::from_secs(300));
        return;
    }

    if let Some(pos) = args.iter().position(|a| a == "--reply-fd") {
        let fd_str = &args[pos + 1];
        let fd: i32 = fd_str.parse().expect("invalid fd");

        let pid = std::process::id();
        let _ = fs::write(pid_file_path(), pid.to_string());

        unsafe {
            libc::write(fd, b"OK\n".as_ptr() as *const _, 3);
        }

        thread::sleep(Duration::from_secs(300));
        return;
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        use adb_client::{ExternalBinaryLauncher, ServerLauncher};
        let exe = env::current_exe().unwrap();
        let launcher = ExternalBinaryLauncher::new(exe);
        launcher.launch("tcp:5037", None).await.unwrap();
        println!("helper finished");
    });
}

#[cfg(not(unix))]
fn main() {}
