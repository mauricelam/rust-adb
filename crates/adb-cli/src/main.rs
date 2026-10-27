//! adb-cli: Command-line interface binary (`adb`) parsing arguments and invoking client services.

use std::io::Write;

/// Flushes standard output and standard error streams, then immediately exits the process with `code`.
///
/// Calling `exit_cleanly` prevents Tokio runtime shutdown from hanging indefinitely when background
/// tasks (such as `stdin` reader tasks) remain open.
pub fn exit_cleanly(code: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    std::process::exit(code);
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build tokio runtime");

    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_fn = |key: &str| std::env::var(key).ok();
    let mut output = adb_cli::Output::stdio();

    let code = rt.block_on(adb_cli::run_cli(&args, env_fn, &mut output));

    let _ = output.stdout.flush();
    let _ = output.stderr.flush();
    exit_cleanly(code);
}
