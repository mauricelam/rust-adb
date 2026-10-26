use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Golden test case document structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenCase {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
}

static PROG_NAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(\s*)adb(?:\.exe)?:").unwrap());

// Match tcp:5037 or tcp:localhost:5037 (port must be > 0 so tcp:0 in help text is preserved)
static TCP_PORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(tcp:(?:[a-zA-Z0-9\.]+:)?)[1-9]\d*").unwrap());

static VERSION_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(Android Debug Bridge version )\S+").unwrap());

static VERSION_LINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(Version )\S+").unwrap());

static INSTALLED_AS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(Installed as )\S+").unwrap());

static RUNNING_ON_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(Running on ).*").unwrap());

/// Rule 1: Normalize line endings CRLF -> LF.
pub fn normalize_line_endings(input: &str) -> String {
    input.replace("\r\n", "\n")
}

/// Rule 2: Replace program name prefix in error lines (`adb:` or `adb.exe:`) with `<prog>:`.
pub fn normalize_program_name(input: &str) -> String {
    PROG_NAME_RE.replace_all(input, "${1}<prog>:").to_string()
}

/// Rule 3: Replace server ports in messages such as `tcp:<port>` with `tcp:<port>`.
pub fn normalize_server_ports(input: &str) -> String {
    TCP_PORT_RE.replace_all(input, "${1}<port>").to_string()
}

/// Rule 4: Replace temporary directory paths with `<tmp>`.
pub fn normalize_temp_paths(input: &str) -> String {
    let mut s = input.to_string();

    // Unix temp path patterns: /tmp/... or /var/tmp/... or /var/folders/...
    let unix_temp_re =
        Regex::new(r"/(?:tmp|var/tmp|var/folders/[^/]+/[^/]+/[A-Z0-9]+)/[a-zA-Z0-9._-]+").unwrap();
    s = unix_temp_re.replace_all(&s, "<tmp>").to_string();

    // Windows temp path patterns: C:\Users\...\AppData\Local\Temp\...
    let win_temp_re = Regex::new(r"(?i)[a-z]:\\(?:[^\\]+\\)*temp\\[a-zA-Z0-9._-]+").unwrap();
    s = win_temp_re.replace_all(&s, "<tmp>").to_string();

    s
}

/// Rule 5: Replace version numbers and build ids in version output with `<version>`,
/// install path with `<path>`, and OS string with `<os>`.
pub fn normalize_version(input: &str) -> String {
    let s = VERSION_HEADER_RE
        .replace_all(input, "${1}<version>")
        .to_string();
    let s = VERSION_LINE_RE.replace_all(&s, "${1}<version>").to_string();
    let s = INSTALLED_AS_RE.replace_all(&s, "${1}<path>").to_string();
    RUNNING_ON_RE.replace_all(&s, "${1}<os>").to_string()
}

/// Master normalizer combining all rules in sequence.
pub fn normalize_output(input: &str) -> String {
    let s = normalize_line_endings(input);
    let s = normalize_program_name(&s);
    let s = normalize_server_ports(&s);
    let s = normalize_temp_paths(&s);
    normalize_version(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_line_endings() {
        let raw = "line1\r\nline2\r\nline3\n";
        assert_eq!(normalize_line_endings(raw), "line1\nline2\nline3\n");
    }

    #[test]
    fn test_normalize_program_name() {
        let err_unix = "adb: unknown command nosuchcommand\n";
        assert_eq!(
            normalize_program_name(err_unix),
            "<prog>: unknown command nosuchcommand\n"
        );

        let err_win = "adb.exe: unknown command nosuchcommand\n";
        assert_eq!(
            normalize_program_name(err_win),
            "<prog>: unknown command nosuchcommand\n"
        );

        let err_padded = "  adb: -s requires an argument\n";
        assert_eq!(
            normalize_program_name(err_padded),
            "  <prog>: -s requires an argument\n"
        );

        let non_err = "general commands:\n  devices [-l] list connected devices\n";
        assert_eq!(normalize_program_name(non_err), non_err);
    }

    #[test]
    fn test_normalize_server_ports() {
        let msg1 = "* daemon not running; starting now at tcp:5037\n";
        assert_eq!(
            normalize_server_ports(msg1),
            "* daemon not running; starting now at tcp:<port>\n"
        );

        let msg2 = "cannot connect to daemon at tcp:58273: Connection refused\n";
        assert_eq!(
            normalize_server_ports(msg2),
            "cannot connect to daemon at tcp:<port>: Connection refused\n"
        );

        let msg3 = "listen on given socket for adb server [default=tcp:localhost:5037]\n";
        assert_eq!(
            normalize_server_ports(msg3),
            "listen on given socket for adb server [default=tcp:localhost:<port>]\n"
        );

        let help_msg = "  tcp:<port> (<local> may be \"tcp:0\" to pick any open port)\n";
        assert_eq!(normalize_server_ports(help_msg), help_msg);
    }

    #[test]
    fn test_normalize_temp_paths() {
        let path1 = "Installed as /tmp/tmp123456/adb\n";
        assert_eq!(normalize_temp_paths(path1), "Installed as <tmp>/adb\n");

        let path2 = "Path: /var/tmp/tmp_abc123/file.txt\n";
        assert_eq!(normalize_temp_paths(path2), "Path: <tmp>/file.txt\n");
    }

    #[test]
    fn test_normalize_version() {
        let ver =
            "Android Debug Bridge version 1.0.41\nVersion 36.0.2-14143358\nInstalled as /app/adb\nRunning on Linux 6.8.0 (x86_64)\n";
        let expected =
            "Android Debug Bridge version <version>\nVersion <version>\nInstalled as <path>\nRunning on <os>\n";
        assert_eq!(normalize_version(ver), expected);
    }

    #[test]
    fn test_normalize_output_combined() {
        let raw = "adb: cannot connect to daemon at tcp:5037: Connection refused\r\nInstalled as /tmp/tmp98765/adb.exe\r\nRunning on Linux 6.8.0\r\n";
        let normalized = normalize_output(raw);
        assert_eq!(
            normalized,
            "<prog>: cannot connect to daemon at tcp:<port>: Connection refused\nInstalled as <path>\nRunning on <os>\n"
        );
    }
}
