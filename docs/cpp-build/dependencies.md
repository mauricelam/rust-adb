# Host ADB Target Dependency Inventory (Task 0.10a-1)

## 1. Executive Summary & Module Identification

This document provides a comprehensive dependency inventory for the C++ host `adb` executable target. This target compiles both the ADB client and server into a single host executable.

- **Target Executable Module Name:** `adb` (`cc_binary_host`)
- **Primary Build File Location:** `original/Android.bp` (line 469)
- **Defaults Inheritance Chain:**
  1. `adb` (`cc_binary_host`)
  2. `adb_binary_host_defaults` (`cc_defaults` in `original/Android.bp`)
  3. `adb_defaults` (`cc_defaults` in `original/Android.bp`)

---

## 2. Source Files Grouped by Directory

Below is the complete list of C++ source files compiled into the host `adb` binary, either directly by `adb_binary_host_defaults` or transitively through internal static libraries defined within `original/`.

### 2.1 Direct Sources (`adb_binary_host_defaults` in `original/Android.bp`)
- **`original/`**:
  - `shell_service_protocol.cpp`
- **`original/client/`**:
  - `client/adb_client.cpp`
  - `client/adb_install.cpp`
  - `client/bugreport.cpp`
  - `client/commandline.cpp`
  - `client/console.cpp`
  - `client/fastdeploy.cpp`
  - `client/fastdeploycallbacks.cpp`
  - `client/file_sync_client.cpp`
  - `client/incremental.cpp`
  - `client/incremental_server.cpp`
  - `client/incremental_utils.cpp`
  - `client/line_printer.cpp`
  - `client/main.cpp`

### 2.2 Generated Header & Asset Inputs
- **`deployagent.inc`**: Generated via `bin2c_fastdeployagent` rule (`original/Android.bp`) from Java binary output `:deployagent` (`original/fastdeploy/Android.bp`).
- **`deployagentscript.inc`**: Generated via `bin2c_fastdeployagentscript` rule (`original/Android.bp`) from `fastdeploy/deployagent/deployagent.sh`.
- **`platform_tools_version` / `build/version.h`**: Generated header providing build/version number macros via `use_version_lib: true`.

### 2.3 Sources from Internal Static Libraries (within `original/`)

#### Library: `libadb_host` (`original/Android.bp`)
- **`original/`**:
  - `adb.cpp`
  - `adb_io.cpp`
  - `adb_listeners.cpp`
  - `adb_mdns.cpp`
  - `adb_trace.cpp`
  - `adb_unique_fd.cpp`
  - `adb_utils.cpp`
  - `apacket_reader.cpp`
  - `services.cpp`
  - `socket_spec.cpp`
  - `sockets.cpp`
  - `transport.cpp`
  - `transport_fd.cpp`
  - `types.cpp`
- **`original/client/`**:
  - `client/adb_wifi.cpp`
  - `client/auth.cpp`
  - `client/detach.cpp`
  - `client/mdns_utils.cpp`
  - `client/mdnsresponder_client.cpp`
  - `client/transport_emulator.cpp`
  - `client/transport_mdns.cpp`
  - `client/transport_usb.cpp`
  - `client/usb_libusb.cpp`
  - `client/usb_libusb_device.cpp`
  - `client/usb_libusb_hotplug.cpp`
  - `client/usb_libusb_inhouse_hotplug.cpp`
- **`original/client/openscreen/`**:
  - `client/openscreen/mdns_service_info.cpp`
- **`original/client/openscreen/platform/`**:
  - `client/openscreen/platform/logging.cpp`
  - `client/openscreen/platform/task_runner.cpp`
  - `client/openscreen/platform/udp_socket.cpp`
- **`original/client/pairing/`**:
  - `client/pairing/pairing_client.cpp`
- **`original/fdevent/`**:
  - `fdevent/fdevent.cpp`
  - Platform specific:
    - Linux host: `fdevent/fdevent_epoll.cpp`
    - macOS / Windows host: `fdevent/fdevent_poll.cpp`
- **`original/sysdeps/`**:
  - `sysdeps/env.cpp`
  - `sysdeps/errno.cpp`
  - POSIX / Unix host: `sysdeps_unix.cpp`, `sysdeps/posix/network.cpp`
  - Windows host: `sysdeps_win32.cpp`, `sysdeps/win32/errno.cpp`, `sysdeps/win32/stat.cpp`
