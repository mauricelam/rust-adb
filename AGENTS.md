# Project Context: ADB Rust Port

This project aims to create a modern, safe, and performant implementation of the Android Debug Bridge (ADB) protocol in Rust. This repository is structured to facilitate a "compare-and-port" workflow, ensuring high fidelity to the original logic while leveraging Rust's safety features.

## Directory Structure & Purpose

### 1. `original/`

* **Purpose**: Contains the reference C++ source code from the Android Open Source Project (AOSP).
* **Build System**: Uses `Android.bp` (Soong).
* **Agent Instructions**:
* Treat this as the **immutable source of truth**.
* **Documentation**: Consult `original/docs/dev` for high-level architectural overviews and protocol specifications provided by the AOSP maintainers.
* **Analysis**: When porting a feature, first analyze the corresponding files here to understand the state machine, packet headers, and socket handling.
* **Dependency Mapping**: Refer to `Android.bp` files to understand build-time dependencies, compiler flags, and conditional compilation logic used in the original.
* Do not modify files in this directory.



### 2. `rust/`

* **Purpose**: The primary workspace for the new Rust implementation.
* **Build System**: Uses **Cargo**.
* **Agent Instructions**:
* Follow idiomatic Rust patterns (ownership, Result-based error handling, and type safety).
* When porting over the code, make sure it's corresponding documentations are being ported over as well.
* All dependencies must be managed via `Cargo.toml`. Avoid manual linking of external libraries unless absolutely necessary.



### 3. `binaries/`

* **Purpose**: Stores compiled versions of both the original C++ ADB and the new Rust version for side-by-side execution.
* **Agent Instructions**:
* Use these for manual verification and "black-box" testing.
* If a behavior in the Rust implementation deviates from the original, use the binaries to capture and compare network traces (PCAPs).



### 4. `tests/`

* **Purpose**: Integration tests and parity verification suites.
* **Agent Instructions**:
* Focus on **Cross-Implementation Tests**: Write tests that can run against both the original C++ daemon and the new Rust client (and vice versa).
* Include "fuzzing" tests to ensure the Rust parser is more resilient to malformed packets than the original implementation.
* Tests should prefer to be written in Rust, but should not depend on code in the `rust/` directory. In cases where Rust does not work, it is acceptable (but discouraged) to write tests in Python.



---

## Agent Guidelines for Porting

### Phase 1: Protocol Analysis

Before writing Rust code, identify the specific ADB Service or Protocol Layer in `original/`. Document the expected byte sequences and state transitions. Refer to `original/docs/dev` for protocol nuances and `Android.bp` for library dependencies.

### Phase 2: Implementation

* **Memory Safety**: Replace all raw pointer manipulations and `memcpy` calls from the C++ source with safe Rust equivalents (slices, `split_at`, etc.).
* **Error Handling**: Map C++ integer error codes to descriptive Rust `Enums`.
* **Build Integration**: Ensure the `rust/` implementation builds cleanly using `cargo build`.

### Phase 3: Validation

Every new module in `rust/` must have a corresponding integration test in `tests/` that verifies parity with the AOSP implementation.


---

- Cryptographic operations should prefer to use the pure-rust implementations in https://github.com/rustcrypto
- USB operations should prefer to use rusb: https://docs.rs/rusb/latest/rusb/
- TCP operations should use the standard library implementation


## Inspirations

- https://github.com/GoogleChromeLabs/wadb
- https://github.com/cocool97/adb_client
- https://docs.rs/crate/mozdevice/0.5.4/source/