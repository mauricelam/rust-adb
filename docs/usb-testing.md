# USB Test Strategy and Virtual Gadget Spike

This document defines the testing strategy for the USB transport implementation (Phase 6), records test tier requirements for each Phase 6 task, outlines the procedure for a virtual USB gadget spike on Linux, lists candidate C++ source mutants for USB validation, and specifies substitutes where mutation testing cannot be applied.

---

## 1. The Three Test Tiers

Testing USB host transport implementations in a cross-platform environment presents unique challenges. Real USB controllers and devices cannot be assumed in standard headless CI environments, and virtualized devices require kernel-level emulation. To balance test coverage, reproducibility, and automation, USB testing is split into three tiers:

### Tier 1: Host Mock / Transport Trait Unit Tests
- **Environment**: Standard CI (Linux, macOS, Windows) without special permissions or kernel modules.
- **Mechanism**: Pure user-space unit tests. The transport layer abstracts device enumeration, descriptor parsing, and bulk I/O behind traits or mockable backend interfaces (e.g. mocking USB device descriptors, device handles, or endpoint streams).
- **Coverage**:
  - USB interface descriptor matching logic (`original/client/transport_usb.cpp` in `is_adb_interface` and `original/adb.h`): matches interface protocol `ADB_PROTOCOL` (`0x01`) and either standard ADB class/subclass (`ADB_CLASS` `0xff`, `ADB_SUBCLASS` `0x42`) or Debug Class (DbC) class/subclass pair (`ADB_DBC_CLASS` `0xdc`, `ADB_DBC_SUBCLASS` `0x02`).
  - Serial number parsing and string descriptor decoding (`original/client/usb_libusb_device.cpp` in `LibUsbDevice::RetrieveSerial`).
  - ADB packet framing over USB bulk streams: 24-byte `apacket` header transfer followed by separate payload transfer (`original/client/usb_libusb_device.cpp` in `LibUsbDevice::Write`).
  - Zero-Length Packet (ZLP) calculation logic (`original/client/usb_libusb_device.cpp` in `LibUsbDevice::Write`): evaluates `(data_size & zlp_mask_) == 0` strictly when `data_size > 0`; if `data_size == 0`, `LibUsbDevice::Write` returns early after the 24-byte header, so a ZLP is never sent after a header-only packet.
  - Maximum payload clamping (e.g. 1 MiB vs legacy 4 KiB limits).
  - Error translation from backend error codes to `TransportError`.
- **Limitations**: Does not invoke the real OS USB subsystem (usbfs, WinUSB, or IOKit) or test genuine USB transfer semantics, hardware race conditions, or host controller driver behaviors.

### Tier 2: Virtual USB Gadget (Linux Kernel Emulation via `dummy_hcd` and FunctionFS / USB/IP)
- **Environment**: Privileged self-hosted Linux runners (or VMs with root and kernel module loading capabilities).
- **Mechanism**: Linux kernel USB gadget subsystem using `dummy_hcd` (dummy Host Controller Driver and Device Controller Driver) and FunctionFS (`f_fs`), or `usbip` (USB over IP). A user-space bridge connects `mock-adbd` to the FunctionFS endpoints (`ep0` for descriptors, `ep1` for bulk OUT, `ep2` for bulk IN). The host Linux USB subsystem sees a genuine USB device plugged into a virtual root hub.
- **Coverage**:
  - Full OS USB subsystem integration via `/dev/bus/usb/` (usbfs) and `libusb`.
  - True bulk USB transfers, multi-packet reads/writes, and kernel-level ZLP handling.
  - End-to-end ADB wire handshake (`A_CNXN`), authentication, sync, and shell sessions over USB.
  - Simulated disconnect and reconnect by unbinding and rebinding the gadget to the UDC (USB Device Controller).
  - Automated CI execution of integration tests and mutation kill validation on Linux.
- **Limitations**: Linux-only (cannot run on standard macOS or Windows runners); requires root privileges and kernel modules (`dummy_hcd`, `libcomposite`, `usb_f_fs`).