- **Platform OS USB files**:
  - Linux host: `client/usb_linux.cpp`
  - macOS host: `client/usb_osx.cpp`
  - Windows host: `client/usb_windows.cpp`

#### Library: `libadb_crypto` (`original/crypto/Android.bp`)
- **`original/crypto/`**:
  - `key.cpp`
  - `rsa_2048_key.cpp`
  - `x509_generator.cpp`

#### Library: `libadb_host_protos` (`original/proto/Android.bp`)
- **`original/proto/`**:
  - `adb_host.proto`

#### Library: `libadb_pairing_auth` (`original/pairing_auth/Android.bp`)
- **`original/pairing_auth/`**:
  - `aes_128_gcm.cpp`
  - `pairing_auth.cpp`

#### Library: `libadb_pairing_connection` (`original/pairing_connection/Android.bp`)
- **`original/pairing_connection/`**:
  - `pairing_connection.cpp`

#### Library: `libadb_protos` (`original/proto/Android.bp`)
- **`original/proto/`**:
  - `adb_known_hosts.proto`
  - `key_type.proto`
  - `pairing.proto`

#### Library: `libadb_sysdeps` (`original/Android.bp`)
- **`original/sysdeps/`**:
  - `sysdeps/env.cpp`

#### Library: `libadb_tls_connection` (`original/tls/Android.bp`)
- **`original/tls/`**:
  - `adb_ca_list.cpp`
  - `tls_connection.cpp`

#### Library: `libfastdeploy_host` (`original/fastdeploy/Android.bp`)
- **`original/fastdeploy/deploypatchgenerator/`**:
  - `apk_archive.cpp`
  - `deploy_patch_generator.cpp`
  - `patch_utils.cpp`
- **`original/fastdeploy/proto/`**:
  - `ApkEntry.proto`

#### Library: `libapp_processes_protos_full` (`original/proto/Android.bp`)
- **`original/proto/`**:
  - `app_processes.proto`

---

## 3. Transitive Dependency Inventory Table

The table below lists every dependency linked by `adb` (following `static_libs`, `whole_static_libs`, `shared_libs`, and `header_libs` transitively). Each entry is classified as either **Defined in `original/`** or **External to repository**.

