# Compatibility Test Harness (`compat`)

The `compat` test harness crate provides an isolated test environment and helpers to execute ADB client/server compatibility tests against any implementation (C++ reference binary or Rust implementation).

## Environment Variables

The harness configures client/server processes using the following environment variables:

- `ADB_CLIENT`: Path to the ADB client binary under test. Defaults to `ADB_REFERENCE`.
- `ADB_SERVER_BIN`: Path to the ADB server binary under test. Defaults to `ADB_REFERENCE`.
- `ADB_SERVER_PORT`: Optional explicit TCP port for the server under test. If omitted, the harness automatically selects a random free port for each test server instance.
- `ADB_REFERENCE`: Fallback path to the reference ADB binary (e.g., `binaries/linux/adb`).

## Running Compatibility Tests

To run all compatibility tests using the reference binary:

```bash
cargo test -p compat
```

To run compatibility tests against a custom client binary:

```bash
ADB_CLIENT=/path/to/adb cargo test -p compat
```

To run compatibility tests against a custom server binary:

```bash
ADB_SERVER_BIN=/path/to/adb cargo test -p compat
```

## Isolation & Parallel Execution

Each test server instance runs with an isolated temporary home directory (`HOME`, `USERPROFILE`, `ANDROID_ADB_HOME`, `ANDROID_USER_HOME`). Tests do not touch the user's real `~/.android` directory and can execute safely in parallel.
