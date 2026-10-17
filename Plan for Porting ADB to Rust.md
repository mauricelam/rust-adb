# Plan for Porting ADB to Rust

A staged approach to rewriting the Android Debug Bridge in Rust, where the compatibility tests are written first, proven against the real C++ ADB, and then checked by mutating the C++ source, before any Rust is written. This plan is based on general knowledge of the ADB codebase (packages/modules/adb in AOSP), so verify details against the live source as you go.

## 0. Decide scope first

ADB is three programs, plus shared libraries, tools and tests, in one codebase: the **client** (CLI), the **server** (host daemon on port 5037), and **adbd** (the on-device daemon). They differ a lot in difficulty:

- **Client and server** are mostly portable I/O and protocol logic. Start here.
- **adbd** is tightly coupled to Android internals (properties, SELinux, the framework auth service, the adbd APEX, Soong builds). Port it last, once the host side is stable, and build it with Cargo (cross-compiled for Android targets) rather than Soong.

Also decide up front:

- **Build system:** Cargo for every crate, including adbd; Soong is not used.
- **Target OSes:** every OS that ADB supports, which means Linux, macOS and Windows hosts plus any others in the live build configuration; Windows and macOS USB support are the painful ones.
- **Goal:** behavioral parity (scripts depend on exact output text and exit codes) or a cleaner redesign.

## 1. Read the specs before the code