| Dependency Library | Origin / Classification | Used By (Modules) | Notes |
| :--- | :--- | :--- | :--- |
| **`libadb_crypto`** | Defined in `original/crypto/` | `adb`, `libadb_host` | ADB RSA key generation and crypto utilities |
| **`libadb_host`** | Defined in `original/` | `adb`, `libfastdeploy_host` | Internal static library containing core host ADB logic |
| **`libadb_host_protos`** | Defined in `original/proto/` | `adb`, `libadb_host` | Protobuf library compiled from `proto/adb_host.proto` |
| **`libadb_pairing_auth`** | Defined in `original/pairing_auth/` | `adb`, `libadb_pairing_connection` | AES-128-GCM authentication logic for Wi-Fi pairing |
| **`libadb_pairing_connection`** | Defined in `original/pairing_connection/` | `adb`, `libadb_host` | Peer pairing connection handler logic |
| **`libadb_protos`** | Defined in `original/proto/` | `adb`, `libadb_crypto`, `libadb_host`, `libadb_pairing_connection` | Protobuf definitions (`adb_known_hosts.proto`, `key_type.proto`, `pairing.proto`) |
| **`libadb_sysdeps`** | Defined in `original/` | `adb`, `libadb_crypto` | System dependency wrappers (`sysdeps/env.cpp`) |
| **`libadb_tls_connection`** | Defined in `original/tls/` | `adb`, `libadb_host`, `libadb_pairing_connection` | TLS socket transport and X.509 CA list handling |
| **`libfastdeploy_host`** | Defined in `original/fastdeploy/` | `adb` | FastDeploy APK diff and patch generator library |
| **`libapp_processes_protos_full`** | Defined in `original/proto/` | `adb` | Full Protobuf bindings for `app_processes.proto` |
| **`libandroidfw`** | External (AOSP `frameworks/base/libs/androidfw`) | `adb`, `libfastdeploy_host` | Android framework asset/resource/Zip file reader library |
| **`libbase`** | External (AOSP `system/libbase`) | `adb`, `libadb_crypto`, `libadb_host`, `libadb_pairing_auth`, `libadb_pairing_connection`, `libadb_sysdeps`, `libadb_tls_connection`, `libfastdeploy_host` | Android C++ base library (`android::base::*`, logging, string formatting) |
| **`libbrotli`** | External Third-Party (`libbrotli-dev`) | `adb` | Brotli compression library |
| **`libcrypto`** | External Third-Party (`libssl-dev` / BoringSSL) | `adb`, `libadb_crypto`, `libadb_host`, `libadb_pairing_auth`, `libadb_pairing_connection`, `libadb_tls_connection`, `libfastdeploy_host` | OpenSSL / BoringSSL cryptography library (`libcrypto`) |
| **`libcrypto_utils`** | External (AOSP `system/core/libcrypto_utils`) | `adb`, `libadb_crypto`, `libadb_host`, `libfastdeploy_host` | Android RSA public key format conversion utilities |
| **`libcutils`** | External (AOSP `system/core/libcutils`) | `adb`, `libadb_host`, `libfastdeploy_host` | Android C utilities (`cutils/sockets.h`, threads) |
| **`libdiagnose_usb`** | External (AOSP `system/core/diagnose_usb`) | `adb`, `libadb_host`, `libfastdeploy_host` | Diagnostics helper for USB permissions and accessibility |
| **`liblog`** | External (AOSP `system/logging/liblog`) | `adb`, `libadb_crypto`, `libadb_host`, `libadb_pairing_auth`, `libadb_pairing_connection`, `libadb_sysdeps`, `libadb_tls_connection`, `libfastdeploy_host` | Android logging interface (`__android_log_print`) |
| **`liblz4`** | External Third-Party (`liblz4-dev`) | `adb` | LZ4 frame compression library |
| **`libmdnssd`** | External (AOSP / mDNSResponder `external/mdnsresponder`) | `adb`, `libadb_host`, `libfastdeploy_host` | Apple mDNSResponder client library (`dns_sd.h`) |
| **`libopenscreen-discovery`** | External (AOSP `external/openscreen`) | `adb`, `libadb_host` | OpenScreen mDNS discovery library |
| **`libopenscreen-platform-impl`** | External (AOSP `external/openscreen`) | `adb`, `libadb_host` | OpenScreen platform network and task adapters |
| **`libprotobuf-cpp-full`** | External Third-Party (`libprotobuf-dev`) | `adb`, `libadb_host_protos` | Protocol Buffers C++ runtime library (full variant) |
| **`libprotobuf-cpp-lite`** | External Third-Party (`libprotobuf-dev`) | `libadb_host`, `libadb_pairing_connection` | Protocol Buffers C++ runtime library (lite variant) |
| **`libssl`** | External Third-Party (`libssl-dev` / BoringSSL) | `adb`, `libadb_pairing_connection`, `libadb_tls_connection` | OpenSSL / BoringSSL SSL/TLS library (`libssl`) |
| **`libusb`** | External Third-Party (`libusb-1.0-0-dev`) | `adb`, `libadb_host`, `libfastdeploy_host` | User-space USB access library (`libusb-1.0`) |
| **`libutils`** | External (AOSP `system/core/libutils`) | `adb`, `libadb_host`, `libfastdeploy_host` | Android C++ utilities (`RefBase`, `Vector`, `Compat.h`) |
| **`libz`** | External Third-Party (`zlib1g-dev`) | `adb`, `libfastdeploy_host` | Standard zlib compression library |
| **`libziparchive`** | External (AOSP `system/core/libziparchive`) | `adb`, `libfastdeploy_host` | Android Zip archive extraction and creation library |
| **`libzstd`** | External Third-Party (`libzstd-dev`) | `adb` | Zstandard compression library |
| **`AdbWinApi`** | External (Windows host target) | `libadb_host`, `libfastdeploy_host`, `adb` (Windows) | Windows ADB USB driver API interface (`AdbWinApi.dll`) |
| **`AdbWinUsbApi`** | External (Windows host target) | `adb` (Windows `required`) | Windows WinUSB wrapper library (`AdbWinUsbApi.dll`) |

