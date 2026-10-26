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
    exit_cleanly(0);
}
