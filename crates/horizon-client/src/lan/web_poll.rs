//! The part of mio the network threads use (`io_pool`), for the browser, which has no sockets:
//! a [`Poll`] that only waits for its [`Waker`]. That is all the integrated server's own player
//! needs: its in-memory pipe wakes the thread when bytes arrive. Registering a TCP stream fails.

use std::io;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interest(u8);

impl Interest {
    pub const READABLE: Interest = Interest(1);
    pub const WRITABLE: Interest = Interest(2);
}

impl std::ops::BitOr for Interest {
    type Output = Interest;
    fn bitor(self, other: Interest) -> Interest {
        Interest(self.0 | other.0)
    }
}

#[derive(Default)]
struct Shared {
    woken: Mutex<bool>,
    wake: Condvar,
}

pub struct Registry {
    shared: Arc<Shared>,
}

impl Registry {
    pub fn register<S: ?Sized>(&self, _source: &mut S, _token: Token, _interest: Interest) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "no sockets in the browser"))
    }
    pub fn deregister<S: ?Sized>(&self, _source: &mut S) -> io::Result<()> {
        Ok(())
    }
}

pub struct Poll {
    registry: Registry,
}

impl Poll {
    pub fn new() -> io::Result<Poll> {
        Ok(Poll { registry: Registry { shared: Arc::default() } })
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    /// Waits until the waker is woken or `timeout` passes. No socket events: `events` is left
    /// empty.
    pub fn poll(&mut self, _events: &mut Events, timeout: Option<Duration>) -> io::Result<()> {
        let shared = &self.registry.shared;
        let woken = shared.woken.lock().unwrap_or_else(PoisonError::into_inner);
        let mut woken = match timeout {
            Some(t) => shared.wake.wait_timeout_while(woken, t, |w| !*w).unwrap_or_else(PoisonError::into_inner).0,
            None => shared.wake.wait_while(woken, |w| !*w).unwrap_or_else(PoisonError::into_inner),
        };
        *woken = false;
        Ok(())
    }
}

pub struct Waker {
    shared: Arc<Shared>,
}

impl Waker {
    pub fn new(registry: &Registry, _token: Token) -> io::Result<Waker> {
        Ok(Waker { shared: registry.shared.clone() })
    }
    pub fn wake(&self) -> io::Result<()> {
        *self.shared.woken.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.shared.wake.notify_one();
        Ok(())
    }
}

/// Socket events: never any in the browser.
pub struct Events;

impl Events {
    pub fn with_capacity(_capacity: usize) -> Events {
        Events
    }
}

pub struct Event(std::convert::Infallible);

impl Event {
    pub fn token(&self) -> Token {
        match self.0 {}
    }
    pub fn is_readable(&self) -> bool {
        match self.0 {}
    }
    pub fn is_read_closed(&self) -> bool {
        match self.0 {}
    }
    pub fn is_writable(&self) -> bool {
        match self.0 {}
    }
    pub fn is_write_closed(&self) -> bool {
        match self.0 {}
    }
    pub fn is_error(&self) -> bool {
        match self.0 {}
    }
}

impl<'a> IntoIterator for &'a Events {
    type Item = &'a Event;
    type IntoIter = std::iter::Empty<&'a Event>;
    fn into_iter(self) -> Self::IntoIter {
        std::iter::empty()
    }
}

pub mod net {
    use std::io::{self, Read, Write};

    /// A TCP stream: never registered in the browser, where `std::net` has no sockets.
    pub struct TcpStream(std::net::TcpStream);

    impl TcpStream {
        pub fn from_std(stream: std::net::TcpStream) -> TcpStream {
            TcpStream(stream)
        }
        pub fn shutdown(&self, how: std::net::Shutdown) -> io::Result<()> {
            self.0.shutdown(how)
        }
    }

    impl Read for TcpStream {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.0.read(out)
        }
    }

    impl Write for TcpStream {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.write(data)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }
}
