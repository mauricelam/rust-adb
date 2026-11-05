# macOS USB Support in ADB

> **Verification Notice**: The behaviors documented here are analyzed directly from upstream C++ `original/client/usb_osx.cpp` and `original/client/usb_libusb.cpp`, and mapped to the Rust `nusb` backend in `crates/adb-transport`. Portions of this guide marked with `[NEEDS HUMAN VERIFICATION]` require verification on real macOS hardware with connected physical devices.

---

## 1. macOS-Specific Behaviors Handled in Upstream C++ ADB

The upstream macOS C++ implementation (`usb_osx.cpp`) handles several Darwin/IOKit-specific platform quirks:

1. **IOKit Interface and Device Matching**:
   - Matches USB devices matching the `IOUSBInterface` service dictionary with class `0xff`, subclass `0x42`, protocol `0x01`.
   - Traverses parent `IOUSBDevice` from `IOUSBInterface` to query Vendor ID, Product ID, and Location ID.
   - Handles location IDs: on macOS, USB devpaths are formatted as `usb:<locationId>X` (hexadecimal string representation of the 32-bit IOKit location ID).
2. **Interface Claiming and Exclusive Access**:
   - Upstream opens interfaces with `USBInterfaceOpen()`. On macOS, if another driver or user client has claimed the interface, exclusive access must be established.
   - `nusb` maps interface claiming on macOS to `IOUSBInterface::USBInterfaceOpen()`.
3. **Endpoint Stall Clearing (`ClearPipeStallBothEnds`)**:
   - `usb_osx.cpp` implements `ClearPipeStallBothEnds`: before starting communications, it can clear the device-side endpoint halt condition if enabled by `ADB_OSX_USB_CLEAR_ENDPOINTS`.
   - When bulk read encounters `kIOUSBPipeStalled`, it calls `ClearPipeStall()` on the interface pipe to reset the data toggle and unfreeze the pipe.
4. **Sleep / Wake and Re-enumeration**:
   - When macOS sleeps and wakes, USB host controllers can reset connections.
   - Upstream runs an explicit `RunLoopThread` polling every 1 second, unregistering dead handles and kicking devices.
   - `nusb` on macOS listens to IOKit notifications for device arrival and removal asynchronously.

---

## 2. Implementation in Rust (`adb-transport`)

In `adb-transport`, macOS support is provided via the `nusb` backend:
- `NusbBackend::list_devices`: queries `nusb::list_devices()` which enumerates through IOKit on macOS.
- `NusbBackend::open`: opens device handles and claims the ADB interface (`claim_interface`).
- `NusbDeviceHandle::clear_halt`: clears stalled bulk endpoints.
- Zero-length packet (ZLP) handling: `UsbTransport` sends 0-byte packets when transfers are exact multiples of the endpoint's `wMaxPacketSize`.
