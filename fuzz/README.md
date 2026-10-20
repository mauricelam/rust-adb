# Fuzz Targets for ADB Protocol Parsers

This directory contains `cargo-fuzz` targets for fuzzing `adb-proto` decoders.

## Targets

- `fuzz_header`: Feeds arbitrary byte slices to `Header::decode` to verify packet header parsing robustly handles unexpected inputs without panicking.
- `fuzz_smart_socket_reply`: Feeds arbitrary byte slices to `parse_reply` and `parse_protocol_string` to verify smart socket framing decoders handle unexpected inputs safely.

## Prerequisites

1. Install Rust nightly toolchain:
   ```bash
   rustup toolchain install nightly
   ```

2. Install `cargo-fuzz`:
   ```bash
   cargo install cargo-fuzz
   ```

## Running Targets Locally

Navigate to the `fuzz/` directory and run a fuzz target:

```bash
cd fuzz

# Run header fuzz target
cargo +nightly fuzz run fuzz_header

# Run smart socket reply fuzz target
cargo +nightly fuzz run fuzz_smart_socket_reply
```

### Running for a Fixed Duration

To run a target for a fixed duration (e.g. 10 seconds):

```bash
cargo +nightly fuzz run fuzz_header -- -max_total_time=10
cargo +nightly fuzz run fuzz_smart_socket_reply -- -max_total_time=10
```

## Reproducing Crashes

If a fuzz target finds a crash or panic, the failing input artifact will be saved in `fuzz/artifacts/<target_name>/crash-<hash>`.

To reproduce a crash locally:

```bash
cargo +nightly fuzz run <target_name> fuzz/artifacts/<target_name>/<crash_filename>
```
