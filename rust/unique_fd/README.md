# Porting adb_unique_fd to Rust

The C++ `adb_unique_fd` is a simple RAII wrapper around a file descriptor.
Its purpose is to ensure that a file descriptor is closed when the `unique_fd`
object goes out of scope, preventing resource leaks.

## Rust Equivalent

In Rust, this pattern is built into the language through the ownership system
and the `Drop` trait. Standard library types that handle file descriptors,
such as `std::fs::File`, `std::net::TcpStream`, and `std::os::unix::net::UnixStream`,
automatically close the underlying file descriptor when they are dropped.

Therefore, a direct port of `adb_unique_fd` is not necessary. Instead, you
should use the appropriate standard library type for your use case.

### Working with Raw File Descriptors

If you have a raw file descriptor that you need to manage, you can use the
`FromRawFd` trait to create a managed object. This is available on Unix-like
systems in the `std::os::unix::io` module. A similar trait, `FromRawSocket`, exists for Windows.

**Example (Unix):**

```rust
use std::fs::File;
use std::os::unix::io::{FromRawFd, AsRawFd};

fn take_ownership_of_fd(fd: i32) {
    // Unsafe because the caller must ensure that `fd` is a valid file
    // descriptor and that no other object is responsible for closing it.
    let file = unsafe { File::from_raw_fd(fd) };

    // Now, `file` owns the file descriptor.
    // It will be automatically closed when `file` goes out of scope.

    println!("File descriptor {} is now managed.", file.as_raw_fd());
} // `file` is dropped and the file descriptor is closed here.

fn main() {
    // Let's create a file to get a file descriptor.
    let file = File::create("foo.txt").unwrap();
    let fd = file.as_raw_fd();

    // To pass ownership to our function, we need to forget the original file
    // object so that it doesn't close the file descriptor.
    std::mem::forget(file);

    take_ownership_of_fd(fd);

    // After this point, using `fd` would be incorrect as it has been closed.
}
```

## Summary

-   There is no need to create a `unique_fd` struct in Rust.
-   Use standard library types like `std::fs::File` and `std::net::TcpStream`
    whenever possible.
-   When you need to take ownership of a raw file descriptor, use the
    `FromRawFd` trait (on Unix) or `FromRawSocket`/`FromRawHandle` (on Windows).
