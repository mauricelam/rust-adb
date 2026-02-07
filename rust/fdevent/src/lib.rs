//! A Rust port of the `fdevent` library from the Android Open Source Project.
//!
//! This crate provides an event-driven I/O multiplexing library that is compatible
//! with the original C++ `fdevent` library's concepts. It uses the `mio` crate
//! under the hood to provide a cross-platform API for handling file descriptor events.
//!
//! ## Usage
//!
//! To use this crate, create an `Fdevent` instance and register file descriptors
//! with it, providing a handler that implements the `FdeventHandler` trait.
//!
//! ```rust,no_run
//! use fdevent::fdevent::{Fdevent, FdeventHandler};
//! use mio::{Interest, Token};
//! use mio::event::Event;
//! use sysdeps::AdbFd;
//! use std::sync::Arc;
//!
//! struct MyHandler;
//!
//! impl FdeventHandler for MyHandler {
//!     fn on_event(&mut self, event: &Event, _fdevent: &mut Fdevent) {
//!         if event.is_readable() {
//!             println!("readable event for token {:?}", event.token());
//!         }
//!     }
//!
//!     fn on_timeout(&mut self, _fdevent: &mut Fdevent) {
//!         println!("timeout event");
//!     }
//! }
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut fdevent = Fdevent::new()?;
//!     let (s1, s2) = sysdeps::net::adb_socketpair()?;
//!     let handler = Box::new(MyHandler);
//!     let token = fdevent.register(Arc::new(s1), handler, Interest::READABLE)?;
//!
//!     // Poll for events in a loop
//!     // fdevent.poll(None)?;
//!     Ok(())
//! }
//! ```

/// Contains the core `fdevent` implementation, including the `Fdevent` struct
/// and `FdeventHandler` trait.
pub mod fdevent;
