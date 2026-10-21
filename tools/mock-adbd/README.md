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

Upon starting, `mock-adbd` prints its bound IP address and port on the **first line** of stdout (e.g. `127.0.0.1:37413`), allowing test scripts or harnesses to read the dynamically allocated port.

### In-Process Testing

`mock-adbd` can also be embedded directly in Rust integration tests as a library:

```rust
use mock_adbd::{MockAdbdOptions, MockAdbdServer};

#[tokio::test]
async fn test_something() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    // Or disable shell_v2 advertisement:
    let server_no_v2 = MockAdbdServer::bind_with_options(
        "127.0.0.1:0",
        MockAdbdOptions {
            enable_shell_v2: false,
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
3. Advertises the `shell_v2` feature by default (unless `--no-shell-v2` / `enable_shell_v2: false` is configured).

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

#### Unknown Services

- Refuses any unknown service (not starting with `shell:`) immediately by sending `A_CLSE(0, host_local_id)`.
