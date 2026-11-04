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
- Mutation kill-rate threshold: 91.7% (33/36 mutants killed, 3 survived/equivalent)
  - Survivors documented as equivalent or unexercised by client-side tests:
    - 11-header-magic-check.patch: Modifies check_header validation logic in daemon transport when running against well-formed mock/remote traffic.
    - 12-payload-capping-logic.patch: Modifies transport max_payload capping logic where MAX_PAYLOAD is already negotiated identically by current mock device features.
    - 13-flow-control-okay.patch: Mutates local socket deletion of FDE_READ under un-acked WRTE packets in legacy deferred ack mode.

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

## 13. USB Library Choice: nusb vs rusb

**Decision:** Choose `nusb` as the USB library for host device enumeration, discovery, and asynchronous transfer backend implementations. No crate dependencies are added in task 6.1a; integration begins in task 6.1c behind an optional Cargo feature `usb` off by default.
**Reason:** `nusb` is a pure-Rust, asynchronous-first USB library designed from the ground up to integrate with asynchronous Rust runtimes (such as `tokio`). Unlike `rusb` (which wraps C `libusb-1.0` via FFI and blocking event loops), `nusb` interacts directly with kernel USB APIs on Linux (`usbfs`), macOS (`IOKit` / `IOUSBHost`), and Windows (`WinUSB`), completely avoiding C build toolchain dependencies, static C library vendoring, and thread-pool bridging for asynchronous transfers.

### Comparison Matrix

| Evaluation Dimension | `nusb` (v0.2.x) | `rusb` (v0.9.x) | Analysis / Advantage |
|---|---|---|---|
| **Async Fit with Tokio** | **Native async**: Asynchronous transfers return `Future`s driven by OS event completion (e.g. io_uring/epoll on Linux usbfs, completion ports/overlapped on Windows, runloop/dispatch on macOS). Optional `tokio` feature. | **Synchronous / Blocking C FFI**: Wrapped around `libusb`. Asynchronous `libusb` transfers require running a dedicated background thread pumping `libusb_handle_events` or offloading blocking calls to `tokio::task::spawn_blocking`. | **nusb**: Native async model seamlessly integrates with Tokio without worker thread pool exhaustion or custom event pump loops. |
| **Hotplug Support** | **Built-in async stream/watcher**: Provides `watch_devices()` producing device arrival/departure events across Linux (netlink/udev), macOS, and Windows. | **Conditional**: Hotplug callback registration (`libusb_hotplug_register_callback`) is only supported on platforms where `libusb` supports it (Linux, macOS; unsupported by upstream `libusb` on Windows). On Windows, C++ ADB implements custom window-message power and device notification threads. | **nusb**: Uniform async hotplug event API across Linux, macOS, and Windows. |
| **Per-OS Host Coverage** | **Linux, macOS, Windows**: Pure Rust kernel drivers via `usbfs` (Linux), `IOKit` (macOS), and `WinUSB` (Windows). | **Linux, macOS, Windows**: Backed by `libusb`. | **Tie / Both support all target host OSes**. |
| **Kernel Driver Detach & Interface Claiming** | Supported on Linux: Can detach kernel drivers automatically (`detach_and_claim_interface`) or explicitly. Interface claiming supported across all OSes. | Supported on Linux: `detach_kernel_driver` and `claim_interface` exposed through `libusb`. | **Tie / Both satisfy requirements**. |
| **Zero-Length Packet (ZLP) & Cancellation** | Supports ZLP flags and zero-byte writes. Asynchronous transfer `Future`s can be cancelled simply by dropping them, triggering native kernel transfer cancellation. | Supports zero-byte transfers; transfer cancellation requires tracking `libusb_transfer` structs and invoking `libusb_cancel_transfer`. | **nusb**: Drop-to-cancel semantics align naturally with Rust async task lifecycles. |
| **Transfer Timeouts** | Built-in async timeouts via standard `tokio::time::timeout` wrapping transfer futures, plus native queue timeout options. | Parameterized timeouts on synchronous calls; async libusb requires timer management. | **nusb**: Standard Tokio composability. |
| **Maintenance & Release Activity** | **Active**: Modern pure-Rust project actively maintained, responsive to issues, targeting modern Rust idioms. | **Stagnant / Slower**: Maintenance is sporadic; tightly coupled to `libusb1-sys` and upstream C `libusb`. | **nusb**: Stronger active momentum in the Rust ecosystem. |
| **License** | **Apache-2.0 OR MIT**: Fully compatible with the project license and upstream AOSP Apache-2.0. | **MIT**: Compatible with project licensing. | **Tie**. |
| **MSRV** | **Rust 1.85+**: Well within the repository MSRV policy of **Rust 1.94.0** (Decision 4). | Rust 1.63+ / unspecified. | **Both satisfy MSRV 1.94.0**. |
| **Windows Driver Requirement** | Direct **WinUSB** integration: Communicates with devices via Microsoft WinUSB driver (`winusb.sys`), matching official Android SDK / Google USB Driver setup. | Requires `libusb`-compatible drivers on Windows: Works best with WinUSB, but libusb Windows backend has historical quirks with composite devices and driver binding. | **nusb**: Direct WinUSB usage matches C++ ADB's native Windows backend (`usb_windows.cpp`). |

