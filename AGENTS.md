# AGENTS.md

## Overview

This project is a modern, safe, and performant implementation of the Android Debug Bridge (ADB) protocol in Rust, designed for strict behavioral parity with upstream C++ ADB. For key architectural decisions and guidelines, refer to [docs/decisions.md](docs/decisions.md). The repository is structured as a Cargo workspace consisting of six crates under `crates/` (the obsolete legacy `rust/` directory has been removed):

- `adb-proto`: Wire protocol packet definitions, serialization, and framing logic.
- `adb-auth`: Key generation, RSA key handling, and authentication protocols.
- `adb-transport`: Connection and transport interfaces for USB, TCP/IP, and mDNS.
- `adb-server`: Host server daemon implementation and service routing.
- `adb-client`: Client-side library for connecting to and interacting with the ADB server.
- `adb-cli`: Command-line interface binary (`adb`) parsing arguments and invoking client services.

## Commands

Run the following script to execute all local CI checks (formatting, linting, unit testing, and compatibility tests):

```bash
./check.sh
```

Alternatively, individual checks can be executed using Cargo:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Reference ADB Binary

The reference C++ ADB binary can be obtained using `scripts/fetch-reference-adb.sh` or by referencing the pre-built binaries stored in the repository under `binaries/`. Set the `ADB_REFERENCE` environment variable to point to the reference binary for your operating system:

- **Linux**:
  ```bash
  export ADB_REFERENCE="$(pwd)/binaries/linux/adb"
  ```
- **macOS**:
  ```bash
  export ADB_REFERENCE="$(pwd)/binaries/mac/adb"
  ```
- **Windows (PowerShell)**:
  ```powershell
  $env:ADB_REFERENCE="$(Get-Location)\binaries\windows\adb.exe"
  ```

To verify the reference ADB binary:

```bash
"$ADB_REFERENCE" version
```

### Compatibility Testing

The `tests/compat` crate provides a test harness for running compatibility tests against any ADB implementation (C++ reference or Rust).

To run compatibility tests:

```bash
cargo test -p compat
```

The harness respects the following environment variables:
- `ADB_CLIENT`: Path to client binary under test (defaults to `ADB_REFERENCE`).
- `ADB_SERVER_BIN`: Path to server binary under test (defaults to `ADB_REFERENCE`).
- `ADB_SERVER_PORT`: Explicit server port (defaults to an automatically allocated free port).
- `ADB_REFERENCE`: Path to reference C++ ADB binary.

## Rules

- **Cargo only**: All crates, binaries, and tests are built and run using Cargo exclusively. Soong and external build systems are not used.
- **No external downloads**: Builds and CI must not download external sources or binaries from URLs.
- **Test first**: A compatibility test counts only after it passes against the C++ ADB on both sides and fails against a deliberately broken C++ build.
- **Fixtures and golden outputs**: Fixtures and golden outputs are never edited without human approval.
- **Task pull requests**: Submit one task per pull request, with the task ID in the title (e.g., `0.4: Add AGENTS.md`).
- **Documentation**: All modules and functions must be documented. If a file being modified is missing documentation, add documentation to those other members even if they are not part of the change.
- **Run CI checks before creating PR**: Every agent must run `./check.sh` and ensure all checks pass prior to creating a pull request or finalizing work.

## When Blocked

If a task needs hardware, an emulator, or a network setup that your environment lacks, do everything else possible and explicitly state the gap in the pull request description.
