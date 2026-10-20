# C++ ADB Standalone Build (Task 0.10b)

## 1. Overview

This document describes the standalone C++ `adb` build setup using the vendored `android-tools-cmake` submodule (`37.0.0p1`). This build allows compiling C++ `adb` from source on Linux and applying mutation patches to `vendor/adb` during mutation testing (Issue 10).

`original/` cannot be built directly without a full AOSP build tree because it lacks external dependencies (e.g. `libbase`, `libcutils`, `libziparchive`, `liblog`, `libutils`, `libcrypto_utils`, OpenScreen, Protobuf, etc.). The `android-tools-cmake` submodule vendors these dependencies and provides a CMake build for standalone host platforms.

---

## 2. Build Requirements & Package Dependencies

To build C++ `adb` from source on a clean Ubuntu runner, install the following `apt` packages:

```bash
sudo apt-get update && sudo apt-get install -y \
  cmake \
  ninja-build \
  libfmt-dev \
  libprotobuf-dev \
  protobuf-compiler \
  libbrotli-dev \
  liblz4-dev \
  libpcre2-dev \
  libzstd-dev \
  zlib1g-dev \
  libusb-1.0-0-dev \
  libgtest-dev
```

---

## 3. CMake Options & Configuration

The build script invokes CMake with the following key options:

- `-GNinja`: Uses Ninja generator for fast incremental builds.
- `-DBUILD_SHARED_LIBS=OFF`: Statically links dependencies where possible.
- `-DANDROID_TOOLS_USE_BUNDLED_LIBUSB=ON`: Uses bundled libusb source in `android-tools-cmake/vendor/libusb` to avoid host driver mismatches.
- `-DANDROID_TOOLS_PATCH_VENDOR=OFF`: Disables `android-tools-cmake` auto-applying internal patches during CMake generation, allowing the build script to manage patch application directly via `git am` or `patch`.

---

## 4. Build Script Usage

`scripts/build-cpp-adb.sh` automates git submodule initialization, optional patch application, CMake generation, Ninja compilation, and output path reporting.

### Command Example

```bash
# Clean build without patches
./scripts/build-cpp-adb.sh

# Apply mutation patches against vendor/adb before building
./scripts/build-cpp-adb.sh --patch my_mutant.patch
```

The compiled binary path is printed to stdout (e.g., `/path/to/target/cpp-build/work/vendor/adb`).

---

## 5. Version Output & Prebuilt Differences

### Built C++ Binary Version
```
Android Debug Bridge version 1.0.41
Version 37.0.0p1-android-tools
Installed as /app/target/cpp-build/work/vendor/adb
Running on Linux 6.8.0 (x86_64)
```

### Reference Binary (`binaries/linux/adb`) Version
```
Android Debug Bridge version 1.0.41
Version 36.0.2-14143358
Installed as /app/binaries/linux/adb
Running on Linux 6.8.0 (x86_64)
```

### Observed Differences
1. **Version Revision String:** Built binary reports `37.0.0p1-android-tools` while prebuilt reference reports `36.0.2-14143358`.
2. **Behavioral Compatibility:** All tests in `tests/compat` pass identically against both binaries, as protocol framing, smart sockets, and service handling remain 100% compatible.
