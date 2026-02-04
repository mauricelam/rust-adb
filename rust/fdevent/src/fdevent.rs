//! A Rust port of the `fdevent` library for ADB.
//!
//! This module provides a way to monitor multiple file descriptors for
//! readability and writability, and to execute callbacks when events occur.
//! It also supports timeouts and queuing functions to be run on the looper thread.

use anyhow::Result;
#[cfg(unix)]
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token, Waker};
use std::collections::HashMap;
use std::io;
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Errors that can occur when using `fdevent`.
#[derive(Error, Debug)]
pub enum FdeventError {
    /// An I/O error occurred during polling or registration.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

/// A specialized `Result` type for `fdevent` operations.
pub type FdeventResult<T> = Result<T, FdeventError>;

/// A trait for handling events on file descriptors.
///
/// Users should implement this trait to define the behavior when a file
/// descriptor becomes readable, writable, or when a timeout occurs.
///
/// This replaces the `fd_func` and `fd_func2` callbacks in the original C++ implementation.
pub trait FdeventHandler: Send {
    /// Called when an event (read/write/error) occurs on the registered file descriptor.
    ///
    /// # Arguments
    ///
    /// * `event` - The `mio::event::Event` containing the details of the event.
    fn on_event(&mut self, event: &mio::event::Event, registry: &mio::Registry);

    /// Called when the timeout set for this file descriptor expires.
    fn on_timeout(&mut self);
}

const WAKER_TOKEN: Token = Token(usize::MAX);

/// A handle to an [`Fdevent`] instance that can be used to queue functions from other threads.
///
/// This provides a thread-safe way to interact with the event loop's run queue.
#[derive(Clone)]
pub struct FdeventHandle {
    /// The queue of functions to be executed on the looper thread.
    /// Ported from `fdevent_context::run_queue_`.
    run_queue: Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>,
    /// The waker used to interrupt the poll call when a new function is queued.
    /// Ported from the interrupt mechanism used in `fdevent_context::Interrupt()`.
    waker: Arc<Waker>,
}

impl FdeventHandle {
    /// Queues a function to be executed on the looper thread.
    ///
    /// This corresponds to the `fdevent_run_on_looper` C++ function.
    ///
    /// # Arguments
    ///
    /// * `f` - The function to be executed.
    pub fn run_on_looper<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        {
            let mut queue = self.run_queue.lock().unwrap();
            queue.push(Box::new(f));
        }
        self.waker.wake().expect("failed to wake fdevent");
    }
}

/// The main event loop context.
///
/// This struct manages the polling of file descriptors and the execution of handlers.
///
/// This corresponds to the `fdevent_context` class in the C++ implementation.
/// The ambient/global context from C++ is replaced by creating and passing
/// an instance of this struct.
pub struct Fdevent {
    /// The `mio::Poll` instance used for I/O multiplexing.
    /// Ported from the platform-specific polling mechanism (epoll/poll).
    poll: Poll,
    /// The buffer for storing events returned by `mio::Poll::poll`.
    events: Events,
    /// A map from tokens to their corresponding event handlers.
    /// Ported from `fdevent_context::installed_fdevents_`.
    handlers: HashMap<Token, Box<dyn FdeventHandler>>,
    /// A map from tokens to their registered timeouts.
    /// Ported from the `timeout` member in the C++ `fdevent` struct.
    timeouts: HashMap<Token, (Instant, Duration)>,
    /// The queue of functions to be executed on the looper thread.
    /// Ported from `fdevent_context::run_queue_`.
    run_queue: Arc<Mutex<Vec<Box<dyn FnOnce() + Send>>>>,
    /// The waker used to interrupt the poll call.
    /// Ported from the interrupt mechanism used in `fdevent_context::Interrupt()`.
    waker: Arc<Waker>,
    /// The counter used to generate unique tokens for registered file descriptors.
    /// Ported from `fdevent_context::fdevent_id_`.
    next_token: usize,
}

impl Fdevent {
    /// Creates a new `Fdevent` context.
    ///
    /// This corresponds to `fdevent_create_context` (internal) or the initialization
    /// of the ambient context in C++.
    pub fn new() -> FdeventResult<Self> {
        let poll = Poll::new()?;
        let events = Events::with_capacity(1024);
        let waker = Arc::new(Waker::new(poll.registry(), WAKER_TOKEN)?);
        Ok(Fdevent {
            poll,
            events,
            handlers: HashMap::new(),
            timeouts: HashMap::new(),
            run_queue: Arc::new(Mutex::new(Vec::new())),
            waker,
            next_token: 0,
        })
    }

    /// Returns a thread-safe [`FdeventHandle`] for this context.
    pub fn get_handle(&self) -> FdeventHandle {
        FdeventHandle {
            run_queue: self.run_queue.clone(),
            waker: self.waker.clone(),
        }
    }

    /// Returns a clone of the `mio::Registry` associated with this context.
    pub fn registry(&self) -> mio::Registry {
        self.poll.registry().try_clone().expect("failed to clone registry")
    }

