# ADB Tracing in Rust

This crate provides a tracing facility for the adb Rust implementation, inspired by the C++ `adb_trace` module. It uses the `log` and `env_logger` crates to provide a flexible and configurable logging solution.

## Usage

To use this tracing crate, first initialize it at the beginning of your application's `main` function by calling `adb_trace_init()`:

```rust
use trace::adb_trace_init;

fn main() {
    adb_trace_init();
    // Your application code here...
}
```

Then, you can use the macros from the `log` crate to log messages. The `target` of the log message should be set to the string representation of the `AdbTrace` enum variant you want to use.

```rust
use log::trace;

fn my_function() {
    trace!(target: "adb", "This is a trace message for the adb tag");
    trace!(target: "sockets", "This is a trace message for the sockets tag");
}
```

## Enabling Tracing

To see the trace messages, you need to set the `ADB_TRACE` environment variable to a comma-separated list of the trace tags you want to enable.

For example, to enable tracing for the `adb` and `sockets` tags, you would run your application like this:

```sh
ADB_TRACE=adb,sockets cargo run
```

### Enabling All Traces

You can enable all traces by setting the `ADB_TRACE` environment variable to "1" or "all":

```sh
ADB_TRACE=all cargo run
```

## Available Trace Tags

The following trace tags are available:

* `adb`
* `sockets`
* `packets`
* `transport`
* `rwx`
* `usb`
* `sync`
* `sysdeps`
* `jdwp`
* `services`
* `auth`
* `fdevent`
* `shell`
* `incremental`
* `mdns`
* `mdns_stack`
