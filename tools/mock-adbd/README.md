# mock-adbd

`mock-adbd` is a lightweight TCP server that emulates the Android Debug Bridge daemon (`adbd`) wire protocol for testing ADB host servers and clients without requiring a physical device or emulator.

## Usage

### Command Line

```bash
cargo run -p mock-adbd -- --listen 127.0.0.1:0
```

#### Options

- `--listen <ADDR>`: Address to bind and listen on (default: `127.0.0.1:0`).

Upon starting, `mock-adbd` prints its bound IP address and port on the **first line** of stdout (e.g. `127.0.0.1:37413`), allowing test scripts or harnesses to read the dynamically allocated port.

### In-Process Testing

`mock-adbd` can also be embedded directly in Rust integration tests as a library:

```rust
use mock_adbd::MockAdbdServer;

#[tokio::test]
async fn test_something() {
    let server = MockAdbdServer::bind("127.0.0.1:0").await.unwrap();
    let addr = server.local_addr();

    // Interact with server at `addr`...

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
3. Omits `shell_v2` feature advertisement so the host defaults to standard `shell:` service requests.

### Supported Services

- **`shell:echo <text>`**:
  - Accepts stream (`A_OKAY`).
  - Sends `<text>\n` in a single `A_WRTE` packet.
  - Waits for host `A_OKAY` (enforcing the 1-WRTE-in-flight rule).
  - Sends `A_CLSE` to close the stream.
- **Other `shell:<cmd>` commands**:
  - Accepts stream (`A_OKAY`).
  - Sends `mock-adbd: unsupported command\n` in a single `A_WRTE`.
  - Waits for host `A_OKAY`.
  - Sends `A_CLSE`.
- **Unknown Services (not starting with `shell:`)**:
  - Refuses the stream immediately by sending `A_CLSE(0, host_local_id)`.
