//! What the I/O side tells the relay, and what the relay asks of it.

use std::sync::Arc;

use xmoq_moqt::Error;
use xmoq_moqt::data::{SubgroupHeader, SubgroupObject};
use xmoq_moqt::fetch::RangeEnd;
use xmoq_moqt::message::ControlMessage;
use xmoq_moqt::name::Location;
use xmoq_moqt::session::Role;

use crate::cache::CachedObject;

/// Names one MOQT session. The I/O side picks the numbers and never reuses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionKey(pub u64);

/// Names one stream the peer opened in a session: a request stream or a
/// subgroup stream. The I/O side picks the numbers; they are unique within the
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StreamKey(pub u64);

/// Names one subgroup stream the relay opens. The relay picks the numbers and
/// never reuses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutStreamKey(pub u64);

/// Names a request stream the relay opens, across all sessions. The relay
/// never reuses one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnRequestKey(pub u64);

/// What a fetch stream the relay writes itself carries after its header, in
/// the order of the stream ([`Action::WriteFetchStream`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchPiece {
    /// An Object from the cache of its track.
    Object(CachedObject),
    /// An End of Range indicator with this Location (§11.4.4.2).
    EndOfRange(RangeEnd, Location),
}

/// What ended on a request stream the relay opened. "Own" is the relay's
/// word for the requests it opens, next to the requests a peer opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnRequestEnd {
    /// The stream could not be opened, or the request could not be written:
    /// the request never reached the peer.
    Unsent,
    /// The peer finished its direction.
    Fin,
    /// The peer reset its direction.
    Reset,
    /// The peer sent `STOP_SENDING` for the relay's direction. The peer's own
    /// direction is still open, and may still carry an answer.
    Stopped,
}

/// How a peer ended a request stream it opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestEnd {
    /// It finished its direction and did nothing else. That is not a
    /// cancellation: §5.1 has a subscriber end a subscription with
    /// `STOP_SENDING`, and §3.3.2 names a reset and `STOP_SENDING` for
    /// cancelling a request.
    Fin,
    /// It reset its direction or sent `STOP_SENDING` for the relay's, which
    /// cancels the request (§3.3.2), or nothing more can be read because this
    /// side stopped the stream.
    Aborted,
}

/// How a subgroup stream of a peer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// With a FIN: every Object of the Subgroup was delivered (§11.4.3).
    Fin,
    /// Without one: reset by the peer, or stopped by the I/O side. The code
    /// is the peer's Stream Reset Error Code when it gave one.
    Aborted(Option<u32>),
    /// Cut off by the end of the peer's connection.
    Lost,
}

