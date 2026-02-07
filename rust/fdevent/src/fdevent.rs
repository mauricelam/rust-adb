//! A Rust port of the `fdevent` library for ADB.
//!
//! This module provides a way to monitor multiple file descriptors for
//! readability and writability, and to execute callbacks when events occur.
//! It also supports timeouts and queuing functions to be run on the looper thread.

use anyhow::Result;
use mio::{Events, Interest, Poll, Token, Waker};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

use sysdeps::AdbFd;

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
    /// * `fdevent` - A mutable reference to the `Fdevent` context.
    fn on_event(&mut self, event: &mio::event::Event, fdevent: &mut Fdevent);

    /// Called when the timeout set for this file descriptor expires.
    ///
    /// # Arguments
    ///
    /// * `fdevent` - A mutable reference to the `Fdevent` context.
    fn on_timeout(&mut self, fdevent: &mut Fdevent);
}

const WAKER_TOKEN: Token = Token(usize::MAX);

/// A handle to an [`Fdevent`] instance that can be used to queue functions from other threads.
///
/// This provides a thread-safe way to interact with the event loop's run queue.
#[derive(Clone)]
pub struct FdeventHandle {
    /// The queue of functions to be executed on the looper thread.
    run_queue: Arc<Mutex<Vec<Box<dyn FnOnce(&mut Fdevent) + Send>>>>,
    /// The waker used to interrupt the poll call when a new function is queued.
    waker: Arc<Waker>,
}

impl FdeventHandle {
    /// Queues a function to be executed on the looper thread.
    ///
    /// This corresponds to the `fdevent_run_on_looper` C++ function.
    ///
    /// # Arguments
    ///
    /// * `f` - The function to be executed. It receives a reference to the `Fdevent` context.
    pub fn run_on_looper<F>(&self, f: F)
    where
        F: FnOnce(&mut Fdevent) + Send + 'static,
    {
        {
            let mut queue = self.run_queue.lock().unwrap();
            queue.push(Box::new(f));
        }
        self.waker.wake().expect("failed to wake fdevent");
    }
}

/// A wrapper around platform-specific `mio` sources.
pub enum MioSource {
    #[cfg(unix)]
    Fd(i32),
    #[cfg(windows)]
    TcpStream(Option<mio::net::TcpStream>),
}

#[cfg(windows)]
impl Drop for MioSource {
    fn drop(&mut self) {
        if let MioSource::TcpStream(ref mut s) = self {
            if let Some(stream) = s.take() {
                // We need to make sure the mio stream doesn't close the socket when dropped,
                // because AdbFd still owns it.
                use std::os::windows::io::IntoRawSocket;
                let std_stream = std::net::TcpStream::from(stream);
                let _ = std_stream.into_raw_socket();
            }
        }
    }
}

impl mio::event::Source for MioSource {
    fn register(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            MioSource::Fd(fd) => {
                use mio::unix::SourceFd;
                registry.register(&mut SourceFd(fd), token, interests)
            }
            #[cfg(windows)]
            MioSource::TcpStream(Some(s)) => registry.register(s, token, interests),
            #[cfg(windows)]
            MioSource::TcpStream(None) => unreachable!(),
        }
    }

    fn reregister(
        &mut self,
        registry: &mio::Registry,
        token: Token,
        interests: Interest,
    ) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            MioSource::Fd(fd) => {
                use mio::unix::SourceFd;
                registry.reregister(&mut SourceFd(fd), token, interests)
            }
            #[cfg(windows)]
            MioSource::TcpStream(Some(s)) => registry.reregister(s, token, interests),
            #[cfg(windows)]
            MioSource::TcpStream(None) => unreachable!(),
        }
    }

    fn deregister(&mut self, registry: &mio::Registry) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            MioSource::Fd(fd) => {
                use mio::unix::SourceFd;
                registry.deregister(&mut SourceFd(fd))
            }
            #[cfg(windows)]
            MioSource::TcpStream(Some(s)) => registry.deregister(s),
            #[cfg(windows)]
            MioSource::TcpStream(None) => unreachable!(),
        }
    }
}

/// The main event loop context.
///
/// This struct manages the polling of file descriptors and the execution of handlers.
pub struct Fdevent {
    /// The `mio::Poll` instance used for I/O multiplexing.
    poll: Poll,
    /// The buffer for storing events returned by `mio::Poll::poll`.
    events: Events,
    /// A map from tokens to their corresponding event handlers.
    handlers: HashMap<Token, Box<dyn FdeventHandler>>,
    /// A map from tokens to their owned file descriptors, mio sources, and interests.
    fds: HashMap<Token, (Arc<AdbFd>, Option<MioSource>, Option<Interest>)>,
    /// A map from tokens to their registered timeouts.
    timeouts: HashMap<Token, (Instant, Duration)>,
    /// The queue of functions to be executed on the looper thread.
    run_queue: Arc<Mutex<Vec<Box<dyn FnOnce(&mut Fdevent) + Send>>>>,
    /// The waker used to interrupt the poll call.
    waker: Arc<Waker>,
    /// The counter used to generate unique tokens for registered file descriptors.
    next_token: usize,
}

