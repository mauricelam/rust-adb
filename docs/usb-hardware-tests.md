# Self-Hosted Runner Setup for USB Hardware Tests

This document describes how to configure self-hosted GitHub Actions runners for physical USB hardware testing across Linux, macOS, and Windows.

Hardware tests are defined in `crates/adb-transport/tests/usb_hw_test.rs` and executed via `.github/workflows/usb-hardware.yml`.

---

## 1. Runner Labels

Each runner machine must register with GitHub Actions using the following required labels:
- **Linux**: `[self-hosted, usb-linux]`
- **macOS**: `[self-hosted, usb-macos]`
- **Windows**: `[self-hosted, usb-windows]`

---

## 2. Linux Runner Configuration (`usb-linux`)

### Prerequisites & OS Packages
- Supported distribution: Ubuntu 22.04 LTS / Debian 12 / Arch Linux / Fedora.
- Packages:
  ```bash
  sudo apt-get update && sudo apt-get install -y build-essential curl git libusb-1.0-0-dev udev usbutils
  ```
- Rust toolchain: 1.94.0 (installed via `rustup`).

### Udev Configuration
1. Install `/etc/udev/rules.d/51-android.rules` as described in [`docs/usb-linux.md`](usb-linux.md).
2. Add the runner system user to the `plugdev` group:
   ```bash
   sudo usermod -aG plugdev "$USER"
   sudo udevadm control --reload-rules
   sudo udevadm trigger
   ```

### Device Preparation
- Connect an Android device with USB debugging enabled.
- Verify using `lsusb` that the device is enumerated with Class `0xff`, Subclass `0x42`, Protocol `0x01`.
- Verify the device permissions: `ls -la /dev/bus/usb/<bus>/<dev>` should show group `plugdev` with `rw` permissions.

---

## 3. macOS Runner Configuration (`usb-macos`)

### Prerequisites
- macOS 12 (Monterey) or later on Intel or Apple Silicon.
- Command Line Tools: `xcode-select --install`.
- Rust toolchain: 1.94.0 (installed via `rustup`).

### System Permissions & Driver Setup
- No additional kernel extensions are needed for standard user-mode USB communication via IOKit.
- If running under macOS privacy controls (TCC), ensure terminal/runner daemon has permission to access external accessories.

### Device Preparation
- Connect an Android device with USB debugging enabled.
- Confirm device presence with `system_profiler SPUSBDataType`.

---

## 4. Windows Runner Configuration (`usb-windows`)

### Prerequisites
- Windows 10/11 or Windows Server 2022.
- Build tools: Visual Studio 2022 C++ Build Tools or Build Tools for Visual Studio.
- Rust toolchain: 1.94.0 (x86_64-pc-windows-msvc).

### Driver Setup (WinUSB)
- Follow the instructions in [`docs/usb-windows.md`](usb-windows.md).
- Ensure the Android ADB interface is bound to the `WinUSB` driver (`winusb.sys`).
- Device Manager should show the device under **Universal Serial Bus devices** or **Android Device** with driver provider Microsoft / WinUSB.

### Device Preparation
- Connect an Android device with USB debugging enabled.
- Verify with PowerShell:
  ```powershell
  Get-PnpDevice -PresentOnly | Where-Object { $_.Class -match "Android" -or $_.Service -match "WinUSB" }
  ```

---

## 5. Running Tests Locally on a Prepared Machine

To execute the hardware validation tests on any configured runner host:
```bash
cargo test -p adb-transport --features usb --test usb_hw_test -- --ignored --nocapture
```
