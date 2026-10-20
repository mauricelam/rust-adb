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

## 9. C++ Build Source for Mutation Testing

**Decision:** Use the vendored `android-tools-cmake` submodule (`37.0.0p1`) rather than building `original/` directly or using external tarballs. Mutants for mutation testing are applied as unified diff patches against `android-tools-cmake/vendor/adb`.
**Reason:** The standalone C++ build from `android-tools-cmake` builds cleanly outside AOSP, vendors required support libraries, and provides a reproducible target for mutation testing on Linux.
