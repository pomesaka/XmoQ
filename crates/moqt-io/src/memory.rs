//! A [`Connection`] between two endpoints of one process, for tests.
//!
//! It keeps to the contract of the traits in the crate root, so that code
//! tested on it meets the same behaviour on a real connection: bytes arrive in
//! order, a stream holds a bounded amount that the receiver has not read, a
//! FIN, a reset and a `STOP_SENDING` each carry over with their code, and once
//! the connection is closed only what had arrived is still read from it. The
//! tests in `contract.rs` run against this connection and a real one.
//!
//! Where the contract leaves the behaviour open, this connection takes the
//! choice that makes a mistake visible:
//!
//! * A half that is dropped without being ended shows at the peer as a reset
//!   or a stop without a code, where QUIC libraries tend to send a FIN.
//! * When the connection is closed, a stream keeps its unread bytes and its
//!   FIN only if its sender waited for them to arrive
//!   ([`SendStream::finish_and_wait`]). On a real connection the rest is lost
//!   or not depending on timing; here it is always lost.
//! * A stream that was not accepted when the connection is closed is never
//!   accepted afterwards, whether or not it had arrived.
//!
//! * Nothing waits between a write and the peer: what `write_all` took is in
//!   the peer's pipe. A real connection has a send buffer, so a stream can
//!   be finished with part of it not delivered, and reset then
//!   ([`SendStream::finish_until`]); here a finished stream is delivered.
//!   What a writer does with a stream that is not delivered is tested on a
//!   connection that delivers when it is told to (`tests`), and the reset
//!   itself on the real connection (`contract`).
//!
//! It models nothing else of QUIC: no packets, no loss, no timing. Its windows
//! are far smaller than real ones ([`STREAM_WINDOW`] bytes a stream and
//! [`ACCEPT_QUEUE`] streams waiting to be accepted, against about 1.25 MB and
//! 100 with quinn's defaults), so that tests reach them with little data.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::{Notify, mpsc, watch};

use crate::{
    Connection, ConnectionEnd, ConnectionLost, ReadError, RecvStream, SendStream, WriteError,
};

/// Bytes a stream holds that its receiver has not read. A writer waits beyond
/// that, as flow control makes it wait on a real connection.
pub const STREAM_WINDOW: usize = 64;

/// Streams of one kind that wait to be accepted. Opening waits beyond that, as
/// the peer's stream limit makes it wait on a real connection.
pub const ACCEPT_QUEUE: usize = 4;

/// A stream half was ended without a FIN, with this code if one was given.
#[derive(Clone, Copy)]
struct Aborted(Option<u32>);

#[derive(Default)]
struct Pipe {
    bytes: VecDeque<u8>,
    finished: bool,
    /// The sender waited for everything to arrive ([`SendStream::finish_and_wait`]),
    /// so the bytes and the FIN outlive the connection.
    arrived: bool,
    /// The sender reset the stream.
    reset: Option<Aborted>,
    /// The receiver stopped the stream.
    stopped: Option<Aborted>,
}

