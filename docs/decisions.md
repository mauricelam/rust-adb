# Architectural Decisions

## 1. Scope

**Decision:** The scope covers the whole repository (client, server, adbd, shared libraries, tools, and tests) on every OS the C++ build supports, porting host-side code first and adbd last.
**Reason:** Porting the full repository ensures complete toolchain replacement, while prioritizing host-side components establishes a stable foundation before addressing device-specific complexities.

## 2. Build System

**Decision:** Use Cargo for every crate, including adbd, which is cross-compiled for Android. Soong is not used.
**Reason:** Cargo provides a unified build system and dependency management across all crates and target platforms without relying on Android build system tooling.

## 3. Async Runtime

**Decision:** Use tokio as the asynchronous runtime.
**Reason:** Tokio provides a production-grade asynchronous runtime with rich networking and I/O abstractions well suited for multiplexed ADB streams and event handling.

## 4. Minimum Supported Rust Version

**Decision:** The Minimum Supported Rust Version (MSRV) is Rust 1.94.0 (released March 2, 2026).
**Reason:** Setting the MSRV to the current stable release on project initiation ensures access to modern language features and standard library capabilities.

## 5. Parity Target

**Decision:** Maintain behavioral parity with the C++ ADB, including identical wire behavior, CLI output text, and exit codes.
**Reason:** Behavioral parity ensures seamless compatibility with existing scripts, Android Studio, Gradle, and external tooling that rely on ADB client and server interface nuances.

### Intentional Differences

- **Client Timeouts and Stream Detachment**: `adb-client` implements configurable connection timeouts (`connect_timeout` default 10s), handshake timeouts for smart-socket status and version reads (`handshake_timeout` default 30s), and server launch acknowledgment timeouts (`ack_timeout` default 15s). Additionally, background server processes are spawned with stdio detached (`null` streams). The C++ client has no client-side timeouts or stream detachment of its own.

## 6. Test-First Rule

**Decision:** A compatibility test counts only after it has passed against the C++ ADB on both sides and failed against a deliberately broken C++ build.
**Reason:** This verification process proves test validity and sensitivity before Rust code is written, ensuring the test suite effectively catches regressions and incompatibilities.

## 7. Crate Layout

**Decision:** The project is organized into six crates: `adb-proto`, `adb-auth`, `adb-transport`, `adb-server`, `adb-client`, and `adb-cli`.
**Reason:** This layout separates protocol logic, authentication, transport interfaces, server routing, client connections, and CLI binary handling into clean, testable layers.

## 8. Open Items

**Decision:** Defer resolution of specific upstream revision pinning and mutation kill-rate targets to their respective dedicated tasks.
**Reason:** These parameters depend on upstream repository synchronization and baseline mutation testing results that are established in subsequent setup tasks.

- Pinned upstream ADB revision: Android SDK Platform-tools 36.0.2 (Build 14143358)
- Mutation kill-rate threshold: 100% (3/3 mutants killed, 0 survived in baseline task 0.10/0.11)

## 9. Key Comment Resolution in adb-auth

**Decision:** `default_comment()` resolves the host name and login name by checking standard environment variables (`HOSTNAME`/`LOGNAME`/`USER` on Unix, `COMPUTERNAME`/`USERNAME` on Windows) and falling back to OS C/Win32 APIs (`gethostname`, `getlogin_r` on Unix via `libc`; `GetComputerNameW`, `GetUserNameW` on Windows) when environment variables are not set or not exported.
**Reason:** Unexported shell variables like `HOSTNAME` in non-interactive subshells previously caused default comments to resolve as `unknown@unknown`. Using OS identity APIs mirrors C++ ADB (`GetLoginNameUTF8` and `GetHostNameUTF8` in `sysdeps/env.cpp`).

## 10. C++ Build Source for Mutation Testing

**Decision:** Use the vendored `android-tools-cmake` submodule (`37.0.0p1`) rather than building `original/` directly or using external tarballs. Mutants for mutation testing are applied as unified diff patches against `android-tools-cmake/vendor/adb`.
**Reason:** The standalone C++ build from `android-tools-cmake` builds cleanly outside AOSP, vendors required support libraries, and provides a reproducible target for mutation testing on Linux.

## 11. Interactive Shell Terminal Handling on Unix

**Decision:** `adb-client` implements Unix interactive shell terminal handling matching C++ `commandline.cpp`. Local terminal raw mode (`libc::cfmakeraw`) is managed via a RAII `RawTerminalGuard` that restores original `libc::termios` settings on drop (including early returns and panics). PTY allocation selection (`PtyMode::Auto`, `No`, `Yes`, `Definitely`) evaluates `is_interactive`, `stdin_isatty`, and device shell v2 support. Initial window size and subsequent size changes triggered via `SIGWINCH` are formatted as `<rows>x<cols>,<xpixels>x<ypixels>\0` (`format_window_size`) and sent over `ShellPacketId::WindowSizeChange` packets. Stdin input is parsed through `EscapeStateMachine`, swallowing `~` at start-of-line and ending the session with status 0 upon `~.`. For unit and integration testing, pseudo-terminal pairs (`libc::openpty`) are used to verify raw terminal transitions, window size signals, and disconnect sequences against `mock-adbd`.
**Reason:** This ensures exact terminal mode and escape sequence parity with standard ADB without leaving local terminals in a corrupt state upon disconnect or error.

## 12. Global CLI Option Parsing

**Decision:** Global options in `adb-cli` are parsed by hand rather than using `clap`.
**Reason:** The C++ ADB global option parser in `adb_commandline` (`original/client/commandline.cpp`) exhibits specific quirks (e.g., `-s123`, `-t123`, accepting global options strictly before the command name, and exact error strings) that standard argument parsing libraries such as `clap` cannot replicate without breaking parity.
