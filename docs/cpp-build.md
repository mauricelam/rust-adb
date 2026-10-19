# C++ ADB Standalone Build (Task 0.10b)

## 1. Overview

To support mutation testing (Issue 10), we require a C++ `adb` executable that can be compiled from source and deliberately modified (mutated) to prove that compatibility tests catch behavioral breakages.

Building `original/` directly outside the Android Open Source Project (AOSP) build environment (Soong) is impractical due to missing internal dependencies (such as `libbase`, `libcutils`, `liblog`, `libutils`, `libandroidfw`, `libziparchive`, `libcrypto_utils`, `libmdnssd`, Open Screen libraries, generated version headers, and fastdeploy Java agents).

Instead, we build C++ `adb` from source using the standalone CMake project [`nmeum/android-tools`](https://github.com/nmeum/android-tools), pinned to release `36.0.1`.

---

## 2. Selected Release & Decision

- **Source Repository:** `nmeum/android-tools`
- **Release Version:** `36.0.1`
- **Tarball URL:** `https://github.com/nmeum/android-tools/releases/download/36.0.1/android-tools-36.0.1.tar.xz`
- **Tarball SHA-256:** `38e8a84b739480141de0836bf6d581b3339ac7d53d0f7ce8c044a3368c8c2f8f`
- **License:** Apache-2.0 (vendored fastboot BSD-2)

Mutants for mutation testing are applied as unified diffs against the vendored ADB source code in `vendor/adb` prior to building. The Rust port continues to refer to `original/` as the standard reference for behavior.

---

## 3. Required Apt Packages (Ubuntu / Debian Linux)

Building `adb` from `android-tools` on Ubuntu requires the following system packages:

```bash
sudo apt-get update && sudo apt-get install -y \
  build-essential \
  cmake \
  ninja-build \
  pkg-config \
  libabsl-dev \
  libbrotli-dev \
  libfmt-dev \
  libgtest-dev \
  liblz4-dev \
  libprotobuf-dev \
  libssl-dev \
  libunwind-dev \
  libusb-1.0-0-dev \
  libzstd-dev \
  protobuf-compiler \
  zlib1g-dev
```

*Note:* `ANDROID_TOOLS_USE_BUNDLED_LIBUSB=ON` is used during CMake configuration to ensure compatibility with libusb super-speed header extensions.

---

## 4. Build Script Usage

The build process is automated via `scripts/build-cpp-adb.sh`:

```bash
# Basic build
./scripts/build-cpp-adb.sh

# Build with custom mutant patches applied against vendor/adb
./scripts/build-cpp-adb.sh --patch path/to/mutant1.patch --patch path/to/mutant2.patch
```

The script:
1. Downloads the pinned tarball to `target/cpp-build/dl/`.
2. Verifies the tarball SHA-256 hash.
3. Extracts a fresh clean work directory into `target/cpp-build/work/`.
4. Applies any supplied `--patch` files inside `vendor/adb`.
5. Configures CMake (`-DBUILD_SHARED_LIBS=OFF -DANDROID_TOOLS_USE_BUNDLED_LIBUSB=ON`).
6. Builds the `adb` target using Ninja.
7. Prints the absolute path to the compiled binary (`target/cpp-build/work/android-tools-36.0.1/build/vendor/adb`).

---

## 5. Observed Output & Behavior Differences

When running `./target/cpp-build/work/android-tools-36.0.1/build/vendor/adb version`:

```text
Android Debug Bridge version 1.0.41
Version 36.0.1-android-tools
Installed as /app/target/cpp-build/work/android-tools-36.0.1/build/vendor/adb
Running on Linux 6.8.0 (x86_64)
```

Compared to the prebuilt reference binary (`binaries/linux/adb version` which prints `Version 36.0.2-14143358`), the built binary prints version string `36.0.1-android-tools`.

The compatibility test suite (`cargo test -p compat`) passes completely against the built binary because test assertions check for general version header matches (`Android Debug Bridge version` / `Version`) rather than rigid build numbers.
