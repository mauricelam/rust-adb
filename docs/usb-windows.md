# Windows USB Driver Requirements and WinUSB Setup

> **Verification Notice**: The Windows behaviors documented here are based on upstream C++ `original/client/usb_windows.cpp` (AdbWinApi / WinUSB) and `nusb`'s Windows backend (`windows_winusb`). Portions of this guide marked with `[NEEDS HUMAN VERIFICATION]` require verification on a physical Windows machine with an attached Android device.

---

## 1. Background: Windows USB Driver Model for ADB

On Windows, user-mode applications cannot directly talk to arbitrary USB devices unless a supported driver is installed. For ADB:
1. Upstream C++ ADB uses the proprietary `AdbWinApi.dll` / `AdbWinUsbApi.dll` wrappers around Microsoft's WinUSB driver (`winusb.sys`).
2. The Rust backend (`nusb`) communicates directly with Microsoft's `WinUSB` driver using Windows SetupAPI / CfgMgr32 device interfaces.
3. For an Android device's ADB interface to be recognized and opened:
   - The device interface must be bound to `WinUSB` (`winusb.sys`).
   - The device interface must expose the standard Android ADB interface class GUID:
     ```text
     {F72FE0D4-CBCB-407D-8814-9ED673D0DD6B}
     ```
     (defined upstream as `ANDROID_USB_CLASS_ID`).

---

## 2. What a User Sees When Drivers Are Missing `[NEEDS HUMAN VERIFICATION]`

If a physical Android device is connected to Windows without a WinUSB driver:
1. In Windows Device Manager, the device appears under **Other devices** with a yellow exclamation mark (labeled e.g. `Android` or `ADB Interface`), or under a vendor-specific class without WinUSB loaded.
2. In `adb devices`:
   - The device is **not listed** at all (`List of devices attached` followed by a blank list).
   - Unlike Linux, Windows does not enumerate the device if WinUSB is not bound, so no "no permissions" state is reported; the device is completely absent from user-mode device enumeration.
3. Windows SetupAPI returns `ERROR_NO_MORE_ITEMS` during enumeration.

---

## 3. Installing the Driver `[NEEDS HUMAN VERIFICATION]`

Users must install a WinUSB-compatible driver for the Android ADB interface:

### Method A: Google USB Driver (Official)
1. Download the Google USB Driver from [developer.android.com/studio/run/win-usb](https://developer.android.com/studio/run/win-usb).
2. Open Device Manager (`devmgmt.msc`).
3. Right-click the Android device -> **Update driver** -> **Browse my computer for drivers**.
4. Select the unzipped `google-usb-driver` folder containing `android_winusb.inf`.

### Method B: Zadig (Universal WinUSB Installer)
1. Download Zadig from [zadig.akeo.ie](https://zadig.akeo.ie).
2. In Zadig, select **Options** -> **List All Devices**.
3. Select the Android ADB interface (e.g., `Composite ADB Interface` or `Android (Interface 0)`).
4. Choose **WinUSB** as the target driver and click **Install Driver** or **Replace Driver**.

---

## 4. Verification

After driver installation:
1. Run `cargo test -p adb-transport --features usb --test usb_hw_test -- --ignored --nocapture`.
2. `test_windows_winusb_device_enumeration` will list the device and confirm that the bulk IN/OUT endpoints are accessible.