Seven documents define the behavior the port has to match; read them in this order. The links pin the `android15-tests-dev` branch, so switch them to the branch you port from, and browse the full tree on [Code Search](https://cs.android.com/android/platform/superproject/+/android-latest-release:packages/modules/adb/).

1. [ADB Internals (README.md)](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/README.md): the map of the codebase, including the three components, smart socket, transports, the fdevent loop and the asocket and apacket model.
2. [OVERVIEW.TXT](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/OVERVIEW.TXT): the three-component overview that the README tells new readers to start with. I have not read it directly.
3. [protocol.txt](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/protocol.txt): the wire protocol between server and adbd.
4. [SERVICES.TXT](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/SERVICES.TXT): every smart-socket request a client can make, host and local.
5. [SYNC.TXT](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/SYNC.TXT): the file-transfer sub-protocol behind push and pull.
6. [Architecture of ADB Wifi](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/docs/dev/adb_wifi.md): pairing, TLS, mDNS discovery and auto-connect.
7. [daemon/jdwp\_service.cpp](https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/android15-tests-dev/daemon/jdwp_service.cpp): the adbd to app JDWP pipeline, which the README says is explained there with diagrams. I have not read it directly.

### What matters for the port

This summarizes the documents above. Where a task in section 7 relies on something they do not state, the item says so.

- **Architecture:** the client and server are one executable running on the host, and adbd runs on the device; host-only code sits behind an `ADB_HOST` guard. The server and adbd are built around a main-thread fdevent loop (an abstraction over poll, epoll and WSAPoll) with a run queue and an interrupt fd, and some services spawn extra threads. This is the model the tokio design in section 2 replaces.
- **Message format:** each message is a 24-byte header of six little-endian 32-bit words (command, arg0, arg1, data length, payload CRC32, and magic equal to the command XOR 0xffffffff), then an optional payload. An invalid header, a corrupt payload or an unknown command must close the connection.
- **Commands:** CNXN, AUTH, OPEN, OKAY, WRTE, CLSE and STLS are live; SYNC is obsolete and never sent on the wire.
- **Handshake:** both sides send CNXN first and ignore everything received before one arrives. The doc gives version 0x01000000 and a max payload of 256 KiB, and notes that older adb hard-coded 4096, so CNXN and AUTH packets sent to a device must stay under that. The identity string is `<systemtype>:<serialno>:<banner>`.
- **Auth:** the device sends an AUTH TOKEN, the host answers with a SIGNATURE, and a failed signature gets a fresh token. After the host has tried all its keys it sends its public key, and the device may ask the user to confirm it.
- **Streams and flow control:** OPEN carries a non-zero local id, and each side has its own stream ids. After one WRTE, no further WRTE may be sent until an OKAY comes back, and violating that closes the connection. The doc mentions possible future windowing; the delayed-ack feature from section 2 is not described in it, so take that from the C++ source.
- **Smart socket:** port 5037, request is a four-hex-digit length plus an ASCII service name, reply is OKAY or FAIL followed by a length and reason.
- **Host services to implement:** version, kill, devices, devices-l, the track-devices variants (text, binary protobuf, text protobuf), emulator, the transport selectors (serial, usb, local, any), the host-serial, host-usb and host-local prefixes, server-status, get-serialno, get-devpath, get-state, and the forward family (forward, norebind, killforward, killforward-all, list-forward).
- **Local services to implement:** shell (v1), shell,v2 (API 24 and up), exec, abb and abb\_exec (API 30 and up), remount, dev and dev-raw, tcp, the local socket variants, framebuffer, jdwp, track-jdwp, track-app, sync, and reverse.
- **Sync:** `sync:` switches the connection to a binary mode of eight-byte packets, a four-letter ASCII id plus a little-endian length. This copy documents LIST (DENT entries, then DONE), SEND (path and mode split at the last comma, DATA chunks up to 64 KiB, DONE carrying the modified time, then OKAY), RECV and STAT. It describes only v1, so the v2 requests and compression in task 1.5 must come from the source.
- **Wireless debugging:** pairing uses a Pairing Server on the device with RSA 2048 in an x509 certificate, bootstrapped by a six-digit pairing code or a QR code. After pairing, adbd listens on a random port and opens with A\_STLS instead of A\_AUTH, so all traffic is TLS. The SPAKE2 detail in task 7.2 is not in this page; confirm it in the pairing code.
- **Discovery:** the device publishes three mDNS service types (`_adb._tcp`, `_adb-tls-pairing._tcp`, `_adb-tls-connect._tcp`) and the host only consumes them. On start the host browses all three and connects to paired devices that publish `_adb-tls-connect`. The CLI surface is `adb mdns check`, `adb mdns services` and `adb pair`.

## 2. Suggested architecture

A Cargo workspace with layered crates:

- `adb-proto`: message types, serialization, shell v2 framing, sync protocol. Pure, no I/O.
- `adb-auth`: RSA keys, Android's custom public-key format, adbkey handling, TLS and pairing.
- `adb-transport`: a `Transport` trait with TCP, USB, and emulator implementations.
- `adb-server`: device registry, stream multiplexing, port forwarding, device tracking.
- `adb-client`: library for talking to a server.
- `adb-cli`: the `adb` binary (clap).

For concurrency, use tokio. The C++ design is an fdevent loop plus per-transport threads, and async maps well onto many multiplexed streams over a single transport. Pay close attention to the flow-control model (per-write OKAY acks), because it is easy to get subtly wrong.

## 3. Milestones, in order

1. **Compatibility suite first (tasks 0.7 to 0.12), then the protocol crate with golden tests.** Capture real traffic from a device and replay it in tests.
2. **Rust client against the existing C++ server.** The smart-socket protocol is stable, so this gives you a working `adb devices` and `adb shell` almost immediately, with no USB or auth work.
3. **Rust server over TCP/emulator** against a real device or emulator, with the existing C++ client in front. Cover auth (AUTH\_TOKEN, SIGNATURE, RSAPUBLICKEY) and `host:track-devices`.
4. **Core services:** shell v1 and v2 (PTY handling, stdin/stdout/stderr multiplexing), `exec`, sync (push, pull, stat, list, v2, compression), forward and reverse.
5. **USB transport.** Evaluate `nusb` (pure Rust) vs. `rusb` (libusb). Expect per-OS quirks: udev permissions on Linux, IOKit on macOS, WinUSB on Windows.
6. **Wireless debugging:** mDNS discovery, SPAKE2 pairing, and TLS. Check early that your TLS stack (rustls) supports keying-material export and custom certificate verification, since the pairing flow depends on both.
7. **Higher-level features:** install (including `abb` and `cmd` paths and incremental install), logcat, jdwp tracking, reboot, root, remount, sideload.
8. **adbd, last, since the goal covers the whole toolchain.**

## 4. Testing strategy

- **Cross-compatibility matrix:** {C++ client, Rust client} x {C++ server, Rust server} x {C++ adbd}. This is the main correctness oracle.
- Port or reuse the existing Python tests (`test_adb.py`, `test_device.py`) against the Rust binary.
- Fuzz the packet parsers with `cargo-fuzz`, since they handle untrusted device input.
- Compare CLI output and exit codes against the C++ client on a corpus of commands.
- Benchmark push/pull and shell throughput early, because ADB performance regressions are very noticeable.

## 5. Known pitfalls

- Version and feature negotiation (max payload size, feature strings) has legacy behavior you will need to preserve.
- Windows needs separate handling for Ctrl-C, console modes, and ConPTY.
- Shell quoting and escaping semantics differ between v1 and v2.
- Server lifecycle (`start-server`, `kill-server`, version-mismatch restarts) is fiddly and heavily relied on by tooling.
- Many tools, such as Android Studio and Gradle, speak directly to port 5037, so the server's smart-socket behavior must stay faithful.

## 6. Prior art

Look at community crates such as `adb_client` and `mozdevice` for ideas and pitfalls. They are partial implementations, but they show how others structured the protocol layer.

## 7. Detailed task breakdown

The port splits into 11 phases of small tasks, each sized for one Jules session and finished only when its parity tests pass. Task IDs are phase.number, and "Depends on" lists the tasks that must be merged first.

### How to run these with Jules

- One task per Jules session and one PR per task. Split any task that touches more than one crate or changes more than about 1,000 lines.
- Write every prompt from one template: goal, the C++ files to read, the Rust crate to change, the exact test commands, and the task's "Done when" line.
- Write the parity tests before the implementation, in a separate PR you review yourself. A test counts only after it has passed against the C++ ADB and failed against a deliberately broken C++ build (tasks 0.9 to 0.11). Recorded traffic and golden outputs are the oracle, so Jules must not change them without review.
- Order: phase 0, then phases 1 and 2 in parallel, then phase 3 (and 4 after it) alongside phase 5, then phases 6 and 7 in parallel, then 8, then 9. Phase 10 (adbd) comes last.

### Phase 0: Foundations

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 0.1 | Record scope decisions (client plus server first, Cargo only, tokio, minimum Rust version, parity target) in docs/decisions.md | none | File merged and linked from AGENTS.md |
| 0.2 | Create the Cargo workspace with the six crates and CI running fmt, clippy and tests on every supported OS (Linux, macOS, Windows and any others the C++ build targets) | 0.1 | CI is green on the empty workspace |
| 0.3 | Pin the upstream ADB revision and build the C++ reference binaries (client, server, and an emulator image with adbd) in CI | 0.2 | CI exposes a reference `adb` binary |
| 0.4 | Write AGENTS.md with build, test and lint commands and the rule that fixtures need human review | 0.2 | Jules can run the full suite from that file alone |
| 0.5 | Build a logging TCP proxy that records smart-socket and wire-protocol traffic into fixture files | 0.3 | Recorded `adb devices` and `adb shell` sessions are checked in |
| 0.6 | Create the parity tracker: every `adb` subcommand and `host:` service with a status column | 0.1 | List is generated from the C++ help output and reviewed |

### Phase 0, part 2: Compatibility suite (written first)

The suite is built and proven before any Rust is written: it must pass with C++ ADB on both sides, then catch deliberate breakage in the C++ source. An implementation task in phases 1 to 10 starts only after the tests for its area have cleared tasks 0.9 and 0.11.

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 0.7 | Make the suite target any implementation: the binaries or endpoints under test come from environment variables, so the same tests run against C++ or Rust | 0.3 | One command runs the suite against a chosen client, server and adbd |
| 0.8 | Write the tests for every row of the parity tracker (CLI golden output and exit codes, smart-socket conformance, recorded wire replay, sync, shell, forward) | 0.5, 0.6, 0.7 | Every tracker row has a test or a recorded reason it has none |
| 0.9 | Baseline run: the whole suite, plus the upstream Python tests, with C++ ADB as both sides | 0.8 | All tests pass on repeated runs; flaky tests are fixed or quarantined with a reason |
| 0.10 | Mutation run: build C++ variants that break compatibility on purpose (change header fields, flow-control rules, output text, exit codes, sync chunking, dropped commands), for example with a mutation tool such as Mull or scripted patches | 0.9 | Every mutant is recorded as caught by at least one test or as a survivor |
| 0.11 | Close the gaps: add or tighten tests until survivors are caught, or document them as equivalent mutants, and repeat the run | 0.10 | Surviving mutants are reviewed and the agreed kill rate is met |
| 0.12 | Freeze and tag the suite; changes need human review, and a scheduled CI job reruns the mutation check | 0.11 | Tagged suite version; the CI job is green |

### Phase 1: adb-proto

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 1.1 | 24-byte packet header encode and decode, command constants, magic check | 0.2 | Round-trip and golden tests pass |
| 1.2 | Smart-socket framing: four-hex-digit length prefix, OKAY and FAIL replies | 0.2 | Output matches recorded fixtures |
| 1.3 | Connection banner and feature-string parsing, version and max-payload constants | 1.1 | Parses banners recorded from several Android versions |
| 1.4 | Shell v2 framing (stdin, stdout, stderr, exit, close-stdin, window-size) | 1.1 | Golden tests against recorded shell sessions |
| 1.5 | Sync protocol v1 and v2 messages (LIST, STAT, SEND, RECV, DONE, DATA, QUIT) and compression flags | 1.1 | Golden tests cover v2 flags |
| 1.6 | Fuzz targets and proptest round trips for every parser above, plus a differential harness against the C++ parser | 1.1 to 1.5 | Fuzz job runs in CI and shows no disagreement with C++ |

### Phase 2: adb-auth

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 2.1 | Load, generate and store adbkey and adbkey.pub with correct permissions; honor ADB\_VENDOR\_KEYS | 0.2 | A key made by Rust loads in C++ adb and the reverse |
| 2.2 | Encode public keys in Android's custom format | 2.1 | Output is byte-identical to C++ for the same key |
| 2.3 | Sign AUTH\_TOKEN challenges the way the C++ code does | 2.1 | An emulator accepts the signature |
| 2.4 | Auth state machine: signature first, then public key, then wait for user approval; handle rejection | 1.1, 2.2, 2.3 | Authorizes against a real device and the approval persists across reconnects |

### Phase 3: adb-client (against the C++ server)

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 3.1 | Connect to port 5037, start the server if absent, check its version | 1.2 | `host:version` works and start-server behavior matches C++ |
| 3.2 | Host services: devices, devices-l, track-devices, features, get-state, get-serialno, kill, connect, disconnect | 3.1 | Replies match the C++ client's for every service |
| 3.3 | Device selection: -s, -d, -e, -t, transport-id, ANDROID\_SERIAL, and the ambiguity errors | 3.2 | Error messages match golden output |
| 3.4 | Shell and exec services (v1 and v2) with client-side raw terminal and window resize | 1.4, 3.3 | Interactive shell works on all three OSes and exit codes propagate |
| 3.5 | Sync client: push, pull, ls, stat, progress, compression, symlinks, modes and timestamps | 1.5, 3.3 | Checksums match on a large file and on a tree of many small files |
| 3.6 | forward and reverse management commands | 3.3 | List, add and remove match C++ |

### Phase 4: adb-cli

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 4.1 | adb-style argument parsing (global options before the subcommand, non-clap quirks), usage and help text | 3.3 | Help output matches C++ |
| 4.2 | Wire each command in the parity tracker to the client library, with matching output formatting | 3.2 to 3.6 | Per-command golden tests pass |
| 4.3 | Exit codes and error text for failures (no device, offline, unauthorized, server errors) | 4.2 | Matches C++ on the failure corpus |
| 4.4 | Environment variables and options: ADB\_SERVER\_SOCKET, ANDROID\_ADB\_SERVER\_PORT, ADB\_TRACE, -H, -P, -L | 4.1 | Each one has a test |

### Phase 5: adb-server and transport (TCP and emulator)

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 5.1 | `Transport` trait and the CNXN handshake (version, max payload, banner, features) | 1.1, 1.3 | Handshake completes with an emulator |
| 5.2 | Stream multiplexing: local and remote ids, OPEN, OKAY, WRTE, CLSE | 5.1 | Many parallel streams share one transport |
| 5.3 | Delayed-ack flow control, backpressure and large payloads | 5.2 | Throughput benchmark is within an agreed margin of C++ |
| 5.4 | TCP transport and emulator discovery | 5.1 | Running emulators are detected automatically |
| 5.5 | Device registry: states (offline, unauthorized, device), serials, transport ids, track-devices notifications | 2.4, 5.4 | Event stream matches C++ for connect, disconnect and auth |
| 5.6 | Smart-socket listener on 5037 and service routing (host, tcp, shell, sync and the rest) | 1.2, 5.5 | The unmodified C++ client passes the phase 3 commands against the Rust server |
| 5.7 | Server-side forward and reverse | 5.6 | `forward --list` and no-rebind behave like C++ |
| 5.8 | Server lifecycle: start, background, kill-server, version-mismatch restart, locking, logging | 5.6 | An IDE and a Gradle build connect to it without changes |

### Phase 6: USB transport

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 6.1 | Choose nusb or rusb in a short written decision, then enumerate devices, detect the ADB interface and read serials | 5.1 | Attached devices are listed |
| 6.2 | Bulk transfer loops, zero-length packets, max payload | 6.1 | Shell and push work over USB on Linux |
| 6.3 | Hotplug and reconnect handling | 6.2 | Unplug and replug mid-transfer recovers the way C++ does |
| 6.4 | macOS and Windows support, plus Linux udev rules and driver notes | 6.2 | Passes on a self-hosted runner for each OS |
| 6.5 | USB stress and error-injection tests | 6.3 | A long transfer soak completes without leaks or hangs |

### Phase 7: Wireless debugging

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 7.1 | mDNS discovery of the connect and pairing services | 5.5 | Discovered devices match the C++ mdns listing |
| 7.2 | SPAKE2 pairing with the pairing code and peer-info exchange | 2.1 | Pairs with a real device |
| 7.3 | TLS transport (STLS), certificate generation and custom verification; first confirm rustls supports keying-material export | 5.1, 7.2 | Connects to a device over Wi-Fi |
| 7.4 | `adb pair`, `connect` and `disconnect` for TLS devices, plus auto-reconnect | 7.1, 7.3 | End-to-end pair and connect works from the Rust CLI |

### Phase 8: Higher-level features

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 8.1 | install, install-multiple and uninstall through the `cmd` and `abb` paths | 3.4, 5.6 | A split APK installs and removes cleanly |
| 8.2 | Incremental install | 8.1 | Matches C++ behavior on supported devices and falls back correctly on others |
| 8.3 | logcat, bugreport and other shell-based commands | 3.4 | Output and exit codes match C++ |
| 8.4 | jdwp, track-jdwp and forward jdwp | 3.6 | A debugger attaches through the Rust server |
| 8.5 | reboot, root, unroot, remount, usb, tcpip, sideload | 5.6 | Each command is checked against an emulator |
| 8.6 | Every command still unchecked in the parity tracker | 8.1 to 8.5 | Tracker shows no open items or each has a recorded reason |

### Phase 9: Compatibility hardening

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 9.1 | Cross-compatibility matrix in CI: {C++, Rust} client x {C++, Rust} server x C++ adbd | 5.6 | All cells run the shared suite |
| 9.2 | Run the upstream Python tests against the Rust binary and triage every failure | 9.1 | Each failure is fixed or documented as intentional |
| 9.3 | Long differential-fuzzing campaigns on all parsers | 1.6 | No unexplained disagreement with C++ |
| 9.4 | Throughput and latency benchmarks for push, pull and shell, with CI regression gates | 5.3 | Gates fail the build beyond the agreed margin |
| 9.5 | Android-version matrix and Windows console and ConPTY pass | 9.1 | Suite passes on each supported version and OS |
| 9.6 | Shadow mode: run Rust beside C++ for a period and log behavior differences | 9.2 | Differences are triaged and the log is clean |
| 9.7 | Packaging, release pipeline and user documentation | 9.5 | Installable binaries for each OS |

### Phase 10: adbd

| ID | Task | Depends on | Done when |
| --- | --- | --- | --- |
| 10.1 | Feasibility spike on Android properties, SELinux, the auth service and the APEX build | 9.1 | Written plan for the on-device build |
| 10.2 | Port the on-device services built with Cargo and cross-compiled for Android, reusing the protocol crates | 10.1 | Rust adbd passes the matrix against the C++ server |

## Scope

The goal is a drop-in replacement for the whole toolchain. That puts adbd and full CLI parity in scope, so adbd becomes the final phase rather than an optional one. Scope is everything in the repository, not only the three programs: shared libraries (crypto, TLS, pairing, mDNS, platform abstractions, protobuf definitions), on-device helpers, tools and tests. Add a repository inventory to phase 0 that maps each directory to a new crate, a wrapper around existing code, or an explicit decision to leave it as is (for example pieces that must stay in Java or C++ on the device). Take the directory list from the live tree.