/// How the passing on of a fetch stream ended ([`Action::PipeFetch`]). In
/// every case but the first the I/O side has reset the stream to the
/// subscriber and stopped the one from the publisher, with the codes given
/// here; the relay does what is left to do on the request streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchPipeEnd {
    /// The publisher's stream ended with a FIN, and so did the subscriber's.
    Finished,
    /// The publisher reset its stream, with this code. The subscriber's was
    /// reset with what [`crate::fetch_reset_passed_on`] makes of it.
    UpstreamAborted(Option<u32>),
    /// The publisher's connection ended. The subscriber's stream was reset
    /// with `UNKNOWN_OBJECT_STATUS`.
    UpstreamLost,
    /// Nothing came from the publisher for the time [`Action::PipeFetch`]
    /// gave, while the subscriber could take more. The subscriber's stream
    /// was reset with `UNKNOWN_OBJECT_STATUS`.
    UpstreamStalled,
    /// The subscriber sent `STOP_SENDING` for the stream, with this code.
    DownstreamStopped(Option<u32>),
    /// The subscriber took nothing for the time [`Action::PipeFetch`] gave.
    /// Its stream was reset with `TOO_FAR_BEHIND`.
    DownstreamStalled,
    /// The subscriber's connection ended, or no stream to it could be
    /// opened.
    DownstreamLost,
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
    /// The peer ended a request stream it opened, in the way `how` says.
    RequestClosed {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// How.
        how: RequestEnd,
    },
    /// A NAMESPACE or `NAMESPACE_DONE` the relay sent on a request stream of
    /// the peer ([`Action::SendRequest`]) was written to the stream. The relay
    /// sends the next ones of a `SUBSCRIBE_NAMESPACE` as these come.
    NamespaceWritten {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// A message arrived on a request stream the relay opened
    /// ([`Action::OpenRequest`]).
    OwnRequestMessage {
        /// The session.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
        /// The message.
        message: ControlMessage,
    },
    /// Something ended on a request stream the relay opened, or the stream
    /// never came about. What that means for the request is for the relay to
    /// say; the I/O side reports what it saw.
    OwnRequestClosed {
        /// The session.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
        /// What happened.
        how: OwnRequestEnd,
    },
    /// The time a publisher gets to answer a SUBSCRIBE of the relay is over.
    /// Answers [`Action::StartUpstreamTimer`] and
    /// [`Action::StartFetchStreamTimer`].
    UpstreamTimeout {
        /// The session of the publisher.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
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
    /// The `FETCH_HEADER` of a unidirectional stream of the peer arrived.
    FetchStreamOpened {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The Request ID of the header.
        request_id: u64,
    },
    /// The passing on of a fetch stream ended by itself. Not reported after
    /// [`Action::AbortFetchPipe`]: an I/O side whose task ended at the same
    /// moment drops the report.
    FetchPipeEnded {
        /// The session of the subscriber.
        session: SessionKey,
        /// The request stream of the subscriber's FETCH.
        stream: StreamKey,
        /// How.
        how: FetchPipeEnd,
    },
    /// The I/O side is done with [`Action::WriteFetchStream`]: the stream was
    /// written and finished, or given up.
    FetchStreamWritten {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
        /// `None` for a stream that was written and finished. For one that
        /// was given up, the Stream Reset Error Code that says why; what was
        /// opened of it was reset with the same.
        given_up: Option<u32>,
    },
    /// The fields of an Object before its payload arrived on a subgroup
    /// stream, and its payload is still on the way: its first byte is
    /// received (§5.1.2). [`Event::Object`] of the same Object follows once
    /// it is whole, and does not where the stream ends first. Not reported
    /// for an Object that is whole when its fields are read.
    ObjectBegun {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// The Object ID of the Object.
        object_id: u64,
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
    /// The subscriber of a subgroup stream the relay asked for stopped it
    /// with `STOP_SENDING`. Reported before [`Event::OutStreamClosed`] of
    /// the stream.
    OutStreamStopped {
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
        /// The stream whose Object, or whose opening, went over the limit.
        out_stream: OutStreamKey,
        /// For each stream to the same subscriber session that is not done,
        /// the payload bytes it has carried. The relay ends the subscription
        /// whose streams carry the most.
        carried: Vec<(OutStreamKey, usize)>,
    },
    /// The time to wait for the last Objects of a track after `PUBLISH_DONE` is
    /// over. Answers [`Action::StartDrainTimer`].
    DrainTimeout {
        /// The session of the publisher.
        session: SessionKey,
        /// The publisher's Track Alias.
        track_alias: u64,
        /// The `wait` of the action this answers.
        wait: u64,
    },
    /// The time to hold a stream whose Track Alias is not known is over.
    /// Answers [`Action::StartEarlyTimer`].
    EarlyStreamTimeout {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// The time to read a stream for the check alone is over. Answers
    /// [`Action::StartCheckTimer`].
    CheckStreamTimeout {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// The time a subscriber had to end its direction of the request stream
    /// of a FETCH that was served is over. Answers
    /// [`Action::StartFetchEndTimer`].
    FetchEndTimeout {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
    },
    /// The session may announce one namespace more. Answers
    /// [`Action::StartAnnouncementTimer`].
    AnnouncementTimeout {
        /// The session.
        session: SessionKey,
    },
    /// The time a SUBSCRIBE is held for a publisher to appear is over
    /// (§10.2.6). Answers [`Action::StartRendezvousTimer`].
    RendezvousTimeout {
        /// The session.
        session: SessionKey,
        /// The request stream of the SUBSCRIBE.
        stream: StreamKey,
    },
    /// The time for the SUBSCRIBE of a Joining FETCH to come is over. Answers
    /// [`Action::StartJoiningFetchTimer`].
    JoiningFetchTimeout {
        /// The session.
        session: SessionKey,
        /// The request stream of the Joining FETCH.
        stream: StreamKey,
    },
    /// The time a FETCH is held for an Object the relay is receiving is over.
    /// Answers [`Action::StartReceiptTimer`].
    ReceiptTimeout {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
    },
    /// The time a peer gets to end its direction of a request stream that
    /// the relay finished its own direction of is over. Reported for every
    /// [`Action::FinishRequest`], whatever became of the stream since.
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
    /// No session with the upstream relay came of
    /// [`Action::ConnectUpstream`].
    UpstreamFailed,
    /// The relay is to stop: every session is told with GOAWAY (§3.6, §10.4),
    /// and a session that opens later is told after its SETUP. A session of
    /// which the relay is the client gets an empty New Session URI (§10.4: "A
    /// client MUST send a zero-length New Session URI").
    GoingAway(GoingAway),
}

/// What the GOAWAY of a relay that is to stop says ([`Event::GoingAway`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoingAway {
    /// Where the clients continue; empty for the URI they used.
    pub new_session_uri: Vec<u8>,
    /// How long the relay waits before it closes the sessions.
    pub timeout_ms: u64,
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
    /// End the relay's half of a request stream with a FIN, and report
    /// [`Event::RequestTimeout`] after the time the peer gets to end its
    /// half.
    ///
    /// WHY the two in one action: the relay finishes its half when it has
    /// said its last, with a refusal, `PUBLISH_DONE` or the end of a FETCH,
    /// and nothing makes the peer end its own. Until it does, the stream
    /// counts as one of those the peer may have open, and the request with
    /// it where the session still holds it. A time limit that had to be
    /// asked for next to every FIN would be left out somewhere.
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
        /// What the subscriber asked for with `SUBGROUP_DELIVERY_TIMEOUT`
        /// on its SUBSCRIBE (§8, §10.2.3), in milliseconds: how long after
        /// the stream's FIN it may take to have all of it, before the
        /// stream is reset with `DELIVERY_TIMEOUT`. `None`: it asked for
        /// nothing, or for 0.
        subgroup_delivery_timeout: Option<u64>,
        /// Where the stream stands among the streams to this session
        /// ([`crate::send_order`]): of two that have something to send,
        /// the one with the larger number goes first (§7.2).
        send_order: i32,
    },
    /// The subscriber changed the `SUBGROUP_DELIVERY_TIMEOUT` of its
    /// subscription with a `REQUEST_UPDATE` (§10.9). Sent for every stream
    /// of the subscription that is not done; the I/O side lets it hold for
    /// the ones it has not been told to finish (see
    /// docs/moqt-implementation-notes.md, the row on a delivery timeout
    /// that changes).
    SetDeliveryTimeout {
        /// The stream.
        out_stream: OutStreamKey,
        /// As in [`Action::OpenDataStream`].
        subgroup_delivery_timeout: Option<u64>,
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
    /// Pass a fetch stream of a publisher on to the subscriber whose FETCH it
    /// answers: open a unidirectional stream to the subscriber, write a
    /// `FETCH_HEADER` with `request_id`, and then the bytes of the
    /// publisher's stream as they are, no faster than the subscriber takes
    /// them. The end is reported with [`Event::FetchPipeEnded`].
    PipeFetch {
        /// The session of the publisher.
        from: SessionKey,
        /// The publisher's fetch stream.
        from_stream: StreamKey,
        /// The session of the subscriber.
        session: SessionKey,
        /// The request stream of the subscriber's FETCH, which names the
        /// pipe from here on.
        stream: StreamKey,
        /// The Request ID of the subscriber's FETCH.
        request_id: u64,
        /// How long the publisher may send nothing while the subscriber can
        /// take more, in milliseconds.
        upstream_wait_ms: u64,
        /// How long the subscriber may take nothing, in milliseconds.
        downstream_wait_ms: u64,
        /// Where the stream to the subscriber stands among the streams to
        /// its session, as in [`Action::OpenDataStream`].
        send_order: i32,
    },
    /// End the passing on of a fetch stream: reset the stream to the
    /// subscriber with `code` and stop the publisher's with `CANCELLED`.
    /// Does nothing when the pipe has ended.
    AbortFetchPipe {
        /// The session of the subscriber.
        session: SessionKey,
        /// The request stream of the subscriber's FETCH.
        stream: StreamKey,
        /// A Stream Reset Error Code (§3.3.3).
        code: u32,
    },
    /// Open a unidirectional stream to the peer, write a `FETCH_HEADER` with
    /// `request_id` and then `pieces`, one after the other, and end the
    /// stream with a FIN. The end is reported with
    /// [`Event::FetchStreamWritten`].
    ///
    /// The pieces are of one Group or indicators alone, so they are encoded
    /// alike in either Group Order.
    WriteFetchStream {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH, which names the stream in that
        /// report.
        stream: StreamKey,
        /// The Request ID of the FETCH the stream answers.
        request_id: u64,
        /// What follows the header. The Objects are shared with the cache
        /// and with the subgroup streams they went out on.
        pieces: Vec<FetchPiece>,
        /// How long the subscriber may take nothing, in milliseconds, before
        /// the stream is reset with `TOO_FAR_BEHIND`: for the stream to be
        /// opened with its header, and for each piece.
        wait_ms: u64,
        /// Where the stream stands among the streams to the session, as in
        /// [`Action::OpenDataStream`].
        send_order: i32,
    },
    /// The subscriber changed the `SUBSCRIBER_PRIORITY` of its FETCH with a
    /// `REQUEST_UPDATE` (§10.9): the fetch stream of
    /// [`Action::PipeFetch`] or [`Action::WriteFetchStream`] that answers it
    /// stands by `send_order` from the next piece it writes, or from when it
    /// opens. Does nothing when that stream has ended.
    SetFetchSendOrder {
        /// The session of the subscriber.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
        /// As in [`Action::OpenDataStream`].
        send_order: i32,
    },
    /// Stop writing a fetch stream of [`Action::WriteFetchStream`] and reset
    /// it with `code`. The end is reported as for a stream that was given
    /// up, with this code. Does nothing when the stream is written.
    AbortFetchWrite {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
        /// A Stream Reset Error Code (§3.3.3).
        code: u32,
    },
    /// Nothing to do but to say it: the relay refused a FETCH, or stopped a
    /// fetch stream it writes, for what its fetch streams keep alive of
    /// Groups its cache has let go of (`MAX_BYTES_HELD_BEYOND_THE_CACHE`).
    /// The numbers are what was kept when it decided.
    NoteHeldBeyondCache {
        /// The session of the FETCH.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
        /// The FETCH was refused; else its stream was stopped.
        refused: bool,
        /// Bytes kept for the relay as a whole.
        relay_bytes: usize,
        /// Bytes kept for the session.
        session_bytes: usize,
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
    /// Drop the timer of [`Action::StartDrainTimer`] for this wait: the
    /// source it was for is over, by the streams it counted or otherwise.
    ///
    /// WHY, though the event of such a timer does nothing: the timer counts
    /// among those its session may have until it runs out, and a publisher
    /// that ends its tracks in quick succession would have none left for
    /// the wait it needs.
    StopDrainTimer {
        /// The session of the publisher.
        session: SessionKey,
        /// The wait, as [`Action::StartDrainTimer`] numbered it.
        wait: u64,
    },
    /// Report [`Event::DrainTimeout`] after the wait for late Objects.
    StartDrainTimer {
        /// The session of the publisher.
        session: SessionKey,
        /// The publisher's Track Alias.
        track_alias: u64,
        /// Which wait this is, to be handed back in the event. WHY: a Track
        /// Alias is used again, the timer of an earlier wait under it cannot
        /// be stopped, and it must not end a later one.
        wait: u64,
    },
    /// Report [`Event::CheckStreamTimeout`] after `wait_ms`. WHY no action
    /// to stop it: a track has [`crate::MAX_CHECKED_STREAMS_PER_TRACK`] such
    /// streams at once, so few such timers run, and one that runs out after
    /// its stream ended does nothing.
    StartCheckTimer {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
        /// How long, in milliseconds.
        wait_ms: u64,
    },
    /// Drop the timer of [`Action::StartEarlyTimer`] for this stream: it is
    /// not held any longer (its PUBLISH came or was refused, or it was given
    /// up). WHY, as for [`Action::StopDrainTimer`]: the timer counts among
    /// those its session may have until it runs out.
    StopEarlyTimer {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// Drop the timer of [`Action::StartJoiningFetchTimer`] for this FETCH:
    /// its SUBSCRIBE came, or the FETCH is over. WHY: as for
    /// [`Action::StopEarlyTimer`].
    StopJoiningFetchTimer {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
    },
    /// Report [`Event::EarlyStreamTimeout`] after the time a stream is held
    /// for a Track Alias that is not known yet.
    StartEarlyTimer {
        /// The session.
        session: SessionKey,
        /// The stream.
        stream: StreamKey,
    },
    /// Report [`Event::FetchEndTimeout`] after `wait_ms`: a FETCH was served
    /// to the end of its fetch stream, and the relay's direction of its
    /// request stream stays open that long for the subscriber to end its
    /// own first.
    StartFetchEndTimer {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
        /// The wait, in milliseconds.
        wait_ms: u64,
    },
    /// Report [`Event::AnnouncementTimeout`] after `wait_ms`, the time in
    /// which a session may announce one namespace: the session has
    /// announced, and gets back what it used of its allowance at that rate.
    StartAnnouncementTimer {
        /// The session.
        session: SessionKey,
        /// The wait, in milliseconds.
        wait_ms: u64,
    },
    /// Report [`Event::RendezvousTimeout`] after `wait_ms`: the relay holds
    /// a SUBSCRIBE for a track nobody publishes, for as long as its
    /// `RENDEZVOUS_TIMEOUT` and the relay's own limit allow.
    StartRendezvousTimer {
        /// The session.
        session: SessionKey,
        /// The request stream of the SUBSCRIBE.
        stream: StreamKey,
        /// The wait, in milliseconds.
        wait_ms: u64,
    },
    /// Report [`Event::JoiningFetchTimeout`] after the time a stream of the
    /// peer gets to say what it is: the relay holds a Joining FETCH whose
    /// SUBSCRIBE it has not read yet, and the SUBSCRIBE has that long to
    /// come.
    StartJoiningFetchTimer {
        /// The session.
        session: SessionKey,
        /// The request stream of the Joining FETCH.
        stream: StreamKey,
    },
    /// Report [`Event::ReceiptTimeout`] after `wait_ms`: the relay holds a
    /// FETCH whose range has an Object it is receiving, until the Object is
    /// whole and the cache can answer.
    StartReceiptTimer {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
        stream: StreamKey,
        /// The wait, in milliseconds.
        wait_ms: u64,
    },
    /// Drop the timer of [`Action::StartReceiptTimer`] for this FETCH: it is
    /// not held any longer. WHY: as for [`Action::StopEarlyTimer`].
    StopReceiptTimer {
        /// The session.
        session: SessionKey,
        /// The request stream of the FETCH.
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
    /// Open a bidirectional stream to the peer and send `message`, a request
    /// of the relay's own, on it. What arrives on the stream is reported with
    /// [`Event::OwnRequestMessage`] and [`Event::OwnRequestClosed`].
    OpenRequest {
        /// The session.
        session: SessionKey,
        /// The name the relay gives the request.
        request: OwnRequestKey,
        /// The request.
        message: ControlMessage,
    },
    /// Send one more message on a request stream the relay opened, after the
    /// request: a `REQUEST_UPDATE` of its SUBSCRIBE.
    SendOwnRequest {
        /// The session.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
        /// The message.
        message: ControlMessage,
    },
    /// Finish the relay's direction of a request stream it opened, and stop
    /// reading the peer's.
    FinishOwnRequest {
        /// The session.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
    },
    /// Cancel a request the relay opened: reset its direction and send
    /// `STOP_SENDING` for the peer's, with a Stream Reset Error Code
    /// (§3.3.2).
    CancelOwnRequest {
        /// The session.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
        /// The code.
        code: u32,
    },
    /// Report [`Event::UpstreamTimeout`] after the time a publisher gets to
    /// answer a SUBSCRIBE of the relay. The timer ends when it has run out,
    /// with [`Action::FinishOwnRequest`] or [`Action::CancelOwnRequest`] for
    /// the request, or with [`Action::StopUpstreamTimer`]; asked for again, it
    /// replaces the one that runs for the request.
    ///
    /// WHY the relay ends the timer and the I/O side never does by itself:
    /// the relay also starts one anew, for the PUBLISH a publisher announces
    /// with `DUPLICATE_SUBSCRIPTION`. An I/O side that dropped the timer on
    /// what it saw arrive could drop the new one for an event it read before
    /// the relay had started it.
    ///
    /// For this and every other timer: an I/O side that cannot start one
    /// reports its event at once. A wait that is cut short costs the peer
    /// what it waited for; a wait that never ends costs the relay what it
    /// holds.
    StartUpstreamTimer {
        /// The session of the publisher.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
    },
    /// As [`Action::StartUpstreamTimer`], for the fetch stream a publisher
    /// owes after its `FETCH_OK`, with a wait of the request's own. It takes
    /// the place of a timer that runs for the request.
    StartFetchStreamTimer {
        /// The session of the publisher.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
        /// How long to wait, in milliseconds.
        wait_ms: u64,
    },
    /// Drop the timer of [`Action::StartUpstreamTimer`] or
    /// [`Action::StartFetchStreamTimer`] for this request: what it waited for
    /// has come, or is not wanted any more.
    StopUpstreamTimer {
        /// The session.
        session: SessionKey,
        /// The request.
        request: OwnRequestKey,
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
    /// Connect to the upstream relay ([`Relay::set_upstream`](crate::Relay::set_upstream)).
    /// The I/O side reports [`Event::SessionOpened`] with [`Role::Client`]
    /// once the session is there, or [`Event::UpstreamFailed`]. It is asked
    /// for again only after one of the two.
    ConnectUpstream {
        /// Where: the New Session URI of a GOAWAY of the upstream (§10.4),
        /// and empty for the place the relay was given.
        uri: Vec<u8>,
        /// How long to wait before connecting, in milliseconds.
        after_ms: u64,
    },
}