    /// Registers a file descriptor to be monitored.
    ///
    /// This corresponds to `fdevent_create` in C++. Note that in this Rust
    /// implementation, the `Fdevent` context owns the handler.
    ///
    /// # Arguments
    ///
    /// * `fd` - The file descriptor to monitor.
    /// * `handler` - The handler to execute when events occur.
    /// * `interest` - The initial set of events to monitor (e.g., READABLE).
    pub fn register<T: AsRawFd>(
        &mut self,
        fd: &T,
        handler: Box<dyn FdeventHandler>,
        interest: Interest,
    ) -> FdeventResult<Token> {
        let token = Token(self.next_token);
        self.next_token += 1;
        #[cfg(unix)]
        self.poll
            .registry()
            .register(&mut SourceFd(&fd.as_raw_fd()), token, interest)?;
        #[cfg(not(unix))]
        return Err(FdeventError::Io(io::Error::new(
            io::ErrorKind::Other,
            "Not supported on this platform",
        )));

        self.handlers.insert(token, handler);
        Ok(token)
    }

    /// Changes the set of monitored events for a registered file descriptor.
    ///
    /// This corresponds to `fdevent_set`, `fdevent_add`, and `fdevent_del` in C++.
    ///
    /// # Arguments
    ///
    /// * `fd` - The file descriptor.
    /// * `token` - The token returned by [`Self::register`].
    /// * `interest` - The new set of events to monitor.
    pub fn reregister<T: AsRawFd>(
        &mut self,
        fd: &T,
        token: Token,
        interest: Interest,
    ) -> FdeventResult<()> {
        #[cfg(unix)]
        self.poll
            .registry()
            .reregister(&mut SourceFd(&fd.as_raw_fd()), token, interest)?;
        #[cfg(not(unix))]
        return Err(FdeventError::Io(io::Error::new(
            io::ErrorKind::Other,
            "Not supported on this platform",
        )));

        Ok(())
    }

    /// Unregisters a file descriptor and removes its handler.
    ///
    /// This corresponds to `fdevent_destroy` in C++.
    ///
    /// # Arguments
    ///
    /// * `fd` - The file descriptor.
    /// * `token` - The token returned by [`Self::register`].
    pub fn unregister<T: AsRawFd>(&mut self, fd: &T, token: Token) -> FdeventResult<()> {
        #[cfg(unix)]
        self.poll
            .registry()
            .deregister(&mut SourceFd(&fd.as_raw_fd()))?;
        #[cfg(not(unix))]
        return Err(FdeventError::Io(io::Error::new(
            io::ErrorKind::Other,
            "Not supported on this platform",
        )));

        self.handlers.remove(&token);
        self.timeouts.remove(&token);
        Ok(())
    }

    /// Sets a timeout for a registered file descriptor.
    ///
    /// This corresponds to `fdevent_set_timeout` in C++. If no events occur within
    /// the specified duration, `on_timeout` will be called on the handler.
    ///
    /// # Arguments
    ///
    /// * `_fd` - The file descriptor (kept for API compatibility with C++).
    /// * `token` - The token returned by [`Self::register`].
    /// * `timeout` - The timeout duration.
    pub fn set_timeout<T: AsRawFd>(
        &mut self,
        _fd: &T,
        token: Token,
        timeout: Duration,
    ) -> FdeventResult<()> {
        self.timeouts.insert(token, (Instant::now(), timeout));
        Ok(())
    }

    fn calculate_poll_timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut min_timeout: Option<Duration> = None;

        for (start, duration) in self.timeouts.values() {
            let deadline = *start + *duration;
            let timeout = if deadline > now {
                deadline.duration_since(now)
            } else {
                Duration::from_secs(0)
            };

            if let Some(min) = min_timeout {
                if timeout < min {
                    min_timeout = Some(timeout);
                }
            } else {
                min_timeout = Some(timeout);
            }
        }
        min_timeout
    }

    /// Polls for events and executes handlers.
    ///
    /// This corresponds to `fdevent_loop` in C++. It should be called in a loop.
    ///
    /// # Arguments
    ///
    /// * `timeout` - An optional maximum time to wait for events. If `None`, it
    ///   will wait indefinitely (unless internal timeouts or the run queue are active).
    pub fn poll(&mut self, timeout: Option<Duration>) -> FdeventResult<()> {
        let mut poll_timeout = self.calculate_poll_timeout();
        if let Some(t) = timeout {
            if let Some(p) = poll_timeout {
                if t < p {
                    poll_timeout = Some(t);
                }
            } else {
                poll_timeout = Some(t);
            }
        }

        self.poll.poll(&mut self.events, poll_timeout)?;
        for event in self.events.iter() {
            if event.token() == WAKER_TOKEN {
                continue;
            }
            if let Some(handler) = self.handlers.get_mut(&event.token()) {
                handler.on_event(event, self.poll.registry());
            }
        }

        let now = Instant::now();
        let mut expired = Vec::new();
        for (token, (start, duration)) in &self.timeouts {
            if now.duration_since(*start) >= *duration {
                expired.push(*token);
            }
        }

        for token in expired {
            if let Some(handler) = self.handlers.get_mut(&token) {
                handler.on_timeout();
            }
            self.timeouts.remove(&token);
        }

        self.flush_run_queue();
        Ok(())
    }

    /// Queues a function to be executed on the looper thread.
    ///
    /// This is a convenience method that calls `get_handle().run_on_looper(f)`.
    pub fn run_on_looper<F>(&mut self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.get_handle().run_on_looper(f);
    }

    /// Internal method to execute all functions in the run queue.
    fn flush_run_queue(&mut self) {
        loop {
            let mut queue = self.run_queue.lock().unwrap();
            if queue.is_empty() {
                break;
            }
            let mut pending: Vec<_> = queue.drain(..).collect();
            drop(queue);
            for f in pending.drain(..) {
                f();
            }
        }
    }
}
