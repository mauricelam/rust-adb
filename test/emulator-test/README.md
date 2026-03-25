# ADB Emulator Parity Tests

This directory contains a comprehensive testing suite for the Rust `adb` implementation, utilizing an Android Emulator to ensure parity with the reference C++ implementation.

## Prerequisites

1.  **Android SDK**: Ensure the Android SDK is installed and the `emulator` and `adb` binaries are in your `PATH`.
2.  **Android Virtual Device (AVD)**: Create an AVD (e.g., named `TestAVD`) using the Android Studio Device Manager or `avdmanager`.
3.  **Rust Toolchain**: `cargo` must be installed.

## Environment Variables

- `ANDROID_HOME`: Path to your Android SDK root.
- `RS_ADB_AVD_NAME`: (Optional) The name of the AVD to use for automated testing (default: uses existing `RS_ADB_SERIAL`).
- `RS_ADB_SERIAL`: (Default: `emulator-5554`) The serial of the device to run tests against.

## Running Tests

### Automated Lifecycle (Emulator Startup)

The `EmulatorGuard` can automate the startup and teardown of an emulator. To use it in a test script:

```rust
use emulator_test::EmulatorGuard;

fn test_with_emulator() {
    let adb_path = "../../binaries/linux/adb";
    let guard = EmulatorGuard::new("TestAVD", adb_path, 5554).expect("Failed to start emulator");
    // Run your tests here...
}
```

### Manual Parity Testing

If an emulator is already running, you can run the parity suite directly:

```bash
cd test/emulator-test
export RS_ADB_SERIAL=emulator-5554
cargo test -- --test-threads=1
```

### Stress Testing

The stress suite tests concurrent shell sessions and large data transfers:

```bash
cd test/emulator-test
cargo test --test stress -- --test-threads=1
```

## Features Tested

- **Shell (V2/PTY)**: Multiplexing of `stdout`/`stderr`, exit codes, and PTY allocation (-t, -T).
- **File Sync**: Recursive `push`/`pull` and large file (100MB+) data integrity.
- **Concurrency**: Multiple simultaneous ADB sessions against a single device.
- **Protocol Parity**: Strict comparison of response codes and output against the reference C++ `adb`.