---

## 4. Compiler Flags, Preprocessor Defines, and Linker Flags

### 4.1 Compiler Flags (`cflags`)
The host `adb` target inherits compiler flags from `adb_defaults`:
- `-Wall`
- `-Wextra`
- `-Werror`
- `-Wexit-time-destructors`
- `-Wno-non-virtual-dtor`
- `-Wno-unused-parameter`
- `-Wno-missing-field-initializers`
- `-Wthread-safety`
- `-Wvla`

**Platform-Specific Flags:**
- **Darwin (macOS):**
  - `-D__APPLE_USE_RFC_3542` (enables IPv6 socket options)
  - `-Wno-sizeof-pointer-memaccess`
- **Windows:**
  - `-DUNICODE=1`
  - `-D_UNICODE=1`
  - `-D_GNU_SOURCE`
  - `-D_POSIX_SOURCE`
  - `-Wno-ignored-attributes`
  - `-Wno-thread-safety`

### 4.2 Preprocessor Defines
- `-DADB_HOST=1` (set in `adb_defaults` to distinguish host client/server build from daemon `adbd`)
- `-DANDROID_BASE_UNIQUE_FD_DISABLE_IMPLICIT_CONVERSION=1` (disables implicit conversion in `android::base::unique_fd`)

### 4.3 C++ Standard and STL
- **C++ Standard (`cpp_std`):** `experimental` (corresponds to C++20 / `-std=gnu++20` in modern LLVM/Clang)
- **C++ Standard Library (`stl`):** `libc++_static`

### 4.4 Linker Flags (`ldflags` & `host_ldlibs`)
- **Shared Libraries (`shared_libs`):** Explicitly empty (`shared_libs: []`) in `adb_binary_host_defaults` on Linux to avoid host dynamic link runtime dependencies and C++ ODR violations.
- **System Linker Libraries (`host_ldlibs`):**
  - **Linux Host:**
    - `-lresolv` (provides `b64_pton` for base64 decoding)
    - `-lutil` (provides `forkpty` for PTY handling)
    - `-lpthread` (POSIX threading)
  - **Darwin Host:**
    - `-lpthread`
    - `-framework CoreFoundation`
    - `-framework IOKit`
    - `-framework Security`
    - `-lobjc`
  - **Windows Host:**
    - `-municode` (linker flag for wide-character `wmain` entry point)
    - `-lws2_32`
    - `-lgdi32`
    - `-luserenv`
    - `-liphlpapi`

---

## 5. Undetermined Items

1. **AOSP Internal Header Modifications:** Some external AOSP libraries (`libbase`, `libcutils`, `libutils`) reference internal Android system headers (`cutils/sockets.h`, `log/log.h`, `android/binder_ibinder.h`). When compiling outside Soong, exact include path ordering must be resolved in Issue 10a-3.
2. **`platform_tools_version` Macro Source:** In the Soong build environment, `platform_tools_version` reads `BUILD_NUMBER` and version files from the AOSP environment. In a standalone build, version strings must be passed via compiler flags (e.g. `-DADB_VERSION="36.0.2-14143358"`).

---

## 6. How to Regenerate This Inventory

To regenerate or verify this inventory from the codebase:
1. Inspect `original/Android.bp` and locate module `cc_binary_host` named `"adb"`.
2. Trace `defaults` fields recursively:
   - `adb` -> `adb_binary_host_defaults` -> `adb_defaults`
3. Inspect sub-module `Android.bp` files across `original/`:
   - `original/crypto/Android.bp` (`libadb_crypto`)
   - `original/proto/Android.bp` (`libadb_host_protos`, `libadb_protos`, `libapp_processes_protos_full`)
   - `original/pairing_auth/Android.bp` (`libadb_pairing_auth`)
   - `original/pairing_connection/Android.bp` (`libadb_pairing_connection`)
   - `original/tls/Android.bp` (`libadb_tls_connection`)
   - `original/fastdeploy/Android.bp` (`libfastdeploy_host`)
4. Aggregate `srcs`, `static_libs`, `shared_libs`, `cflags`, `ldflags`, and `host_ldlibs` from all reachable modules.
