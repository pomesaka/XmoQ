//! What the I/O side tells the relay, and what the relay asks of it.

use std::sync::Arc;

use xmoq_moqt::Error;
use xmoq_moqt::data::{SubgroupHeader, SubgroupObject};
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::session::Role;

/// Names one MOQT session. The I/O side picks the numbers and never reuses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionKey(pub u64);

/// Names one stream the peer opened in a session: a request stream or a
/// subgroup stream. The I/O side picks the numbers; they are unique within the
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamKey(pub u64);

/// Names one subgroup stream the relay opens. The relay picks the numbers and
/// never reuses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutStreamKey(pub u64);

/// How a subgroup stream of a peer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// With a FIN: every Object of the Subgroup was delivered (§11.4.3).
    Fin,
    /// Without one: reset by the peer, stopped by the I/O side, or lost. The
    /// code is the peer's Stream Reset Error Code when it gave one.
    Aborted(Option<u32>),
}

/// Something the I/O side observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A session was established on a connection.
    SessionOpened {
        /// The new session.
        session: SessionKey,
        /// Which end of the connection the relay is: the server for a client
        /// that connected to it, the client for a relay it connected to.
        role: Role,
    },
    /// A message arrived on the peer's control stream.
    ControlMessage {
        /// The session.
        session: SessionKey,
        /// The message.
        message: ControlMessage,
    },
    /// The peer opened a bidirectional stream; this is its first message.
    RequestOpened {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The message.
        first: ControlMessage,
    },
    /// A later message arrived on a request stream of the peer.
    RequestMessage {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The message.
        message: ControlMessage,
    },
    /// The peer ended a request stream it opened: it finished or reset its
    /// direction, or sent `STOP_SENDING` for the relay's (§3.3.2, §5.1).
    RequestClosed {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// The Subgroup Header of a unidirectional stream of the peer arrived.
    DataStreamOpened {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The header.
        header: SubgroupHeader,
    },
    /// An Object arrived on a subgroup stream.
    Object {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The Object.
        object: SubgroupObject,
    },
    /// A subgroup stream of the peer ended.
    DataStreamClosed {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// How it ended.
        end: StreamEnd,
    },
    /// A subgroup stream broke the object model (§2.4.2). The I/O side has
    /// stopped the stream and reports its end separately.
    TrackMalformed {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// A subgroup stream the relay asked for was opened. Answers
    /// [`Action::OpenDataStream`], unless the stream was reset first.
    OutStreamOpened {
        /// The stream.
        out_stream: OutStreamKey,
    },
    /// A subgroup stream the relay asked for is done: written out and
    /// finished, reset, or never opened. Reported once for every
    /// [`Action::OpenDataStream`], and last.
    OutStreamClosed {
        /// The stream.
        out_stream: OutStreamKey,
    },
    /// What waits to be written to the subscriber of this stream is over its
    /// limit: the subscriber does not keep up.
    SendQueueFull {
        /// The stream.
        out_stream: OutStreamKey,
    },
    /// The time to wait for the last Objects of a track after `PUBLISH_DONE` is
    /// over. Answers [`Action::StartDrainTimer`].
    DrainTimeout {
        /// The session of the publisher.
        session: SessionKey,
        /// The publisher's Track Alias.
        track_alias: u64,
    },
    /// The time to hold a stream whose Track Alias is not known is over.
    /// Answers [`Action::StartEarlyTimer`].
    EarlyStreamTimeout {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// The time a peer gets to close a request stream that the relay is done
    /// with is over. Answers [`Action::StartRequestTimer`].
    RequestTimeout {
        /// The session.
        session: SessionKey,
        /// The request stream.
        stream: StreamKey,
    },
    /// The I/O side found a protocol error in what the peer sent.
    SessionFailed {
        /// The session.
        session: SessionKey,
        /// The error.
        error: Error,
    },
    /// The session is gone, whoever closed it.
    SessionClosed {
        /// The session.
        session: SessionKey,
    },
}

/// Something for the I/O side to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Send a message on the relay's control stream of the session.
    SendControl {
        /// The session.
        session: SessionKey,
        /// The message.
        message: ControlMessage,
    },
    /// Send a message on a request stream the peer opened.
    SendRequest {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The message.
        message: ControlMessage,
    },
    /// End the relay's half of a request stream with a FIN.
    FinishRequest {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// Cancel a request: reset the relay's half and stop the peer's (§3.3.2).
    ResetRequest {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// A Stream Reset Error Code (§3.3.3).
        code: u32,
    },
    /// Open a subgroup stream to a subscriber and write the header. The I/O
    /// side reports [`Event::OutStreamOpened`] and [`Event::OutStreamClosed`]
    /// for it.
    OpenDataStream {
        /// The session of the subscriber.
        session: SessionKey,
        /// The name the relay gives the stream.
        out_stream: OutStreamKey,
        /// The header to write.
        header: SubgroupHeader,
    },
    /// Write an Object on a subgroup stream the relay opened.
    SendObject {
        /// The stream.
        out_stream: OutStreamKey,
        /// The Object, shared between the subscribers it goes to.
        object: Arc<SubgroupObject>,
    },
    /// End a subgroup stream the relay opened with a FIN, after what is queued.
    FinishDataStream {
        /// The stream.
        out_stream: OutStreamKey,
    },
    /// Reset a subgroup stream the relay opened, dropping what is queued, also
    /// when it was finished and is not written out yet.
    ResetDataStream {
        /// The stream.
        out_stream: OutStreamKey,
        /// A Stream Reset Error Code (§3.3.3).
        code: u32,
    },
    /// Tell the peer to stop sending on one of its subgroup streams.
    StopDataStream {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// A Stream Reset Error Code (§3.3.3).
        code: u32,
    },
    /// Report [`Event::DrainTimeout`] after the wait for late Objects.
    StartDrainTimer {
        /// The session of the publisher.
        session: SessionKey,
        /// The publisher's Track Alias.
        track_alias: u64,
    },
    /// Report [`Event::EarlyStreamTimeout`] after the time a stream is held
    /// for a Track Alias that is not known yet.
    StartEarlyTimer {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// Report [`Event::RequestTimeout`] after the time a peer gets to close
    /// its direction of a request stream the relay is done with.
    StartRequestTimer {
        /// The session.
        session: SessionKey,
        /// The request stream.
        stream: StreamKey,
    },
    /// Tell the peer to stop sending on a request stream (`STOP_SENDING`),
    /// with a Stream Reset Error Code. The relay's own direction is not
    /// touched.
    StopRequest {
        /// The session.
        session: SessionKey,
        /// The request stream.
        stream: StreamKey,
        /// The code.
        code: u32,
    },
    /// Close the session.
    CloseSession {
        /// The session.
        session: SessionKey,
        /// The Session Termination Error Code (§3.5).
        code: u32,
        /// Why, for the log and for the peer.
        reason: String,
    },
}
