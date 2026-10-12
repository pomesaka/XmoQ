//! The I/O side of `MoQ` Transport: moves messages and Objects between the
//! decoders of `xmoq-moqt` and the streams of a connection.
//!
//! The `XmoQ` client and relay both use this crate, so the handling of resets,
//! limits and deadlines exists once. The connection is a trait,
//! [`Connection`], with two implementations: [`wt`] over `web-transport-quinn`, and
//! [`memory`], a pair of in-process endpoints that the tests of this crate and
//! of its users run on.
//!
//! # What a connection has to do
//!
//! The traits below are the whole interface to the transport library. Their
//! documentation is a contract: the stream handling of this crate and the
//! relay rely on every sentence of it, and [`memory`] keeps to it, so that a
//! test that passes there says something about a real connection. An
//! implementation over another library has to keep to it as well.
//!
//! FIXME: read datagrams. Nothing here does, so an `OBJECT_DATAGRAM` is lost and
//! an unknown datagram type does not close the session, which §11 of
//! `draft-ietf-moq-transport-18` requires.

#[cfg(test)]
mod contract;
pub mod memory;
mod streams;
#[cfg(test)]
mod tests;
#[cfg(feature = "quic")]
pub mod wt;

use std::future::Future;

pub use streams::{
    Direction, FetchBody, FetchChunk, FetchStreamWriter, IncomingStreams, OutgoingEvent,
    OutgoingSubgroup, PeerRequestClose, PeerRequestStreamEvent, QueueFull, RequestClose,
    RequestReadError, RequestReader, RequestStreamEvent, SendBudget, SendError, SendLimits,
    StreamConfig, StreamEnd, StreamStops, Trace, UniEvent, open_fetch_stream,
    open_fetch_stream_without_header, read_request_streams, read_uni_streams, send_message,
};

/// Why reading from a stream stopped short of a FIN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The peer reset the stream. The code is its Stream Reset Error Code, or
    /// `None` when what arrived is not an application error code.
    Reset(Option<u32>),
    /// The connection is gone.
    Lost(String),
}

/// Why writing to a stream failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The peer sent `STOP_SENDING`. The code is as in [`ReadError::Reset`].
    Stopped(Option<u32>),
    /// The connection is gone.
    Lost(String),
}

/// How a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEnd {
    /// The peer closed the session, with this Session Termination Error Code
    /// and reason.
    ClosedByPeer {
        /// The code.
        code: u32,
        /// The reason, with what is not UTF-8 replaced.
        reason: String,
    },
    /// This endpoint closed the session, or the peer ended it in order
    /// without saying why.
    ClosedLocally,
    /// The connection went without a close: a timeout or a transport error.
    Lost(String),
}

/// The longest reason of a session close, in bytes of UTF-8.
/// `draft-ietf-webtrans-http3-16` §6: the Application Error Message of
/// `WT_CLOSE_SESSION` "MUST NOT exceed 1024 bytes", and a sender that
/// truncates "MUST do so at a UTF-8 character boundary".
pub const MAX_CLOSE_REASON_LEN: usize = 1024;

/// The reason of a close as it is sent: see [`Connection::close`].
fn close_reason(reason: &[u8]) -> String {
    let mut reason = String::from_utf8_lossy(reason).into_owned();
    let mut len = reason.len().min(MAX_CLOSE_REASON_LEN);
    while !reason.is_char_boundary(len) {
        len -= 1;
    }
    reason.truncate(len);
    reason
}

/// The connection is gone; nothing more can be opened or accepted on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionLost(pub String);

impl std::fmt::Display for ConnectionLost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "connection lost: {}", self.0)
    }
}

impl std::error::Error for ConnectionLost {}

/// The receiving half of a stream.
///
/// A half that is done with has to be ended by [`Self::stop`] unless it was
/// read to its FIN or to an error. What the peer sees when a half is merely
/// dropped is up to the implementation, and nothing in this crate relies on it.
pub trait RecvStream: Send + 'static {
    /// Reads into `buffer`. Returns the number of bytes, at least one, or `None`
    /// at a FIN.
    ///
    /// * Bytes arrive in the order they were written, and the FIN after all of
    ///   them.
    /// * Dropping the future loses nothing: bytes are taken only when it
    ///   completes.
    /// * Once the peer's reset has arrived, the call fails with
    ///   [`ReadError::Reset`]; bytes that were not read yet are gone.
    /// * Once the connection is gone, bytes and a FIN that had arrived before
    ///   are still read, and then the call fails with [`ReadError::Lost`].
    ///   Whether something had arrived is a matter of timing, unless the
    ///   sender waited for it with [`SendStream::finish_and_wait`].
    fn read(
        &mut self,
        buffer: &mut [u8],
    ) -> impl Future<Output = Result<Option<usize>, ReadError>> + Send;

    /// Tells the peer to stop sending (`STOP_SENDING`) with a Stream Reset
    /// Error Code. Does not wait for the peer.
    fn stop(self, code: u32);
}

