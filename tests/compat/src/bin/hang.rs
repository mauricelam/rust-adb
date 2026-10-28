use std::env;
use std::io::Write;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.iter().any(|arg| arg == "--quick") {
        println!("QUICK_STDOUT");
        let _ = std::io::stdout().flush();
        eprintln!("QUICK_STDERR");
        let _ = std::io::stderr().flush();
        return;
    }

    if args.iter().any(|arg| arg == "--large") {
        let chunk_a = vec![b'A'; 64 * 1024];
        let chunk_b = vec![b'B'; 64 * 1024];

        let stdout = std::io::stdout();
        let mut handle_out = stdout.lock();
        let stderr = std::io::stderr();
        let mut handle_err = stderr.lock();

        // Write ~1.5 MiB (24 * 64 KiB = 1,572,864 bytes) to both streams
        for _ in 0..24 {
            let _ = handle_out.write_all(&chunk_a);
            let _ = handle_err.write_all(&chunk_b);
        }
        let _ = handle_out.flush();
        let _ = handle_err.flush();
        return;
    }

    let pid = std::process::id();
    #[cfg(unix)]
    let pgid = unsafe { libc::getpgrp() };
    #[cfg(not(unix))]
    let pgid = 0;

    println!("HANG_STDOUT PID={pid} PGID={pgid}");
    let _ = std::io::stdout().flush();

    eprintln!("HANG_STDERR PID={pid} PGID={pgid}");
    let _ = std::io::stderr().flush();

    thread::sleep(Duration::from_secs(60));
}
