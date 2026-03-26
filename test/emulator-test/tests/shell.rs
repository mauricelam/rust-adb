use emulator_test::harness::AdbTestHarness;
use anyhow::Result;

#[test]
fn test_shell_v2_multiplexing() -> Result<()> {
    // Note: In real scenarios, EmulatorGuard would be used in a #[tokio::test] or via a global fixture.
    // For these unit-test like structure, we assume an emulator is already running if RS_ADB_AVD_NAME is set.
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    // Test stdout/stderr separation
    let args = ["shell", "echo foo; echo bar >&2"];
    harness.run_parity(&args)?;

    Ok(())
}

#[test]
fn test_shell_exit_code() -> Result<()> {
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    let args = ["shell", "exit 42"];
    harness.run_parity(&args)?;

    Ok(())
}

#[test]
fn test_shell_pty_allocation() -> Result<()> {
    let serial = std::env::var("RS_ADB_SERIAL").unwrap_or_else(|_| "emulator-5554".to_string());
    let harness = AdbTestHarness::new(&serial)?;

    // -tt should allocate a PTY, causing [ -t 0 ] to return 0
    let args = ["shell", "-tt", "[ -t 0 ]"];
    harness.run_parity(&args)?;

    // -T should NOT allocate a PTY
    let args = ["shell", "-T", "[ -t 0 ]"];
    let rust_out = harness.run_rust(&args)?;
    let orig_out = harness.run_original(&args)?;

    assert_ne!(rust_out.status.code(), Some(0));
    assert_ne!(orig_out.status.code(), Some(0));

    Ok(())
}
