# mock-adbd

`mock-adbd` is a lightweight TCP server that emulates the Android Debug Bridge daemon (`adbd`) wire protocol for testing ADB host servers and clients without requiring a physical device or emulator.

## Usage

### Command Line

```bash
cargo run -p mock-adbd -- --listen 127.0.0.1:0
```

#### Options

- `--listen <ADDR>`: Address to bind and listen on (default: `127.0.0.1:0`).
- `--no-shell-v2`: Omit advertising the `shell_v2` feature in the connection banner.
- `--no-stat-v2`: Omit advertising the `stat_v2` feature in the connection banner.
- `--no-ls-v2`: Omit advertising the `ls_v2` feature in the connection banner.
- `--no-sendrecv-v2`: Omit advertising the `sendrecv_v2` feature in the connection banner.
- `--fixed-push-mkdir`: Advertise the `fixed_push_mkdir` feature in the connection banner (default: false).

Upon starting, `mock-adbd` prints its bound IP address and port on the **first line** of stdout (e.g. `127.0.0.1:37413`), allowing test scripts or harnesses to read the dynamically allocated port.

### In-Process Testing

`mock-adbd` can also be embedded directly in Rust integration tests as a library with an in-memory file tree (`InMemoryFs`):

```rust
use mock_adbd::{InMemoryFs, MockAdbdOptions, MockAdbdServer};

#[tokio::test]
async fn test_sync_operations() {
    let fs = InMemoryFs::new();
    fs.add_dir("/data", 0o040755, 1000);
    fs.add_file("/data/hello.txt", b"hello world".to_vec(), 0o100644, 2000);

    let server = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            fs: fs.clone(),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // Inspect files after client pushes or modifies the filesystem:
    let pushed = server.fs().get_file("/data/hello.txt");
    assert_eq!(pushed, Some(b"hello world".to_vec()));

    server.shutdown();
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
3. Advertises features by default: `shell_v2`, `stat_v2`, `ls_v2`, `sendrecv_v2` (unless turned off via flags).

### Supported Services

#### File Sync Service (`sync:`)

The `sync:` service manages file transfers and directory queries backed by `InMemoryFs`:

- **STAT v1 (`STAT`) & STAT v2 (`STA2`, `LST2`)**: returns file metadata (mode, size, mtime) or wire error (`ENOENT = 2` for missing files in v2, zero stat in v1).
- **LIST v1 (`LIST`) & LIST v2 (`LIS2`)**: lists directory entries in stable sorted order (`DENT` / `DNT2`), terminated by `DONE`.
- **RECV v1 (`RECV`) & RECV v2 (`RCV2`)**: streams uncompressed file contents in `DATA` chunks of at most 64 KiB, terminated by `DONE` (or `FAIL` if missing/invalid).
- **SEND v1 (`SEND`) & SEND v2 (`SND2`)**: receives file data chunks and `DONE` with modified time, verifies parent directory existence, updates `InMemoryFs`, and acknowledges with `OKAY` (or `FAIL` on missing parent directory).
- **QUIT**: terminates the file sync session and closes the stream.

#### Shell V2 Service (`shell,<args>:<command>`)

When `shell_v2` is enabled, service requests matching `shell,<args>:<command>` (where `<args>` contains `v2`) use Shell V2 framing:

- **`echo <text>`**: writes `<text>\n` to stdout (`ShellPacketId::Stdout`) and exits 0 (`ShellPacketId::Exit`).
- **`exit <n>`**: exits with status code `n` (0 to 255).
- **`stderr <text>`**: writes `<text>\n` to stderr (`ShellPacketId::Stderr`) and exits 0.
- **`cat`**: copies incoming stdin packets (`ShellPacketId::Stdin`) to stdout until `ShellPacketId::CloseStdin` is received, then exits 0.
- **`env-term`**: writes the value of `TERM=<value>` from service arguments (or `unset\n` if omitted) to stdout and exits 0.
- **`mkdir <args>`**: creates the specified directories in `InMemoryFs` and exits 0.
- **PTY Window Size (`pty` in arguments)**: each `ShellPacketId::WindowSizeChange` received is answered by writing `WINSZ <text>\n` to stdout, where `<text>` is the payload without trailing NUL.
- **Unsupported Shell V2 Commands**: writes `mock-adbd: unsupported command\n` to stderr (`ShellPacketId::Stderr`) and exits with status 127.

#### Fallback Shell V1 Service (`shell:<cmd>`)

When `shell_v2` is disabled or a standard `shell:<cmd>` request is received:

- **`shell:echo <text>`**: accepts stream (`A_OKAY`), sends `<text>\n`, and closes (`A_CLSE`).
- **`shell:mkdir <args>`**: creates directory in `InMemoryFs` and closes stream.
- **Other `shell:<cmd>` commands**: sends `mock-adbd: unsupported command\n` and closes stream.

#### Unknown Services

- Refuses any unknown service immediately by sending `A_CLSE(0, host_local_id)`.
