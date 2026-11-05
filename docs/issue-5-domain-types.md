# Design: Domain types in place of strings across client, server, and CLI (Issue 5)

## Summary of Exploration and Architectural Plan

### 1. ConnectionState and Banner
- Move `ConnectionState` into `adb-proto::state`.
- Implement `FromStr`, `Display`, and `from_service_type`.
- Expose `system_type() -> ConnectionState` on `Banner` while preserving the raw string for exact round-trip banner formatting.

### 2. TransportSelector Unification
- Unify `TransportSelector` in `adb-proto::transport_selector` across `adb-client` and `adb-server`.
- Replace `TransportTarget` in `adb-server`.
- Update `parse_smart_socket_request` to return structured results with explicit, tested fallback semantics.

### 3. Services and SocketSpec
- Introduce `HostService` and `Service` enums in `adb-proto` with `Display` and `FromStr`.
- Introduce `SocketSpec` enum (`Tcp`, `LocalAbstract`, `LocalReserved`, `LocalFilesystem`, `DevRaw`, `Jdwp`) to eliminate string slicing (`starts_with("tcp:")`, `starts_with("dev-raw:")`).
- Replace string-prefix dispatch (`starts_with("host")`, `starts_with("transport")`) across smart-socket routing and session switching.

### 4. Typed Client Returns and CLI Formatting
- Transition `AdbClient` return types:
  - `get_state` -> `Outcome<ConnectionState>`
  - `devices` -> `Outcome<Vec<DeviceListing>>`
  - `features` -> `Outcome<Features>`
  - `transport_id` -> `Outcome<TransportId>`
- Remove `.replace('\n', ",")` string re-parsing across `sync_session.rs`, `forward.rs`, `reverse.rs`, and `shell.rs`.
- Move string formatting into `adb-cli`.

### 5. ServerAddress, Launcher Options, and DeviceEntry
- Refactor `ServerAddress` using `Host` enum (`Loopback`, `Name(String)`) and `ServerPort(pub u16)`.
- Pass structured `ServerLaunchOptions { address, listen_mode, one_device }` to launchers.
- Refactor `DeviceEntry` to use `serial: Serial` (`Emulator(ConsolePort)`, `Network { host, port }`, `Other(String)`) and `features: Features`.
- Parse `ANDROID_ADB_SERVER_PORT` cleanly in `bind_default` without swallowing errors.