impl Fdevent {
    /// Creates a new `Fdevent` context.
    pub fn new() -> FdeventResult<Self> {
        let poll = Poll::new()?;
        let events = Events::with_capacity(1024);
        let waker = Arc::new(Waker::new(poll.registry(), WAKER_TOKEN)?);
        Ok(Fdevent {
            poll,
            events,
            handlers: HashMap::new(),
            fds: HashMap::new(),
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
        self.poll
            .registry()
            .try_clone()
            .expect("failed to clone registry")
    }

    /// Registers a file descriptor to be monitored.
    ///
    /// # Arguments
    ///
    /// * `fd` - The file descriptor to monitor.
    /// * `handler` - The handler to execute when events occur.
    /// * `interest` - The initial set of events to monitor.
    pub fn register(
        &mut self,
        fd: Arc<AdbFd>,
        handler: Box<dyn FdeventHandler>,
        interest: Interest,
    ) -> FdeventResult<Token> {
        let token = Token(self.next_token);
        self.next_token += 1;

        let mut source = match () {
            #[cfg(unix)]
            () => {
                use std::os::unix::io::AsRawFd;
                MioSource::Fd(fd.as_raw_fd())
            }
            #[cfg(windows)]
            () => {
                use std::os::windows::io::{AsRawSocket, FromRawSocket};
                let s_raw = fd.as_raw_socket();
                if s_raw != windows_sys::Win32::Networking::WinSock::INVALID_SOCKET as _ {
                    // SAFETY: s_raw is a valid socket.
                    let stream = unsafe { std::net::TcpStream::from_raw_socket(s_raw as _) };
                    MioSource::TcpStream(Some(mio::net::TcpStream::from_std(stream)))
                } else {
                    return Err(FdeventError::Io(io::Error::new(
                        io::ErrorKind::Other,
                        "Only sockets are supported on Windows fdevent",
                    )));
                }
            }
        };

        self.poll
            .registry()
            .register(&mut source, token, interest)?;

        self.handlers.insert(token, handler);
        self.fds.insert(token, (fd, Some(source), Some(interest)));
        Ok(token)
    }

    /// Changes the set of monitored events for a registered file descriptor.
    ///
    /// # Arguments
    ///
    /// * `token` - The token returned by [`Self::register`].
    /// * `interest` - The new set of events to monitor.
    pub fn reregister(&mut self, token: Token, interest: Interest) -> FdeventResult<()> {
        self.set_interests(token, Some(interest))
    }

    /// Sets the interests for a registered file descriptor.
    ///
    /// If interest is `None`, it deregisters from the poller but keeps the handler.
    pub fn set_interests(&mut self, token: Token, interest: Option<Interest>) -> FdeventResult<()> {
        let (_, source, current_interest) = self.fds.get_mut(&token).ok_or_else(|| {
            FdeventError::Io(io::Error::new(io::ErrorKind::NotFound, "Token not found"))
        })?;

        if *current_interest == interest {
            return Ok(());
        }

        if let Some(source) = source {
            match (*current_interest, interest) {
                (Some(_), Some(new)) => {
                    self.poll.registry().reregister(source, token, new)?;
                }
                (Some(_), None) => {
                    self.poll.registry().deregister(source)?;
                }
                (None, Some(new)) => {
                    self.poll.registry().register(source, token, new)?;
                }
                (None, None) => {}
            }
        }

        *current_interest = interest;
        Ok(())
    }

    /// Unregisters a file descriptor and removes its handler.
    ///
    /// # Arguments
    ///
    /// * `token` - The token returned by [`Self::register`].
    pub fn unregister(&mut self, token: Token) -> FdeventResult<Arc<AdbFd>> {
        let (fd, source, current_interest) = self.fds.remove(&token).ok_or_else(|| {
            FdeventError::Io(io::Error::new(io::ErrorKind::NotFound, "Token not found"))
        })?;

        if let Some(mut source) = source {
            if current_interest.is_some() {
                self.poll.registry().deregister(&mut source)?;
            }
        }

        self.handlers.remove(&token);
        self.timeouts.remove(&token);
        Ok(fd)
    }

    /// Sets a timeout for a registered file descriptor.
    pub fn set_timeout(&mut self, token: Token, timeout: Duration) -> FdeventResult<()> {
        if !self.fds.contains_key(&token) {
            return Err(FdeventError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                "Token not found",
            )));
        }
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
        let now = Instant::now();

        // To allow handlers to call methods on Fdevent (which requires &mut self),
        // we take the events and handlers out of self temporarily.
        let events = std::mem::replace(&mut self.events, Events::with_capacity(0));
        let tokens: Vec<Token> = events.iter().map(|e| e.token()).collect();

        for token in tokens {
            if token == WAKER_TOKEN {
                continue;
            }

            if let Some(mut handler) = self.handlers.remove(&token) {
                if let Some(event) = events.iter().find(|e| e.token() == token) {
                    handler.on_event(event, self);
                }

                if self.fds.contains_key(&token) && !self.handlers.contains_key(&token) {
                    self.handlers.insert(token, handler);
                }

                if let Some(timeout_data) = self.timeouts.get_mut(&token) {
                    timeout_data.0 = now;
                }
            }
        }
        self.events = events;

        let mut expired = Vec::new();
        for (token, (start, duration)) in &self.timeouts {
            if now.duration_since(*start) >= *duration {
                expired.push(*token);
            }
        }

        for token in expired {
            if let Some(mut handler) = self.handlers.remove(&token) {
                handler.on_timeout(self);

                if self.fds.contains_key(&token) && !self.handlers.contains_key(&token) {
                    self.handlers.insert(token, handler);
                }

                if let Some(timeout_data) = self.timeouts.get_mut(&token) {
                    timeout_data.0 = now;
                }
            }
        }

        self.flush_run_queue();
        Ok(())
    }

    /// Queues a function to be executed on the looper thread.
    pub fn run_on_looper<F>(&mut self, f: F)
    where
        F: FnOnce(&mut Fdevent) + Send + 'static,
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
                f(self);
            }
        }
    }
}
