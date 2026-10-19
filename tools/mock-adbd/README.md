# mock-adbd

`mock-adbd` is a lightweight mock implementation of the Android Debug Bridge daemon (`adbd`) wire protocol for testing ADB client/server tools without requiring an Android emulator or physical device.

## Usage

### Command Line Interface

To run `mock-adbd` as a standalone process:

```bash
cargo run -p mock-adbd -- --listen 127.0.0.1:0
```

On startup, `mock-adbd` prints its bound TCP socket address (e.g. `127.0.0.1:41235`) on the first line of output before accepting incoming ADB connections.

### In-Process Library

Tests can spawn `mock-adbd` in-process using its Rust API:

```rust
use mock_adbd::MockAdbdServer;

let server = MockAdbdServer::bind("127.0.0.1:0").await?;
let bound_addr = server.local_addr();

tokio::spawn(async move {
    server.run().await.unwrap();
});
```

## Protocol Implementation

### Handshake
- Upon connection, `mock-adbd` reads the host's `A_CNXN` packet (ignoring any preceding data per the ADB specification).
- It replies with a `A_CNXN` packet containing `A_VERSION` (`0x01000001`) and a device banner string (`device::ro.product.name=mock;ro.product.model=mock_model;ro.product.device=mock_device;features=`).
- No features are advertised (specifically omitting `shell_v2`), forcing host clients to fallback to standard `shell:` service framing.
- Device authentication (`A_AUTH`) is not required.

### Supported Services
- **`shell:echo <text>`**: Responds with `<text>\n` in a single `A_WRTE` packet, waits for host `A_OKAY`, and closes the stream with `A_CLSE`.
- **`shell:<unsupported_command>`**: Responds with `mock-adbd: unsupported command\n` in a single `A_WRTE`, waits for host `A_OKAY`, and closes the stream with `A_CLSE`.
- **Unknown Services**: Any service string not starting with `shell:` is directly refused with an `A_CLSE` packet (`arg0 = 0`, `arg1 = remote_id`) without sending `A_OKAY`.

### Flow Control & Resilience
- **1-WRTE-in-flight Rule**: `mock-adbd` never sends a second `A_WRTE` packet on a stream before receiving an `A_OKAY` acknowledgment from the host.
- **Error Handling**: `mock-adbd` validates 24-byte packet header magic (`magic == command ^ 0xffffffff`), maximum payload limit (`MAX_PAYLOAD`), and payload checksums. Malformed, oversized, or truncated packets result in graceful connection termination without panics.
