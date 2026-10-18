# C++ ADB Standalone Build Spike (Task 0.10a)

## 1. Overview

This spike investigates building the host C++ `adb` executable directly from `original/` outside the Android Open Source Project (AOSP) build environment (Soong / `Android.bp`). The primary goal is to establish a standalone Linux build mechanism for `adb` that can be compiled and modified during mutation testing (task 0.10a-2 and subsequent tasks).

This issue is documentation-only; no build files or code modifications are added in this task.

---

## 2. Target Identification & Dependency Analysis

### Host Binary Target
The target in `original/Android.bp` that produces the host ADB binary (client and server in a single executable) is:
- **Module Name:** `adb` (`cc_binary_host`)
- **Defaults Chain:** `adb` -> `adb_binary_host_defaults` -> `adb_defaults`

### Source Files
For host Linux builds, the target compiles the following source files:

1. **Direct Sources (`adb_binary_host_defaults`):**
   - `client/adb_client.cpp`
   - `client/bugreport.cpp`
   - `client/commandline.cpp`
   - `client/file_sync_client.cpp`
   - `client/main.cpp`
   - `client/console.cpp`
   - `client/adb_install.cpp`
   - `client/line_printer.cpp`
   - `client/fastdeploy.cpp`
   - `client/fastdeploycallbacks.cpp`
   - `client/incremental.cpp`
   - `client/incremental_server.cpp`
   - `client/incremental_utils.cpp`
   - `shell_service_protocol.cpp`

2. **Generated Header Assets:**
   - `deployagent.inc` (generated via `bin2c_fastdeployagent` from `:deployagent` and `bin2c_fastdeployagentscript` from `fastdeploy/deployagent/deployagent.sh`)
   - `build/version.h` / `version.h` (generated via `platform_tools_version`)

3. **Sources from Internal Static Libraries (within `original/`):**
   - **`libadb_host` (`original/Android.bp`):**
     - Common core: `adb.cpp`, `adb_io.cpp`, `adb_listeners.cpp`, `adb_mdns.cpp`, `adb_trace.cpp`, `adb_unique_fd.cpp`, `adb_utils.cpp`, `apacket_reader.cpp`, `fdevent/fdevent.cpp`, `services.cpp`, `sockets.cpp`, `socket_spec.cpp`, `sysdeps/env.cpp`, `sysdeps/errno.cpp`, `transport.cpp`, `transport_fd.cpp`, `types.cpp`
     - Host client/transport: `client/openscreen/mdns_service_info.cpp`, `client/openscreen/platform/logging.cpp`, `client/openscreen/platform/task_runner.cpp`, `client/openscreen/platform/udp_socket.cpp`, `client/auth.cpp`, `client/adb_wifi.cpp`, `client/detach.cpp`, `client/usb_libusb.cpp`, `client/usb_libusb_device.cpp`, `client/usb_libusb_hotplug.cpp`, `client/usb_libusb_inhouse_hotplug.cpp`, `client/transport_emulator.cpp`, `client/mdnsresponder_client.cpp`, `client/mdns_utils.cpp`, `client/transport_mdns.cpp`, `client/transport_usb.cpp`, `client/pairing/pairing_client.cpp`
     - Target Linux: `client/usb_linux.cpp`, `fdevent/fdevent_epoll.cpp`
     - Target POSIX: `sysdeps_unix.cpp`, `sysdeps/posix/network.cpp`
   - **`libadb_crypto` (`original/crypto/`):** `key.cpp`, `rsa_2048_key.cpp`, `x509_generator.cpp`
   - **`libadb_host_protos` (`original/proto/`):** `adb_host.proto`
   - **`libadb_pairing_auth` (`original/pairing_auth/`):** `aes_128_gcm.cpp`, `pairing_auth.cpp`
   - **`libadb_pairing_connection` (`original/pairing_connection/`):** `pairing_connection.cpp`
   - **`libadb_protos` (`original/proto/`):** `adb_known_hosts.proto`, `key_type.proto`, `pairing.proto`
   - **`libadb_sysdeps` (`original/`):** `sysdeps/env.cpp`
   - **`libadb_tls_connection` (`original/tls/`):** `adb_ca_list.cpp`, `tls_connection.cpp`
   - **`libfastdeploy_host` (`original/fastdeploy/`):** `apk_archive.cpp`, `deploy_patch_generator.cpp`, `patch_utils.cpp`, `ApkEntry.proto`
   - **`libapp_processes_protos_full` (`original/proto/`):** `app_processes.proto`

---

### Transitive Library Dependency Accounting

The table below accounts for every library dependency declared in `original/Android.bp` and submodules, following `static_libs`, `shared_libs`, and `header_libs` transitively.

