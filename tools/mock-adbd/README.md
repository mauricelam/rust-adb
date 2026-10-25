# mock-adbd

`mock-adbd` is a lightweight TCP server that emulates the Android Debug Bridge daemon (`adbd`) wire protocol for testing ADB host servers and clients without requiring a physical device or emulator.

## Usage

### Command Line

```bash
cargo run -p mock-adbd -- --listen 127.0.0.1:0
```

#### Options

- `--listen <ADDR>`: Address to bind and listen on (default: `127.0.0.1:0`).
- `--no-shell-v2`: Omit advertising the `shell_v2` feature in the connection banner, forcing clients to fall back to the v1 `shell:` protocol.
- `--no-stat-v2`: Omit advertising the `stat_v2` feature in the connection banner, forcing clients to fall back to v1 stat requests (`STAT`).
- `--no-ls-v2`: Omit advertising the `ls_v2` feature in the connection banner, forcing clients to fall back to v1 list requests (`LIST`).
- `--no-sendrecv-v2`: Omit advertising the `sendrecv_v2` feature in the connection banner, forcing clients to fall back to v1 file transfer requests (`SEND`/`RECV`).
- `--fixed-push-mkdir`: Advertise the `fixed_push_mkdir` feature in the connection banner.

Upon starting, `mock-adbd` prints its bound IP address and port on the **first line** of stdout (e.g. `127.0.0.1:37413`), allowing test scripts or harnesses to read the dynamically allocated port.

### In-Process Testing

`mock-adbd` can also be embedded directly in Rust integration tests as a library:

```rust
use mock_adbd::{MockAdbdOptions, MockAdbdServer};

#[tokio::test]
async fn test_something() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    // Pre-populate in-memory file tree
    server.fs().add_file("/data/test.bin", vec![1, 2, 3], 0o100644, 1000);

    // Or disable v2 features:
    let server_no_v2 = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            enable_shell_v2: false,
            enable_stat_v2: false,
            enable_ls_v2: false,
            enable_sendrecv_v2: false,
            fixed_push_mkdir: false,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    server.shutdown();
    server_no_v2.shutdown();
}
```

## Protocol & Supported Services

### Handshake

When a connection is established, `mock-adbd`:
1. Reads the host's initial `CNXN` header.
2. Replies with a `CNXN` message containing the protocol version (`A_VERSION = 0x01000001`), negotiated max payload, and a device banner identifying product properties:
   - `ro.product.name=mock_device`
   - `ro.product.model=mock_model`
   - `ro.product.device=mock_device`
3. Advertises `shell_v2`, `stat_v2`, `ls_v2`, and `sendrecv_v2` features by default (unless overridden via options).

### Supported Services

#### Shell V2 Service (`shell,<args>:<command>`)

When `shell_v2` is enabled, service requests matching `shell,<args>:<command>` (where `<args>` contains `v2`) use Shell V2 framing:

- **`echo <text>`**: writes `<text>\n` to stdout (`ShellPacketId::Stdout`) and exits 0 (`ShellPacketId::Exit`).
- **`exit <n>`**: exits with status code `n` (0 to 255).
- **`stderr <text>`**: writes `<text>\n` to stderr (`ShellPacketId::Stderr`) and exits 0.
- **`cat`**: copies incoming stdin packets (`ShellPacketId::Stdin`) to stdout until `ShellPacketId::CloseStdin` is received, then exits 0.
- **`env-term`**: writes the value of `TERM=<value>` from service arguments (or `unset\n` if omitted) to stdout and exits 0.
- **PTY Window Size (`pty` in arguments)**: each `ShellPacketId::WindowSizeChange` received is answered by writing `WINSZ <text>\n` to stdout, where `<text>` is the payload without trailing NUL.
- **Unsupported Shell V2 Commands**: writes `mock-adbd: unsupported command\n` to stderr (`ShellPacketId::Stderr`) and exits with status 127.

Flow control strictly obeys the 1-WRTE-in-flight rule: every outgoing `ShellPacket` is transmitted as an `A_WRTE` message and the next is only sent after receiving `A_OKAY` from the host.

#### Fallback Shell V1 Service (`shell:<cmd>`)

When `shell_v2` is disabled or a standard `shell:<cmd>` request is received:

- **`shell:echo <text>`**:
  - Accepts stream (`A_OKAY`).
  - Sends `<text>\n` in a single `A_WRTE` payload.
  - Waits for host `A_OKAY`.
  - Sends `A_CLSE` to close the stream.
- **Other `shell:<cmd>` commands**:
  - Accepts stream (`A_OKAY`).
  - Sends `mock-adbd: unsupported command\n` in a single `A_WRTE`.
  - Waits for host `A_OKAY`.
  - Sends `A_CLSE`.

#### File Sync Service (`sync:`)

The `sync:` service handles bidirectional file synchronization (`adb ls`, `adb pull`, `adb push`) backed by an in-memory file system (`InMemoryFs`). It supports both v1 and uncompressed v2 requests:

- **`STAT` / `STA2` / `LST2`**: returns file metadata (mode, size, mtime) or a wire error status (`error = 2` / ENOENT when path is missing).
- **`LIST` / `LIS2`**: lists entries in a directory in stable order followed by `ID_DONE`.
- **`RECV` / `RCV2`**: transfers file data in `DATA` chunks up to 64 KiB followed by `ID_DONE`, or returns `ID_FAIL` with an error message when the file is missing or is a directory.
- **`SEND` / `SND2`**: receives file `DATA` chunks ending in `ID_DONE`, saving the file to `InMemoryFs` and responding with `ID_OKAY` on success or `ID_FAIL` if the parent directory does not exist.
- **`QUIT`**: cleanly terminates the sync session (`A_CLSE`).

Flow control strictly obeys the 1-WRTE-in-flight rule for all outgoing packets.

#### Unknown Services

- Refuses any unknown service (not starting with `shell:` or `sync:`) immediately by sending `A_CLSE(0, host_local_id)`.
