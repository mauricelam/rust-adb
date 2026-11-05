# Linux USB Permissions and Udev Rules

> **Verification Notice**: The udev rules, group configurations, and diagnostics documented here match upstream Android C++ ADB (`client/usb_linux.cpp` and `diagnose_usb.cpp`). Portions of this guide marked with `[NEEDS HUMAN VERIFICATION]` require verification on real physical Linux distributions (Ubuntu, Debian, Fedora, Arch) with attached Android hardware.

---

## 1. Background: "no permissions" State in ADB

On Linux, USB device nodes located under `/dev/bus/usb/` are root-owned by default (e.g. `root:root` with permissions `0660` or `0644`). When an ordinary user runs `adb devices`, the ADB server scans `/dev/bus/usb/` and tries to open the device for read/write access.

If the user does not have write permissions to the USB device node:
1. Opening the device fails with `EACCES` (`Permission denied`).
2. The ADB server falls back to opening the device read-only or detecting its interface descriptors without root access.
3. The server registers the device with the connection state `ConnectionState::NoPerm`.
4. In `adb devices`, the output matches:
   ```text
   List of devices attached
   <device-path>	no permissions (user <username> is not in the plugdev group); see [http://developer.android.com/tools/device.html]
   ```
   Or if the user is already in the `plugdev` group:
   ```text
   List of devices attached
   <device-path>	no permissions (missing udev rules? user is in the plugdev group); see [http://developer.android.com/tools/device.html]
   ```
5. Attempting to acquire the transport (such as running `adb shell`, `adb push`, or `adb logcat`) fails immediately with:
   ```text
   insufficient permissions for device: <reason>
   See [http://developer.android.com/tools/device.html] for more information
   ```

---

## 2. Udev Rule Setup `[NEEDS HUMAN VERIFICATION]`

To give unprivileged users access to Android devices over USB, install a udev rule file that assigns Android USB devices to the `plugdev` group and grants `0660` or `0666` permissions.

### Example: `/etc/udev/rules.d/51-android.rules`

Create `/etc/udev/rules.d/51-android.rules` with the following content:

```udev
# Google / Android Open Accessory / ADB Vendor IDs
SUBSYSTEM=="usb", ATTR{idVendor}=="18d1", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# Samsung
SUBSYSTEM=="usb", ATTR{idVendor}=="04e8", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# HTC
SUBSYSTEM=="usb", ATTR{idVendor}=="0bb4", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# LG
SUBSYSTEM=="usb", ATTR{idVendor}=="1004", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# Motorola
SUBSYSTEM=="usb", ATTR{idVendor}=="22b8", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# Sony
SUBSYSTEM=="usb", ATTR{idVendor}=="0fce", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# Huawei
SUBSYSTEM=="usb", ATTR{idVendor}=="12d1", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# Xiaomi
SUBSYSTEM=="usb", ATTR{idVendor}=="2717", MODE="0660", GROUP="plugdev", TAG+="uaccess"
# OnePlus
SUBSYSTEM=="usb", ATTR{idVendor}=="2a70", MODE="0660", GROUP="plugdev", TAG+="uaccess"

# Catch-all ADB interface match (ADB class=0xff, subclass=0x42, protocol=0x01)
SUBSYSTEM=="usb", ENV{DEVTYPE}=="usb_device", ATTR{bInterfaceClass}=="ff", ATTR{bInterfaceSubClass}=="42", ATTR{bInterfaceProtocol}=="01", MODE="0660", GROUP="plugdev", TAG+="uaccess"
```

> **Note on systemd/elogind distros**: Many modern distributions (Fedora, Arch, Debian 12+, Ubuntu 22.04+) use `uaccess` tags with systemd-logind to grant ACL permissions automatically to the active desktop session user without requiring group membership. However, upstream ADB diagnostics specifically look for `plugdev` membership as a fallback diagnostic.

---

## 3. Group Configuration `[NEEDS HUMAN VERIFICATION]`

Ensure the `plugdev` group exists and that your user belongs to it:

```bash
# 1. Create the plugdev group if it does not exist
sudo groupadd -r plugdev 2>/dev/null || true

# 2. Add your current user to the plugdev group
sudo usermod -aG plugdev "$USER"

# 3. Reload udev rules and trigger device updates
sudo udevadm control --reload-rules
sudo udevadm trigger

# 4. Activate group in the current session (or log out and log back in)
newgrp plugdev
```

---

## 4. Restarting the ADB Server

After updating udev rules and group membership, kill any running server so it re-enumerates devices:

```bash
adb kill-server
adb devices
```

The device should now transition from `no permissions` to `device` (or `unauthorized` if host keys must be authorized on screen).