struct Shared {
    pipe: Mutex<Pipe>,
    changed: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Pipe> {
        // A poisoned pipe is one whose other half panicked; its state is still
        // the best information there is.
        self.pipe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The sending half of an in-memory stream.
pub struct MemorySend {
    shared: Arc<Shared>,
    /// Bytes the stream holds unread before a write waits.
    window: usize,
    closed: watch::Receiver<bool>,
    /// `finish` or `reset` was called.
    ended: bool,
}

/// The receiving half of an in-memory stream.
pub struct MemoryRecv {
    shared: Arc<Shared>,
    closed: watch::Receiver<bool>,
    /// `stop` was called.
    ended: bool,
}

fn pipe(closed: &watch::Receiver<bool>, window: usize) -> (MemorySend, MemoryRecv) {
    let shared = Arc::new(Shared {
        pipe: Mutex::new(Pipe::default()),
        changed: Notify::new(),
    });
    (
        MemorySend {
            shared: Arc::clone(&shared),
            window,
            closed: closed.clone(),
            ended: false,
        },
        MemoryRecv {
            shared,
            closed: closed.clone(),
            ended: false,
        },
    )
}

const CLOSED: &str = "the connection was closed";

impl SendStream for MemorySend {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), WriteError> {
        let mut rest = bytes;
        while !rest.is_empty() {
            let notified = self.shared.changed.notified();
            {
                let mut pipe = self.shared.lock();
                if *self.closed.borrow() {
                    return Err(WriteError::Lost(CLOSED.to_string()));
                }
                if let Some(Aborted(code)) = pipe.stopped {
                    return Err(WriteError::Stopped(code));
                }
                let room = self.window.saturating_sub(pipe.bytes.len());
                let (now, later) = rest.split_at(room.min(rest.len()));
                if !now.is_empty() {
                    pipe.bytes.extend(now);
                    rest = later;
                    self.shared.changed.notify_waiters();
                    continue;
                }
            }
            tokio::select! {
                () = notified => {}
                _ = self.closed.changed() => {}
            }
        }
        Ok(())
    }

    fn finish(mut self) {
        self.ended = true;
        self.shared.lock().finished = true;
        self.shared.changed.notify_waiters();
    }

    fn finish_and_wait(mut self) -> impl Future<Output = Result<(), WriteError>> + Send {
        self.ended = true;
        let outcome = {
            let mut pipe = self.shared.lock();
            if *self.closed.borrow() {
                Err(WriteError::Lost(CLOSED.to_string()))
            } else if let Some(Aborted(code)) = pipe.stopped {
                Err(WriteError::Stopped(code))
            } else {
                // What was written is in the pipe, which stands for the peer's
                // transport: nothing is left to wait for.
                pipe.finished = true;
                pipe.arrived = true;
                Ok(())
            }
        };
        self.shared.changed.notify_waiters();
        std::future::ready(outcome)
    }

    fn reset(mut self, code: u32) {
        self.ended = true;
        self.abort(Some(code));
    }

    fn stopped(&mut self) -> impl Future<Output = WriteError> + Send {
        let shared = Arc::clone(&self.shared);
        let mut closed = self.closed.clone();
        async move {
            loop {
                let notified = shared.changed.notified();
                if *closed.borrow() {
                    return WriteError::Lost(CLOSED.to_string());
                }
                if let Some(Aborted(code)) = shared.lock().stopped {
                    return WriteError::Stopped(code);
                }
                tokio::select! {
                    () = notified => {}
                    _ = closed.changed() => {}
                }
            }
        }
    }
}

impl MemorySend {
    fn abort(&self, code: Option<u32>) {
        let mut pipe = self.shared.lock();
        if !pipe.finished && pipe.reset.is_none() {
            pipe.reset = Some(Aborted(code));
            // A reset drops what the receiver has not read yet.
            pipe.bytes.clear();
        }
        drop(pipe);
        self.shared.changed.notify_waiters();
    }
}

impl Drop for MemorySend {
    fn drop(&mut self) {
        // A half that goes with its connection says nothing about the stream.
        if !self.ended && !*self.closed.borrow() {
            // The contract leaves this open; see the module documentation.
            self.abort(None);
        }
    }
}

impl RecvStream for MemoryRecv {
    async fn read(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, ReadError> {
        loop {
            let notified = self.shared.changed.notified();
            {
                let mut pipe = self.shared.lock();
                let closed = *self.closed.borrow();
                if closed && !pipe.arrived {
                    return Err(ReadError::Lost(CLOSED.to_string()));
                }
                if let Some(Aborted(code)) = pipe.reset {
                    return Err(ReadError::Reset(code));
                }
                if !pipe.bytes.is_empty() {
                    let len = pipe.bytes.len().min(buffer.len());
                    for (slot, byte) in buffer.iter_mut().zip(pipe.bytes.drain(..len)) {
                        *slot = byte;
                    }
                    self.shared.changed.notify_waiters();
                    return Ok(Some(len));
                }
                if pipe.finished {
                    return Ok(None);
                }
                if closed {
                    return Err(ReadError::Lost(CLOSED.to_string()));
                }
            }
            tokio::select! {
                () = notified => {}
                _ = self.closed.changed() => {}
            }
        }
    }

    fn stop(mut self, code: u32) {
        self.ended = true;
        self.halt(Some(code));
    }
}

impl MemoryRecv {
    fn halt(&self, code: Option<u32>) {
        let mut pipe = self.shared.lock();
        if pipe.stopped.is_none() {
            pipe.stopped = Some(Aborted(code));
            pipe.bytes.clear();
        }
        drop(pipe);
        self.shared.changed.notify_waiters();
    }
}

impl Drop for MemoryRecv {
    fn drop(&mut self) {
        // A half that goes with its connection says nothing about the stream.
        if self.ended || *self.closed.borrow() {
            return;
        }
        let over = {
            let pipe = self.shared.lock();
            (pipe.finished && pipe.bytes.is_empty()) || pipe.reset.is_some()
        };
        if !over {
            // The contract leaves this open; see the module documentation.
            self.halt(None);
        }
    }
}

/// How one end of a [`pair`] was closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Close {
    /// The Session Termination Error Code.
    pub code: u32,
    /// The reason.
    pub reason: Vec<u8>,
    /// Which end closed: the first of the pair or the second.
    first: bool,
}

struct Incoming {
    uni: tokio::sync::Mutex<mpsc::Receiver<MemoryRecv>>,
    bi: tokio::sync::Mutex<mpsc::Receiver<(MemorySend, MemoryRecv)>>,
}

/// One end of an in-process connection.
pub struct MemoryConnection {
    to_peer_uni: mpsc::Sender<MemoryRecv>,
    to_peer_bi: mpsc::Sender<(MemorySend, MemoryRecv)>,
    incoming: Incoming,
    /// The window of the streams this end opens.
    window: usize,
    closed_sender: Arc<watch::Sender<bool>>,
    closed: watch::Receiver<bool>,
    close: Arc<Mutex<Option<Close>>>,
    /// Whether this is the first end of its pair.
    first: bool,
}

/// Two connected ends.
#[must_use]
pub fn pair() -> (MemoryConnection, MemoryConnection) {
    pair_with_window(STREAM_WINDOW)
}

/// Two connected ends whose streams hold `window` bytes unread before a
/// write waits, where [`pair`] has [`STREAM_WINDOW`]. WHY: a write of a few
/// bytes that waits for its reader is something a real connection does when
/// its windows are used up, and [`pair`] never does.
#[must_use]
pub fn pair_with_window(window: usize) -> (MemoryConnection, MemoryConnection) {
    let (a_uni, b_incoming_uni) = mpsc::channel(ACCEPT_QUEUE);
    let (b_uni, a_incoming_uni) = mpsc::channel(ACCEPT_QUEUE);
    let (a_bi, b_incoming_bi) = mpsc::channel(ACCEPT_QUEUE);
    let (b_bi, a_incoming_bi) = mpsc::channel(ACCEPT_QUEUE);
    let (closed_sender, closed) = watch::channel(false);
    let closed_sender = Arc::new(closed_sender);
    let close = Arc::new(Mutex::new(None));
    let end = |first, to_peer_uni, to_peer_bi, uni, bi| MemoryConnection {
        first,
        to_peer_uni,
        to_peer_bi,
        incoming: Incoming {
            uni: tokio::sync::Mutex::new(uni),
            bi: tokio::sync::Mutex::new(bi),
        },
        window,
        closed_sender: Arc::clone(&closed_sender),
        closed: closed.clone(),
        close: Arc::clone(&close),
    };
    (
        end(true, a_uni, a_bi, a_incoming_uni, a_incoming_bi),
        end(false, b_uni, b_bi, b_incoming_uni, b_incoming_bi),
    )
}

impl MemoryConnection {
    /// How the connection was closed, by either end, if it was.
    #[must_use]
    pub fn close_info(&self) -> Option<Close> {
        self.close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn lost() -> ConnectionLost {
        ConnectionLost(CLOSED.to_string())
    }

    async fn until_closed(&self) {
        let mut closed = self.closed.clone();
        // An error means the sender is gone, which is closed as well.
        let _ = closed.wait_for(|closed| *closed).await;
    }
}

impl Connection for MemoryConnection {
    type Send = MemorySend;
    type Recv = MemoryRecv;

