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
- Mutation kill-rate threshold: [Placeholder: Resolved in task 0.11]

## 9. C++ Build Source for Mutation

**Decision:** Use `nmeum/android-tools` release tarball version `36.0.1` as the C++ ADB build source for mutation testing. Mutants are applied as unified patches against `vendor/adb` prior to building with CMake (`-DBUILD_SHARED_LIBS=OFF -DANDROID_TOOLS_USE_BUNDLED_LIBUSB=ON`).
**Release Tarball URL:** `https://github.com/nmeum/android-tools/releases/download/36.0.1/android-tools-36.0.1.tar.xz`
**Tarball SHA-256:** `38e8a84b739480141de0836bf6d581b3339ac7d53d0f7ce8c044a3368c8c2f8f`
**Reason:** Building `original/` directly outside AOSP requires fetching and managing dozens of internal AOSP submodules and generated assets. `nmeum/android-tools` provides a clean standalone CMake build environment pinned close to platform-tools 36.x, allowing fast C++ builds and mutant insertion.