| Library | Type / Origin | Standalone Source Strategy | AOSP-Only? | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **`libadb_crypto`** | Internal (`original/crypto/`) | Built from `original/crypto/` | No | Internal ADB module |
| **`libadb_host`** | Internal (`original/`) | Built from `original/` | No | Internal ADB host library |
| **`libadb_host_protos`** | Internal (`original/proto/`) | Built from `proto/adb_host.proto` | No | Protobuf generation |
| **`libadb_pairing_auth`** | Internal (`original/pairing_auth/`) | Built from `original/pairing_auth/` | No | Internal ADB pairing auth |
| **`libadb_pairing_connection`**| Internal (`original/pairing_connection/`)| Built from `original/pairing_connection/`| No | Internal ADB pairing connection |
| **`libadb_protos`** | Internal (`original/proto/`) | Built from `proto/*.proto` | No | Protobuf generation |
| **`libadb_sysdeps`** | Internal (`original/`) | Built from `sysdeps/env.cpp` | No | Internal sysdeps module |
| **`libadb_tls_connection`** | Internal (`original/tls/`) | Built from `original/tls/` | No | Internal ADB TLS connection |
| **`libfastdeploy_host`** | Internal (`original/fastdeploy/`)| Built from `original/fastdeploy/`| No | FastDeploy host library |
| **`libapp_processes_protos_full`** | Internal (`original/proto/`)| Built from `proto/app_processes.proto`| No | Protobuf generation |
| **`libandroidfw`** | External AOSP (`frameworks/base/libs/androidfw`) | Fetch source from AOSP | **Yes** | Required for APK/Zip parsing in FastDeploy |
| **`libbase`** | External AOSP (`system/libbase`) | Fetch source from AOSP | **Yes** | Android C++ base library (`android::base`) |
| **`libbrotli`** | External Third-Party | Distribution Package (`libbrotli-dev`) | No | System brotli library |
| **`libcrypto`** | External Third-Party | Distribution Package (`libssl-dev` / OpenSSL/BoringSSL) | No | Crypto library |
| **`libcrypto_utils`** | External AOSP (`system/core/libcrypto_utils`) | Fetch source from AOSP | **Yes** | Android RSA pubkey conversion utilities |
| **`libcutils`** | External AOSP (`system/core/libcutils`) | Fetch source from AOSP | **Yes** | Android C utilities (`cutils/sockets.h`, threads) |
| **`libdiagnose_usb`** | External AOSP (`system/core/diagnose_usb`) | Fetch source from AOSP | **Yes** | USB permission diagnosis output |
| **`liblog`** | External AOSP (`system/logging/liblog`) | Fetch source from AOSP | **Yes** | Android logging abstraction (`__android_log_print`) |
| **`liblz4`** | External Third-Party | Distribution Package (`liblz4-dev`) | No | System LZ4 library |
| **`libmdnssd`** | External Third-Party/AOSP (`external/mdnsresponder`) | Fetch source or custom headers | **Yes** | mDNSResponder client library |
| **`libopenscreen-discovery`** | External Third-Party (`external/openscreen`) | Fetch source from OpenScreen repo | No | OpenScreen mDNS discovery implementation |
| **`libopenscreen-platform-impl`** | External Third-Party (`external/openscreen`) | Fetch source from OpenScreen repo | No | OpenScreen platform adapters |
| **`libprotobuf-cpp-full`** | External Third-Party | Distribution Package (`libprotobuf-dev`, `protobuf-compiler`) | No | Protocol Buffers C++ runtime |
| **`libssl`** | External Third-Party | Distribution Package (`libssl-dev` / OpenSSL/BoringSSL) | No | SSL/TLS library |
| **`libusb`** | External Third-Party | Distribution Package (`libusb-1.0-0-dev`)| No | Host USB library |
| **`libutils`** | External AOSP (`system/core/libutils`) | Fetch source from AOSP | **Yes** | Android utilities (`RefBase`, `Compat.h`, `Vector`) |
| **`libz`** | External Third-Party | Distribution Package (`zlib1g-dev`) | No | Standard zlib compression library |
| **`libziparchive`** | External AOSP (`system/core/libziparchive`) | Fetch source from AOSP | **Yes** | Android zip archive processing |
| **`libzstd`** | External Third-Party | Distribution Package (`libzstd-dev`) | No | Zstandard compression library |
| **`libpthread`** | Host System Linker | Distribution / Toolchain (`-lpthread`) | No | POSIX threads |
| **`libresolv`** | Host System Linker | Distribution / Toolchain (`-lresolv`) | No | Base64 decode (`b64_pton`) |
| **`libutil`** | Host System Linker | Distribution / Toolchain (`-lutil`) | No | PTY support (`forkpty`) |
| **Device-Only Libs** (`libprocessgroup`, `libselinux`, `libadbd_auth`, `libadbd_fs`, `libasyncio`, `libcutils_sockets`, `libcap`, `libminijail`, `libbinder`) | Device/Daemon Targets (`adbd`, `abb`) | **Not Needed** for Host `adb` | No | Excluded from host build |

---

## 3. Options Considered for Standalone C++ Build