### Upstream C++ ADB Capabilities and Coverage

Inspection of pinned C++ sources (`original/client/usb.h`, `original/client/usb_libusb*.cpp`, `original/client/usb_linux.cpp`, `original/client/usb_osx.cpp`, `original/client/usb_windows.cpp`, `original/client/transport_usb.cpp`) reveals the following set of USB subsystem requirements:

1. **Device Enumeration and Filtering**:
   - Matches interface descriptor fields: Class `0xFF` (`ADB_CLASS`), SubClass `0x42` (`ADB_SUBCLASS`), and Protocol `0x01` (`ADB_PROTOCOL`), or Intel DbC Class `0xDC` (`ADB_DBC_CLASS`) and SubClass `0x02` (`ADB_DBC_SUBCLASS`).
   - Requires exactly two bulk endpoints (one IN, one OUT).
   - Reads device string descriptors (specifically `iSerialNumber`) to obtain the device serial.
   - Computes unique device paths (e.g. `usb:1-2.3`).
   - *Status in nusb:* Fully supported via device and configuration descriptor access and string descriptor reading before or after opening.
2. **Interface Claiming and Kernel Detach**:
   - On Linux, kernel drivers (such as cdc_acm or other composite drivers claiming interfaces) must be detached, and the ADB interface must be claimed exclusively.
   - *Status in nusb:* Fully supported via `detach_and_claim_interface`.
3. **Bulk Asynchronous I/O**:
   - Streaming 24-byte header and variable-length payload transfers over IN and OUT bulk endpoints.
   - *Status in nusb:* Fully supported via `Endpoint::make_writer()` and `Endpoint::make_reader()` or transfer queues.
4. **Zero-Length Packet (ZLP) Handling**:
   - Upstream rule: If OUT transfer length is a multiple of `wMaxPacketSize`, an explicit ZLP must be transmitted to signal packet termination: `if (zero_mask && !(len & zero_mask)) write_zlp()`.
   - *Status in nusb:* Fully supported by submitting zero-length slice transfers.
5. **Clear Halt**:
   - Upstream issues `libusb_clear_halt` on bulk IN and OUT endpoints upon claiming the interface to reset toggle bits and clear stall conditions.
   - *Status in nusb:* Endpoint stalls can be cleared via standard USB control transfers (`CLEAR_FEATURE` for `ENDPOINT_HALT`) submitted to endpoint 0.
6. **Device Reset & Kicking**:
   - `usb_reset()` calls `ioctl(fd, USBDEVFS_RESET)` on Linux or `libusb_reset_device` to trigger a bus reset on fatal protocol errors.
   - *Status in nusb:* `nusb` currently focuses on device communication; full port/bus reset is available on Linux via raw descriptor fd or ioctl, but is limited or platform-dependent on macOS and Windows (matching C++ ADB, where `usb_reset` on Windows is explicitly unimplemented and falls back to `usb_kick`).
7. **Hotplug and Power Management**:
   - Monitoring device arrival and removal without polling loops where supported; handling OS sleep/wake cycles that reset USB host controllers.
   - *Status in nusb:* Built-in async hotplug watcher; disconnection events surfaced immediately when transfer futures complete with disconnected error statuses.

### C++ Capabilities Lacking in `nusb` and Mitigations

1. **Explicit Bus Reset Method**: `nusb`'s high-level `Device` API does not currently expose a generic cross-platform `reset()` method equivalent to `libusb_reset_device()`.
   - *Mitigation:* On Linux, the underlying file descriptor can issue `USBDEVFS_RESET` if needed; on Windows, C++ ADB already does not implement bus reset (falling back to kicking the connection); on macOS, bus reset forcibly disconnects all interfaces on composite devices. In Rust ADB, connection teardown and closing the interface handles (`usb_kick`) achieves identical socket recovery without requiring whole-bus hard resets.
2. **Custom Intel DbC Debug Class Specialization**: `nusb` does not filter devices automatically; our crate-independent layer (task 6.1b) implements the descriptor matching rule for both standard ADB (`0xFF`/`0x42`/`0x01`) and DbC (`0xDC`/`0x02`/`0x01`).

### Reversal Conditions

This decision will be re-evaluated and potentially reversed in favor of `rusb` (or a direct `libusb` binding) if:
1. Production hardware testing on macOS (`IOKit`) or Windows (`WinUSB`) in Phase 6.4 uncovers insurmountable kernel communication issues, device handle leakages, or driver incompatibilities within `nusb` that cannot be resolved upstream.
2. Inability to execute full-speed and SuperSpeed bulk transfers at parity throughput with C++ ADB due to limitations in `nusb`'s internal buffer management.
3. Upstream maintenance of `nusb` halts or introduces breaking API shifts incompatible with our MSRV policy (Rust 1.94.0).