### Tier 3: Physical Hardware Testing
- **Environment**: Physical host machines (Linux, macOS, Windows) connected via physical USB cables to real Android devices or hardware gadget boards (e.g. Raspberry Pi running Linux gadget stack).
- **Mechanism**: End-to-end execution of the test suite and CLI against real USB hardware and devices.
- **Coverage**:
  - Platform-specific native USB driver stacks: Linux `usbfs`, macOS `IOKit` / `IOUSBHost`, Windows `WinUSB` (`winusb.sys`).
  - Hardware electrical characteristics, physical hotplugging, cable bounce, transfer stalls, and bus resets.
  - Host OS power management events (suspend/resume/sleep cycles), where host controllers power down and invalidate active endpoint handles.
- **Limitations**: Cannot be automated in standard pull-request CI without a dedicated, maintained physical device lab. Difficult to inject deterministic hardware errors.

---

## 2. Phase 6 Task Tier Mapping

Every task in Phase 6 has specific verification goals. The table below states the highest tier required to satisfy each task's acceptance criteria, alongside what each task can claim at lower tiers:

| Task ID | Task Description | Highest Required Tier | Tier 1 Claim | Tier 2 Claim | Tier 3 Claim |
|---|---|---|---|---|---|
| **6.1** | Choose `nusb` or `rusb`; enumerate devices, detect ADB interface, read serials | **Tier 2** (Linux CI) / **Tier 3** (Cross-OS verification) | Unit tests verify descriptor filtering (`original/client/transport_usb.cpp` in `is_adb_interface`: class `0xff`/subclass `0x42`/protocol `0x01` or DbC pair `0xdc`/`0x02`/`0x01`), string descriptor decode, and serial matching against recorded mock descriptor bytes. | Host ADB server discovers and lists the virtual USB gadget in `adb devices` over real Linux usbfs. | Verifies native enumeration on physical devices across Linux, macOS, and Windows. |
| **6.2** | Bulk transfer loops, zero-length packets, max payload; shell and push work over USB | **Tier 2** (Linux CI) | Unit tests verify packet splitting, ZLP conditions (sent only after non-empty payload when length is a multiple of max packet size, never after header-only packets, per `original/client/usb_libusb_device.cpp` in `LibUsbDevice::Write`), and buffer framing on mocked streams. | Full `adb shell` and `adb push`/`pull` compatibility tests pass against `mock-adbd` over virtual USB on Linux. | Verified against physical devices on Linux, macOS, and Windows with maximum throughput and no stalls. |
| **6.3** | Hotplug and reconnect handling; mid-transfer recovery | **Tier 2** (Linux CI) / **Tier 3** (Physical disconnect) | Unit tests simulate device removal errors (`ENODEV`, device disconnected) and transport cleanup. | Simulated disconnect/reconnect via UDC unbind/bind during active transfer verifies auto-reconnect and cleanup on Linux. | Verifies physical cable unplug/replug during transfer recovers cleanly without daemon hang or crash. |
| **6.4** | macOS and Windows support, plus Linux udev rules and driver notes | **Tier 3** (Cross-OS Physical / Runner) | Unit tests verify platform-specific path formatting and backend trait implementations. | Not applicable (Tier 2 `dummy_hcd` is Linux-specific). | Acceptance requires test run on macOS (IOKit) and Windows (WinUSB) physical runners. |
| **6.5** | USB stress and error-injection tests; soak without leaks or hangs | **Tier 2** (Automated soak) / **Tier 3** (Hardware soak) | Unit tests inject simulated short reads, timeout errors, and corrupted headers. | 1000-iteration shell/sync loop runs over virtual USB in Linux CI with heap and file descriptor leak checks. | Long soak transfer (e.g. multi-gigabyte sync push/pull) on real hardware runs cleanly. |

---

## 3. Virtual Gadget Spike: Procedure for Linux Runner