### Option 1: Custom CMake Build with AOSP Dependency Fetch Script (Recommended)
- **Description:** Provide a `CMakeLists.txt` for `original/` accompanied by a fetch script (`scripts/fetch-aosp-deps.sh`) that clones the minimal required set of AOSP support repositories (`system/libbase`, `system/core`, `frameworks/base`, `external/openscreen`, `external/mdnsresponder`) pinned to tag `platform-tools-36.0.2` / Android 15 revision. Standard third-party dependencies (`protobuf`, `openssl`, `libusb`, `brotli`, `lz4`, `zstd`, `zlib`) are satisfied via standard host distribution packages (`apt`).
- **Build System:** CMake + Ninja/Make.
- **How AOSP-only libraries are provided:** Downloaded into a local directory (e.g. `deps/`) or CMake `FetchContent`, compiling `libbase`, `libcutils`, `libutils`, `libziparchive`, `libcrypto_utils`, `libdiagnose_usb`, `liblog`, and `libandroidfw` as static sub-targets.
- **Revisions Supported:** Exactly matches `original/` (`platform-tools-36.0.2` / Build 14143358).
- **License:** Apache-2.0.
- **Pros:**
  - Direct control over compiler flags, include paths, and preprocessor definitions (`-DADB_HOST=1`).
  - Guaranteed exact revision matching with `original/` and reference binaries.
  - Fewest moving parts and fast iteration cycle for mutation testing (`ninja` recompiles modified C++ files in seconds).
- **Cons:** Requires creating and maintaining `CMakeLists.txt` and the dependency fetch script.

### Option 2: Existing Community Standalone Build (e.g. `nmeum/android-tools`)
- **Description:** Adopt a community CMake project (such as `android-tools` maintained by Linux distribution packagers like `nmeum`) that builds host Android tools standalone outside AOSP.
- **Build System:** CMake.
- **How AOSP-only libraries are provided:** Vendors trimmed snapshots of `system/core`, `system/libbase`, `frameworks/base`, etc.
- **Revisions Supported:** Tracks specific Linux distribution packaging tags (often corresponding to older platform-tools releases, e.g., 34.x / 35.x).
- **License:** Apache-2.0 / BSD / MIT.
- **Why Not Chosen:** Community packaging trees track their own independent snapshot releases. Adapting a third-party `android-tools` repository to build our specific `original/` source tree (`platform-tools-36.0.2`) would require overriding its vendored sources or maintaining patches against an external project. Creating our own minimal CMake build is simpler and more reliable.

### Option 3: Full AOSP / Soong Tree Build
- **Description:** Checkout a full AOSP source tree and build using Soong / `m adb`.
- **Why Not Chosen:** Extremely resource-heavy (hundreds of gigabytes, hours of download and build setup), non-portable across light CI runners, and unnecessary for host-only C++ mutation testing.

---

## 4. Recommendation

**Recommended Approach:** **Option 1 (Custom CMake Build with AOSP Fetch Script)**

### Justification
Option 1 provides the fewest moving parts, exact alignment with `original/` (`platform-tools-36.0.2`), and the fastest iteration time for mutation testing on Linux. It isolates the build to host-only targets without requiring a full AOSP environment or heavy external submodules.

---

## 5. Source Revision Comparison

- **`docs/decisions.md` Pinned Revision:** Android SDK Platform-tools `36.0.2` (Build `14143358`).
- **Reference Binary (`binaries/linux/adb version`):**
  `Android Debug Bridge version 1.0.41`, `Version 36.0.2-14143358`.
- **`original/` Source Tree:** Matches the upstream C++ source code corresponding to `platform-tools-36.0.2` (Build `14143358`).

---

## 6. Follow-up Issues (Task 0.10a-2 Breakdown)

Each follow-up issue is scoped to ~30 minutes of work:

1. **Task 0.10a-2a: AOSP Dependency Fetch Script**
   - Create `scripts/fetch-aosp-deps.sh` to fetch required AOSP library repositories (`system/libbase`, `system/core`, `frameworks/base`, `external/openscreen`, `external/mdnsresponder`) at revision `36.0.2` into a local `deps/` folder.
2. **Task 0.10a-2b: CMake Configuration for Sub-Libraries**
   - Create `CMakeLists.txt` establishing build targets for AOSP sub-libraries (`libbase`, `libcutils`, `libutils`, `libziparchive`, `libcrypto_utils`, `libdiagnose_usb`, `liblog`, `libandroidfw`) and ADB internal libraries (`libadb_crypto`, `libadb_host_protos`, `libadb_pairing_auth`, `libadb_pairing_connection`, `libadb_protos`, `libadb_sysdeps`, `libadb_tls_connection`, `libfastdeploy_host`).
3. **Task 0.10a-2c: Executable Build Target & Code Generation**
   - Add genrule rules for fastdeploy assets (`deployagent.inc`, `deployagentscript.inc`) and version headers, and define the host `adb` binary executable target linking against system distribution packages (`protobuf`, `openssl`, `libusb`, `brotli`, `lz4`, `zstd`, `zlib`).
4. **Task 0.10a-2d: Build Verification & Documentation**
   - Execute C++ build on Linux, verify binary functionality (`./build/adb version`), ensure compatibility with integration tests, and update C++ build instructions in documentation.

---

## 7. Verification & Limitations

- **Web Access Note:** Active web search was unavailable in the execution environment during this spike. Information regarding third-party community builds (`nmeum/android-tools`, Debian/Arch packaging) was derived from standard packaging conventions and existing repository knowledge, and remains unverified against live external web repos.
