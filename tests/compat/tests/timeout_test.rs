use compat::{get_cmd_timeout, run_command_with_timeout};
use std::env;
use std::time::Duration;

#[test]
fn test_run_command_timeout_panics_and_kills_process() {
    let result = std::panic::catch_unwind(|| {
        run_command_with_timeout(
            env!("CARGO_BIN_EXE_hang"),
            &["--hang", "param_test"],
            &[],
            Duration::from_secs(2),
        );
    });

    assert!(
        result.is_err(),
        "Expected run_command_with_timeout to panic on timeout"
    );

    let panic_payload = result.unwrap_err();
    let panic_msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        panic!("Unknown panic payload type");
    };

    assert!(
        panic_msg.contains("timed out"),
        "Panic message did not contain 'timed out': {panic_msg}"
    );
    assert!(
        panic_msg.contains("param_test"),
        "Panic message did not contain command args: {panic_msg}"
    );
    assert!(
        panic_msg.contains("HANG_STDOUT"),
        "Panic message did not contain captured stdout: {panic_msg}"
    );
    assert!(
        panic_msg.contains("HANG_STDERR"),
        "Panic message did not contain captured stderr: {panic_msg}"
    );

    let pid_line = panic_msg
        .lines()
        .find(|l| l.contains("PID="))
        .expect("Missing PID output in panic message");

    let pid_str = pid_line
        .split("PID=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();
    let pid: u32 = pid_str.parse().expect("Failed to parse PID");

    #[cfg(unix)]
    {
        let pgid_str = pid_line
            .split("PGID=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        let pgid: u32 = pgid_str.parse().expect("Failed to parse PGID");
        assert!(
            !compat::is_process_group_running(pgid),
            "Process group {pgid} should no longer be running after timeout"
        );
    }

    assert!(
        !compat::is_process_running(pid),
        "Process {pid} should no longer be running after timeout"
    );
}

#[test]
fn test_run_command_quick_success() {
    let output = run_command_with_timeout(
        env!("CARGO_BIN_EXE_hang"),
        &["--quick"],
        &[],
        Duration::from_secs(5),
    );

    assert!(output.success(), "Command should exit successfully");
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.contains("QUICK_STDOUT"),
        "Stdout should contain 'QUICK_STDOUT'"
    );
    assert!(
        output.stderr.contains("QUICK_STDERR"),
        "Stderr should contain 'QUICK_STDERR'"
    );
}

#[test]
fn test_run_command_large_output_no_deadlock() {
    let output = run_command_with_timeout(
        env!("CARGO_BIN_EXE_hang"),
        &["--large"],
        &[],
        Duration::from_secs(10),
    );

    assert!(output.success(), "Command should exit successfully");
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output.stdout.len() >= 1_000_000,
        "Stdout length should be >= 1 MiB, got {}",
        output.stdout.len()
    );
    assert!(
        output.stderr.len() >= 1_000_000,
        "Stderr length should be >= 1 MiB, got {}",
        output.stderr.len()
    );
}

#[test]
fn test_cmd_timeout_env_var_honored_and_invalid_panics() {
    // 1. Valid value
    env::set_var("COMPAT_CMD_TIMEOUT_SECS", "3");
    assert_eq!(get_cmd_timeout(), Duration::from_secs(3));

    // 2. Invalid value panics clearly
    env::set_var("COMPAT_CMD_TIMEOUT_SECS", "invalid_value");
    let result = std::panic::catch_unwind(get_cmd_timeout);

    assert!(
        result.is_err(),
        "Expected get_cmd_timeout to panic on invalid COMPAT_CMD_TIMEOUT_SECS"
    );

    let panic_payload = result.unwrap_err();
    let panic_msg = if let Some(s) = panic_payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic_payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        panic!("Unknown panic payload type");
    };

    assert!(
        panic_msg.contains("Invalid COMPAT_CMD_TIMEOUT_SECS"),
        "Panic message did not contain 'Invalid COMPAT_CMD_TIMEOUT_SECS': {panic_msg}"
    );

    env::remove_var("COMPAT_CMD_TIMEOUT_SECS");
}