This procedure describes how to configure a virtual USB gadget on a privileged Linux runner using `dummy_hcd` and FunctionFS (`f_fs`), allowing C++ and Rust ADB servers to discover and communicate with `mock-adbd` as a real USB device.

> **Status Notice:** Every step in this procedure is currently marked as **`[UNVERIFIED]`** until validated on a dedicated self-hosted Linux runner with kernel module permissions.

### Prerequisites `[UNVERIFIED]`
- Linux kernel 5.4+ with modules: `dummy_hcd`, `libcomposite`, `usb_f_fs`.
- Root (`sudo`) privileges on the runner.
- Mounted `/sys/kernel/config` (ConfigFS).

### Step 1: Load Kernel Modules `[UNVERIFIED]`
Load the dummy host controller driver and composite gadget framework:
```bash
sudo modprobe dummy_hcd
sudo modprobe libcomposite
```
*Verification:* Confirm the virtual UDC and virtual host controller appear:
```bash
ls /sys/class/udc/
# Expected output: dummy_udc.0
ls /sys/bus/usb/devices/
# Expected output includes a dummy root hub (e.g., usbX)
```

### Step 2: Create USB Gadget via ConfigFS `[UNVERIFIED]`
Set up the USB gadget descriptors matching standard Android ADB device specifications:
```bash
sudo mkdir -p /sys/kernel/config/usb_gadget/g1
cd /sys/kernel/config/usb_gadget/g1

# USB Vendor & Product IDs (Google ADB)
echo 0x18d1 > idVendor
echo 0x4ee7 > idProduct
echo 0x0200 > bcdUSB
echo 0x0100 > bcdDevice

# Device strings (English)
mkdir -p strings/0x409
echo "1234567890ABCDEF" > strings/0x409/serialnumber
echo "Google" > strings/0x409/manufacturer
echo "Virtual ADB Device" > strings/0x409/product

# Configuration 1
mkdir -p configs/c.1/strings/0x409
echo "ADB Config" > configs/c.1/strings/0x409/configuration
echo 500 > configs/c.1/MaxPower

# ADB FunctionFS Function
mkdir -p functions/ffs.adb
ln -s functions/ffs.adb configs/c.1/
```

### Step 3: Mount FunctionFS `[UNVERIFIED]`
Prepare the mount point and mount the FunctionFS filesystem:
```bash
sudo mkdir -p /dev/usb-ffs/adb
sudo mount -t functionfs adb /dev/usb-ffs/adb
```

### Step 4: Launch `mock-adbd` FunctionFS Bridge `[UNVERIFIED]`
FunctionFS requires a user-space daemon to write descriptors into `/dev/usb-ffs/adb/ep0` and handle bulk read/writes on `ep1` and `ep2`:
1. Open `/dev/usb-ffs/adb/ep0`.
2. Write descriptors specifying Interface Class `0xFF` (Vendor Specific), Subclass `0x42`, Protocol `0x01` (ADB), with one Bulk IN and one Bulk OUT endpoint (512 bytes max packet for High Speed, 1024 for SuperSpeed).
3. Read strings/events from `ep0`.
4. Forward incoming packets from the Bulk OUT endpoint to `mock-adbd`'s packet processor, and write responses to the Bulk IN endpoint.

*(Note: Building this user-space bridge is out of scope for task 6.0 and will be implemented in a follow-up task if the spike is approved.)*

### Step 5: Bind the Gadget to the Virtual UDC `[UNVERIFIED]`
Enable the USB gadget by binding it to the dummy UDC:
```bash
UDC_NAME=$(ls /sys/class/udc | head -n 1)
echo "$UDC_NAME" | sudo tee /sys/kernel/config/usb_gadget/g1/UDC
```
*Verification:* Kernel messages (`dmesg`) will show a new high-speed USB device connected to the dummy HCD.