/// The sending half of a stream.
///
/// A half that is done with has to be ended by [`Self::finish`] or
/// [`Self::reset`]. What the peer sees when a half is merely dropped is up to
/// the implementation, and nothing in this crate relies on it.
pub trait SendStream: Send + 'static {
    /// Writes all of `bytes`.
    ///
    /// * Completes when the transport has taken the bytes, not when the peer
    ///   has read them. It waits while the peer's flow control window is full,
    ///   which is how a receiver that does not read holds its sender back.
    /// * Fails with [`WriteError::Stopped`] once the peer sent `STOP_SENDING`.
    /// * Dropping the future may leave a prefix of `bytes` written.
    fn write_all(&mut self, bytes: &[u8]) -> impl Future<Output = Result<(), WriteError>> + Send;

    /// Ends the stream with a FIN after what was written. Does not wait: the
    /// transport delivers what is left and the FIN on its own, as long as the
    /// connection lives. What was written is not lost when the half is gone
    /// after this call.
    ///
    /// Closing the connection is another matter: [`Connection::close`] drops
    /// what the transport has not delivered yet. Whoever closes a connection
    /// right after the last write uses [`Self::finish_and_wait`] first.
    fn finish(self);

    /// Like [`Self::finish`], and completes when everything written and the
    /// FIN have reached the peer's transport, so that closing the connection
    /// afterwards does not take them back. It does not wait for the peer to
    /// read them.
    ///
    /// Fails with [`WriteError::Stopped`] when the peer sent `STOP_SENDING`
    /// first, and with [`WriteError::Lost`] when the connection went first.
    /// The caller bounds the wait: a peer that does not read may never let
    /// everything through.
    fn finish_and_wait(self) -> impl Future<Output = Result<(), WriteError>> + Send;

    /// Resets the stream with a Stream Reset Error Code. Does not wait. What
    /// was written and not yet delivered may be lost.
    fn reset(self, code: u32);

    /// Like [`Self::finish`], but what has not reached the peer's transport
    /// when `give_up` completes is given up: the stream is reset with the
    /// code `give_up` gave. `true` when everything and the FIN got there, or
    /// when the stream ended some other way; `false` when it was reset here.
    /// A reset can come after the peer had everything, of which this end
    /// has not heard yet: the peer then has the whole stream, and `false`
    /// says only that a reset was sent.
    ///
    /// WHY a default that only finishes: it is right for a transport that
    /// holds nothing between `write_all` and its peer, which the in-memory
    /// connection is (`memory`: what is written is in the peer's pipe).
    /// There is nothing left to give up at the FIN then. A transport with a
    /// send buffer has to implement this.
    fn finish_until<G>(self, give_up: G) -> impl Future<Output = bool> + Send
    where
        Self: Sized,
        G: Future<Output = u32> + Send,
    {
        drop(give_up);
        self.finish();
        std::future::ready(true)
    }

    /// Completes when the peer sent `STOP_SENDING`, with
    /// [`WriteError::Stopped`], or when the connection is gone, with
    /// [`WriteError::Lost`]. Dropping the future loses nothing.
    fn stopped(&mut self) -> impl Future<Output = WriteError> + Send;
}

/// One session-carrying connection.
pub trait Connection: Send + Sync + 'static {
    /// The sending half of its streams.
    type Send: SendStream;
    /// The receiving half of its streams.
    type Recv: RecvStream;

    /// Opens a unidirectional stream. Waits while the peer's limit on streams
    /// is reached, which is how a peer that does not read holds this endpoint
    /// back. Fails once the connection is gone.
    fn open_uni(&self) -> impl Future<Output = Result<Self::Send, ConnectionLost>> + Send;

    /// Opens a bidirectional stream, like [`Self::open_uni`].
    fn open_bi(
        &self,
    ) -> impl Future<Output = Result<(Self::Send, Self::Recv), ConnectionLost>> + Send;

    /// The next unidirectional stream the peer opened. Streams are not
    /// necessarily accepted in the order they were opened.
    ///
    /// Once the connection is gone the call fails. A stream that was not
    /// accepted by then may be lost, also when it had arrived: only the bytes
    /// and the FIN of streams that were already accepted are still read
    /// ([`RecvStream::read`]). So "the peer's transport has it"
    /// ([`SendStream::finish_and_wait`]) does not mean the peer will read it,
    /// and an endpoint that must know asks at the level above; see
    /// `docs/moqt-implementation-notes.md` for what `XmoQ` does after
    /// `PUBLISH_DONE`.
    ///
    /// A stream the peer reset right after opening it may never be accepted
    /// either.
    fn accept_uni(&self) -> impl Future<Output = Result<Self::Recv, ConnectionLost>> + Send;

    /// The next bidirectional stream the peer opened, like
    /// [`Self::accept_uni`].
    fn accept_bi(
        &self,
    ) -> impl Future<Output = Result<(Self::Send, Self::Recv), ConnectionLost>> + Send;

    /// Closes the session with a Session Termination Error Code and a reason,
    /// which reach the peer: its [`Self::closed`] completes with
    /// [`ConnectionEnd::ClosedByPeer`] and both.
    ///
    /// The reason is sent as UTF-8, with what is not UTF-8 replaced, and cut
    /// to at most [`MAX_CLOSE_REASON_LEN`] bytes at a character boundary.
    ///
    /// Does not wait. The connection is gone when [`Self::closed`] completes,
    /// which can take a moment, and until then calls on it may still succeed.
    /// A caller that ends its process right after closing waits for
    /// [`Self::closed`] first, or the peer may see the connection lost
    /// instead.
    ///
    /// Every stream of the connection is gone with it, and with them what was
    /// written and had not reached the peer yet: see
    /// [`SendStream::finish_and_wait`].
    fn close(&self, code: u32, reason: &[u8]);

    /// Completes when the connection is gone, and says how it ended.
    fn closed(&self) -> impl Future<Output = ConnectionEnd> + Send;

    /// How the path to the peer stands now, for a log. `None` from a
    /// connection that has no congestion control to speak of.
    fn path(&self) -> Option<PathNow> {
        None
    }
}

/// What a transport says of its path to the peer at one moment
/// ([`Connection::path`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathNow {
    /// The congestion window, in bytes.
    pub cwnd: u64,
    /// The round-trip time the transport reckons with.
    pub rtt: std::time::Duration,
    /// Packets counted as lost so far.
    pub lost_packets: u64,
    /// Times the congestion control was told of congestion so far.
    pub congestion_events: u64,
}
