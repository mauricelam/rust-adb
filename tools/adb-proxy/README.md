# adb-proxy

`adb-proxy` is a logging TCP proxy tool that forwards traffic unchanged between an ADB client and server (or server and device) while recording each transferred data chunk to a file.

## Usage

```bash
cargo run --package adb-proxy -- --listen <addr> --target <addr> --out <file>
```

### Command-Line Arguments

- `--listen <addr>`, `-l <addr>`: Local socket address to listen on (e.g. `127.0.0.1:5037`).
- `--target <addr>`, `-t <addr>`: Target socket address to forward traffic to (e.g. `127.0.0.1:5038`).
- `--out <file>`, `-o <file>`: Output file path where recorded JSON Lines traffic will be saved.

## Recording Format

Traffic is recorded in JSON Lines (`.jsonl`) format, where each line is a self-contained JSON object representing a single chunk of transferred data.

### Fields

- `direction` (string): The direction of traffic flow for the chunk. Either `"client_to_server"` or `"server_to_client"`.
- `timestamp_us` (unsigned integer): Monotonic timestamp in microseconds elapsed since the proxy process started.
- `bytes` (string): Hexadecimal string representation of the raw bytes transferred in the chunk.

### Example

```json
{"direction":"client_to_server","timestamp_us":1024,"bytes":"0004686f7374"}
{"direction":"server_to_client","timestamp_us":2048,"bytes":"4f4b4159"}
```