### Step 6: Confirm C++ ADB Server Lists the Device `[UNVERIFIED]`
Start the reference C++ ADB server and verify device discovery:
```bash
adb kill-server
adb start-server
adb devices -l
```
*Expected output:*
```text
List of devices attached
1234567890ABCDEF       device usb:dummy_udc.0 product:Virtual_ADB_Device model:Virtual_ADB_Device device:Virtual_ADB_Device transport_id:1
```
Run a simple command to verify end-to-end communication:
```bash
adb -s 1234567890ABCDEF shell echo "USB gadget OK"
```

### Step 7: Teardown `[UNVERIFIED]`
To cleanly dismantle the virtual gadget:
```bash
echo "" | sudo tee /sys/kernel/config/usb_gadget/g1/UDC
sudo umount /dev/usb-ffs/adb
sudo rm -rf /sys/kernel/config/usb_gadget/g1/configs/c.1/ffs.adb
sudo rm -rf /sys/kernel/config/usb_gadget/g1/configs/c.1/strings/0x409
sudo rm -rf /sys/kernel/config/usb_gadget/g1/configs/c.1
sudo rm -rf /sys/kernel/config/usb_gadget/g1/functions/ffs.adb
sudo rm -rf /sys/kernel/config/usb_gadget/g1/strings/0x409
sudo rm -rf /sys/kernel/config/usb_gadget/g1
sudo rmmod dummy_hcd
```

---

## 4. Candidate C++ USB Mutants

To validate that our tests are sensitive to USB-specific bugs, we identify candidate mutants in the C++ ADB source code (`original/client/`). In standard builds on non-Windows platforms (Linux and macOS), `is_libusb_enabled()` in `original/client/transport_usb.cpp` defaults to `true` (unless overridden via the `ADB_LIBUSB` environment variable), which causes ADB to use the `libusb` backend (`original/client/usb_libusb.cpp` and `original/client/usb_libusb_device.cpp`) rather than the legacy `usb_linux.cpp` backend. Consequently, candidate mutants for default test runs must target the active `libusb` backend implementation and shared transport functions.

The table below outlines each mutant, the backend it targets, its location, the failure mode it introduces, and the minimum test tier required to kill it:

| Mutant Description | Backend | Target File & Function | Introduced Breakage | Minimum Tier Needed | Rationale |
|---|---|---|---|---|---|
| **Corrupted Interface Filter** | `libusb` | `original/client/transport_usb.cpp` in `is_adb_interface` (called by `LibUsbDevice::FindAdbInterface` in `original/client/usb_libusb_device.cpp`) | Mutate interface match: alter `ADB_SUBCLASS` (`0x42` to `0x43`), `ADB_CLASS` (`0xff` to `0xfe`), or DbC match (`ADB_DBC_SUBCLASS` `0x02` to `0x03`). | **Tier 2** (Virtual Gadget) or **Tier 3** (Hardware) | Without a real or virtual USB device enumerating on the bus, descriptor parsing in `FindAdbInterface` is never reached. |
| **Wrong Zero-Length Packet (ZLP) Condition** | `libusb` | `original/client/usb_libusb_device.cpp` in `LibUsbDevice::Write` | Mutate ZLP condition `if ((data_size & zlp_mask_) == 0)` to `!= 0`, or remove the early return `if (data_size == 0) return true;` so that ZLPs are erroneously sent after 0-byte payload (header-only) packets. | **Tier 2** (Virtual Gadget) or **Tier 3** (Hardware) | Appears when transfer size is a non-zero multiple of endpoint max packet size (e.g. 512 or 1024 bytes). Omission causes the receiver to hang waiting for termination; sending ZLP after header-only packets corrupts stream synchronization. |
| **Header and Payload Sent as One Transfer** | `libusb` | `original/client/usb_libusb_device.cpp` in `LibUsbDevice::Write` | Combine the 24-byte `apacket` header and payload into a single `libusb_bulk_transfer` instead of two separate transfers. | **Tier 2** (Virtual Gadget) or **Tier 3** (Hardware) | USB hardware endpoints and packet readers expect the 24-byte header in its own transfer frame. Combining them breaks receivers expecting distinct endpoint packets. |
| **Omitted Clear Halt on Claim** | `libusb` | `original/client/usb_libusb_device.cpp` in `LibUsbDevice::ClaimInterface` | Remove the endpoint stall clearing loop `for (uint8_t endpoint : {read_endpoint_, write_endpoint_}) libusb_clear_halt(...)` after `libusb_claim_interface`. | **Tier 2** (Virtual Gadget) or **Tier 3** (Hardware) | If an endpoint was left in a stalled or halted state from a prior interrupted session or error, subsequent bulk reads or writes will immediately fail with stall (`LIBUSB_ERROR_PIPE`). |
| **Wrong Endpoint Selection** | `libusb` | `original/client/usb_libusb_device.cpp` in `LibUsbDevice::FindAdbInterface` | Swap assignments: set `write_endpoint_ = bulk_in; read_endpoint_ = bulk_out;`. | **Tier 2** (Virtual Gadget) or **Tier 3** (Hardware) | Host attempts to read from an OUT endpoint and write to an IN endpoint, causing immediate pipe errors (`LIBUSB_ERROR_PIPE`). |
| **Disabled Hotplug Disconnect Notification** | `libusb` | `original/client/usb_libusb.cpp` in `LibUsbConnection::OnError` | Remove `transport_->HandleError(reason);` or `libusb_inhouse_hotplug::report_error(*this);`. | **Tier 2** (Virtual Gadget unbind) or **Tier 3** (Unplug) | When a device disconnects or suffers a bus error mid-transfer, failing to notify transport keeps the connection handle alive and leaks zombie transport state. |

