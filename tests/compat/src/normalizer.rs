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

// Match tcp:5037 or tcp:localhost:5037 or tcp:127.0.0.1:5037
static TCP_PORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(tcp:(?:[a-zA-Z0-9\.]+:)?)[1-9]\d*").unwrap());

static VERSION_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(Android Debug Bridge version )\S+").unwrap());

static VERSION_LINE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(Version )\S+").unwrap());

static INSTALLED_AS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(Installed as )\S+").unwrap());

static RUNNING_ON_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(Running on ).*").unwrap());

// Match WSA / OS-specific connection refused error details on Windows or other OSes
static CONN_REFUSED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(cannot connect to daemon at tcp:<port>: ).*(?:refused|actively refused).*")
        .unwrap()
});

static WIN_CONN_REFUSED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"cannot connect to (<serial>|\S+): No connection could be made because the target machine actively refused it\..*")
        .unwrap()
});

// Match standalone allocated port line like "37697\n"
static STANDALONE_PORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^[1-9]\d{3,5}$").unwrap());

// Match device serials like 127.0.0.1:58273 or localhost:58273
static SERIAL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:127\.0\.0\.1|localhost):[1-9]\d*").unwrap());

// Match transport_id:1 or transport_id:42 property
static TRANSPORT_ID_PROP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"transport_id:[1-9]\d*").unwrap());

// Match transfer summary rate, bytes, and time: " 2480.9 MB/s (307200 bytes in 0.000s)"
static TRANSFER_SUMMARY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d+(?:\.\d+)? MB/s \(\d+ bytes in \d+(?:\.\d+)?s\)").unwrap());

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
    let s = TCP_PORT_RE.replace_all(input, "${1}<port>").to_string();
    STANDALONE_PORT_RE.replace_all(&s, "<port>").to_string()
}

/// Rule 7: Replace device serials such as `127.0.0.1:58273` with `<serial>`.
pub fn normalize_device_serials(input: &str) -> String {
    SERIAL_RE.replace_all(input, "<serial>").to_string()
}

/// Rule 8: Normalize transport ID property `transport_id:1` -> `transport_id:<transport_id>`.
pub fn normalize_transport_ids(input: &str) -> String {
    TRANSPORT_ID_PROP_RE
        .replace_all(input, "transport_id:<transport_id>")
        .to_string()
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

    s.replace("<tmp>\\", "<tmp>/")
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

/// Rule 9: Replace transfer rate, byte count, and elapsed time in summary lines with placeholders.
pub fn normalize_transfer_summary(input: &str) -> String {
    TRANSFER_SUMMARY_RE
        .replace_all(input, "<rate> MB/s (<bytes> bytes in <time>s)")
        .to_string()
}

// Match OS error details in parentheses like "(os error 2)"
static OS_ERROR_PAREN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r" \(os error \d+\)").unwrap());

/// Rule 6: Remove OS-specific help lines (e.g. `--local-agent` on Linux/macOS vs Windows)
/// and normalize connection refused OS error text.
pub fn normalize_os_differences(input: &str) -> String {
    let mut lines = Vec::new();
    for line in input.split('\n') {
        if line.contains("--local-agent") {
            continue;
        }
        lines.push(line);
    }
    let s = lines
        .join("\n")
        .replace(",libusb", "")
        .replace(",openscreen_mdns", "")
        .replace("openscreen_mdns,", "");
    let s = CONN_REFUSED_RE
        .replace_all(&s, "${1}Connection refused")
        .to_string();
    let s = WIN_CONN_REFUSED_RE
        .replace_all(&s, "failed to connect to '${1}': Connection refused")
        .to_string();
    OS_ERROR_PAREN_RE.replace_all(&s, "").to_string()
}

/// Master normalizer combining all rules in sequence.
pub fn normalize_output(input: &str) -> String {
    let s = normalize_line_endings(input);
    let s = normalize_program_name(&s);
    let s = normalize_server_ports(&s);
    let s = normalize_device_serials(&s);
    let s = normalize_transport_ids(&s);
    let s = normalize_temp_paths(&s);
    let s = normalize_version(&s);
    let s = normalize_transfer_summary(&s);
    normalize_os_differences(&s)
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
    fn test_normalize_os_differences() {
        let win_err = "cannot connect to daemon at tcp:<port>: cannot connect to 127.0.0.1:58273: No connection could be made because the target machine actively refused it. (10061)\n";
        assert_eq!(
            normalize_os_differences(win_err),
            "cannot connect to daemon at tcp:<port>: Connection refused\n"
        );

        let win_conn_err = "cannot connect to <serial>: No connection could be made because the target machine actively refused it. (10061)\n";
        assert_eq!(
            normalize_os_differences(win_conn_err),
            "failed to connect to '<serial>': Connection refused\n"
        );

        let host_feat = "shell_v2,cmd,libusb,push_sync\n";
        assert_eq!(
            normalize_os_differences(host_feat),
            "shell_v2,cmd,push_sync\n"
        );

        let help_text = "     --local-agent: locate agent files\n     uninstall [-k]\n";
        assert_eq!(normalize_os_differences(help_text), "     uninstall [-k]\n");
    }

    #[test]
    fn test_normalize_device_serials() {
        let conn = "connected to 127.0.0.1:41759\n";
        assert_eq!(normalize_device_serials(conn), "connected to <serial>\n");

        let list = "127.0.0.1:41759\tdevice\n";
        assert_eq!(normalize_device_serials(list), "<serial>\tdevice\n");
    }

    #[test]
    fn test_normalize_transport_ids() {
        let dev_l = "127.0.0.1:41759        device product:mock_device model:mock_model device:mock_device transport_id:1\n";
        assert_eq!(
            normalize_transport_ids(dev_l),
            "127.0.0.1:41759        device product:mock_device model:mock_model device:mock_device transport_id:<transport_id>\n"
        );
    }

    #[test]
    fn test_normalize_transfer_summary() {
        let summary_push =
            "/data/f_300k.bin: 1 file pushed, 0 skipped. 2480.9 MB/s (307200 bytes in 0.000s)\n";
        assert_eq!(
            normalize_transfer_summary(summary_push),
            "/data/f_300k.bin: 1 file pushed, 0 skipped. <rate> MB/s (<bytes> bytes in <time>s)\n"
        );

        let summary_pull = "1 file pulled, 0 skipped. 0.5 MB/s (100 bytes in 0.020s)\n";
        assert_eq!(
            normalize_transfer_summary(summary_pull),
            "1 file pulled, 0 skipped. <rate> MB/s (<bytes> bytes in <time>s)\n"
        );

        let summary_zero = "0 files pushed, 1 skipped.\n";
        assert_eq!(
            normalize_transfer_summary(summary_zero),
            "0 files pushed, 1 skipped.\n"
        );
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