    async fn open_uni(&self) -> Result<MemorySend, ConnectionLost> {
        let (send, recv) = pipe(&self.closed, self.window);
        tokio::select! {
            biased;
            () = self.until_closed() => return Err(Self::lost()),
            delivered = self.to_peer_uni.send(recv) => delivered.map_err(|_| Self::lost())?,
        }
        Ok(send)
    }

    async fn open_bi(&self) -> Result<(MemorySend, MemoryRecv), ConnectionLost> {
        let (send, peer_recv) = pipe(&self.closed, self.window);
        let (peer_send, recv) = pipe(&self.closed, self.window);
        tokio::select! {
            biased;
            () = self.until_closed() => return Err(Self::lost()),
            delivered = self.to_peer_bi.send((peer_send, peer_recv)) => {
                delivered.map_err(|_| Self::lost())?;
            }
        }
        Ok((send, recv))
    }

    async fn accept_uni(&self) -> Result<MemoryRecv, ConnectionLost> {
        let mut incoming = self.incoming.uni.lock().await;
        accept(&mut incoming, self.until_closed()).await
    }

    async fn accept_bi(&self) -> Result<(MemorySend, MemoryRecv), ConnectionLost> {
        let mut incoming = self.incoming.bi.lock().await;
        accept(&mut incoming, self.until_closed()).await
    }

    fn close(&self, code: u32, reason: &[u8]) {
        let mut close = self
            .close
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if close.is_none() {
            *close = Some(Close {
                code,
                reason: crate::close_reason(reason).into_bytes(),
                first: self.first,
            });
        }
        self.closed_sender.send_replace(true);
    }

    async fn closed(&self) -> ConnectionEnd {
        self.until_closed().await;
        match self.close_info() {
            Some(close) if close.first == self.first => ConnectionEnd::ClosedLocally,
            Some(close) => ConnectionEnd::ClosedByPeer {
                code: close.code,
                reason: String::from_utf8_lossy(&close.reason).into_owned(),
            },
            None => ConnectionEnd::Lost("the peer went away".to_string()),
        }
    }
}

/// The next stream of `incoming`. Closed first: a stream that was not
/// accepted when the connection went is gone with it, also one that had
/// arrived. See the module documentation.
async fn accept<S>(
    incoming: &mut mpsc::Receiver<S>,
    closed: impl Future<Output = ()>,
) -> Result<S, ConnectionLost> {
    tokio::select! {
        biased;
        () = closed => Err(MemoryConnection::lost()),
        stream = incoming.recv() => stream.ok_or_else(MemoryConnection::lost),
    }
}

impl Drop for MemoryConnection {
    fn drop(&mut self) {
        // An endpoint that goes away takes the connection with it.
        self.closed_sender.send_replace(true);
    }
}