---

## 5. Mutation Kill Rate Limitations and Substitutes

### What Cannot Be Proven via Mutation Kill Rate
1. **Host USB Code in CI Without USB Hardware**:
   - The mutation testing pipeline established in Phase 0 (tasks 0.10 and 0.11) runs automated CI tests against mutated C++ binaries. In standard GitHub Actions runners, there is no USB controller or device. Any mutant applied to `client/usb_*.cpp` (such as the ZLP rule or interface filter) will **survive** because the code paths are never reached.
2. **Platform-Specific Host Stacks (macOS and Windows)**:
   - Even if Tier 2 (Virtual Gadget via `dummy_hcd`) is deployed in Linux CI, it cannot exercise `client/usb_osx.cpp` (IOKit) or `client/usb_windows.cpp` (WinUSB).
   - Therefore, a global 90%+ mutation kill rate across the entire C++ codebase cannot realistically include `client/usb_*.cpp` unless executed on a specialized test matrix equipped with USB emulation on all three OSes (which does not exist for macOS/Windows in standard runners).

### Agreed Substitutes for Phase 6 USB Validation
Where automated mutation testing cannot achieve kill coverage, the following verification substitutes are mandatory:

1. **Human-Reviewed Recorded C++ Runs on Real Hardware**:
   - For each supported operating system (Linux, macOS, Windows), maintainers execute a standardized verification script against reference C++ ADB with a physical Android device connected.
   - The test script captures:
     - Full USB enumeration trace (`ADB_TRACE=usb,transports`).
     - Handshake, small shell commands, and bulk file transfers (`push` and `pull` with file sizes chosen to hit boundary conditions: 0 bytes, 511 bytes, 512 bytes, 1024 bytes, 64 KiB, and 10 MiB to verify ZLP handling).
     - Physical disconnect during an active transfer.
   - The recorded trace is inspected and archived as a golden reference.
2. **Differential Execution on Hardware**:
   - The exact same verification script is executed using the Rust `adb-cli` and server against the same physical device on each OS.
   - The resulting logs and wire captures are compared against the C++ golden run.
3. **Tier 1 Unit Coverage for Replicated Logic**:
   - All pure logic extracted from C++ USB code (ZLP conditions, packet splitting math, descriptor parsing) must be placed in pure Rust modules covered by unit tests that achieve 100% branch coverage.
4. **Linux-Only Tier 2 Virtual Gadget Integration**:
   - On Linux runners where `dummy_hcd` is available, automated integration tests run regularly to ensure no regressions in USB connection handling.
