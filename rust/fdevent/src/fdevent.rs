use anyhow::Result;
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};
use std::collections::HashMap;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FdeventError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid file descriptor: {0}")]
    InvalidFd(RawFd),
}

pub type FdeventResult<T> = Result<T, FdeventError>;

pub trait FdeventHandler {
    fn on_event(&mut self, events: &Events);
}

pub struct Fdevent {
    poll: Poll,
    events: Events,
    handlers: HashMap<Token, Box<dyn FdeventHandler>>,
    next_token: usize,
}

impl Fdevent {
    pub fn new() -> FdeventResult<Self> {
        let poll = Poll::new()?;
        let events = Events::with_capacity(1024);
        Ok(Fdevent {
            poll,
            events,
            handlers: HashMap::new(),
            next_token: 0,
        })
    }

    pub fn register<T: AsRawFd>(
        &mut self,
        fd: &T,
        handler: Box<dyn FdeventHandler>,
        interest: Interest,
    ) -> FdeventResult<Token> {
        let token = Token(self.next_token);
        self.next_token += 1;
        self.poll
            .registry()
            .register(&mut SourceFd(&fd.as_raw_fd()), token, interest)?;
        self.handlers.insert(token, handler);
        Ok(token)
    }

    pub fn reregister<T: AsRawFd>(
        &mut self,
        fd: &T,
        token: Token,
        interest: Interest,
    ) -> FdeventResult<()> {
        self.poll
            .registry()
            .reregister(&mut SourceFd(&fd.as_raw_fd()), token, interest)?;
        Ok(())
    }

    pub fn unregister<T: AsRawFd>(&mut self, fd: &T, token: Token) -> FdeventResult<()> {
        self.poll
            .registry()
            .deregister(&mut SourceFd(&fd.as_raw_fd()))?;
        self.handlers.remove(&token);
        Ok(())
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> FdeventResult<()> {
        self.poll.poll(&mut self.events, timeout)?;
        for event in self.events.iter() {
            if let Some(handler) = self.handlers.get_mut(&event.token()) {
                let mut events = Events::with_capacity(1);
                events.iter_mut().next().clone_from(Some(event));
                handler.on_event(&events);
            }
        }
        Ok(())
    }
}
