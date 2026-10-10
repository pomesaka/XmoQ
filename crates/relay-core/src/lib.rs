//! The routing and delivery decisions of the `XmoQ` relay.
//!
//! The crate does no I/O. [`Relay::handle`] takes one [`Event`] that the I/O
//! side observed and returns the [`Action`]s to carry out, in order. All MOQT
//! sessions of the relay live in the one [`Relay`], so a decision that touches
//! several sessions (an Object fanned out to subscribers, a publisher leaving)
//! is made in one place and can be tested without a socket.
//!
//! Section numbers (`§9.4`) refer to `draft-ietf-moq-transport-18`.
//!
//! What this first relay does not do, each a rule of the draft it does not meet:
//!
//! Several publishers of one track (§9.3, §9.5) are not handled: see the FIXME
//! at the top of `routing.rs`. [`TrackSource`] is one source of a track, and
//! [`Track`] holds one of them today.
//!
//! FIXME: apply the Subscription Filter of a SUBSCRIBE (§5.1.2). Every
//! subscriber gets the Objects that arrive after its subscription, which is what
//! the Largest Object filter asks for, and what the other filters do not.
//!
//! FIXME: pass a `NEW_GROUP_REQUEST` of a subscriber on to the publisher
//! (§10.2.13, MUST for a relay). It is ignored.
//!
//! FIXME: enforce `OBJECT_DELIVERY_TIMEOUT` and `SUBGROUP_DELIVERY_TIMEOUT`
//! (§8). They are ignored, as a parameter of a SUBSCRIBE and as a Track
//! Property.
//!
//! FIXME: hold a SUBSCRIBE that carries `RENDEZVOUS_TIMEOUT` until a publisher
//! appears (§10.2.6, SHOULD). It is rejected at once like any other.
//!
//! FIXME: share one encoding of an Object between subscribers. Each subscriber
//! gets its own copy of the payload today.

mod events;
mod routing;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

pub use events::{
    Action, Event, OutStreamKey, OwnRequestEnd, OwnRequestKey, RequestEnd, SessionKey, StreamEnd,
    StreamKey,
};
use routing::{Announcement, Ask, OwnRequest, PendingTrack, Refusal, Waiter};
pub use routing::{
    MAX_NAMESPACES_PER_SESSION, MAX_OWN_REQUESTS_PER_SESSION, MAX_PASSED_ON_REASON_LEN,
    MAX_WAITING_SUBSCRIBERS,
};
use xmoq_moqt::data::{SubgroupHeader, SubgroupId, SubgroupObject};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::{
    ControlMessage, Publish, ReasonPhrase, Subscribe, publish_done_status, request_error_code,
    setup_option,
};
use xmoq_moqt::name::{FullTrackName, Location};
use xmoq_moqt::params::{self, Parameter};
pub use xmoq_moqt::session::Role;
use xmoq_moqt::session::{
    AfterRefusal, PeerRequest, PeerRequestEvent, PeerRequestId, PeerRequestOpened, Session,
    SubscribedAlias, Transport,
};
use xmoq_moqt::{Error, RequestFault, TerminationCode, UsageError, stream_reset_code};

/// Subgroup streams of one session that are held because their Track Alias is
/// not known yet: the PUBLISH travels on another stream and may be read
/// later. §11.4.2 lets a receiver "buffer it for a brief period"; the I/O side
/// runs that period ([`Action::StartEarlyTimer`]).
///
/// Provisional: the draft sets no limit, and `XmoQ` has not decided one.
pub const MAX_EARLY_STREAMS: usize = 8;

/// Objects held across the early streams of one session. Provisional.
pub const MAX_EARLY_OBJECTS: usize = 32;

/// Bytes of payload and properties held across the early streams of one
/// session. Provisional.
///
/// WHY bytes on top of the count: one Object can be as large as the I/O side
/// lets through, so a count alone bounds nothing.
pub const MAX_EARLY_BYTES: usize = 1024 * 1024;

/// The Stream Count of a `PUBLISH_DONE` whose sender cannot give the exact
/// number (§10.11: "it MUST set Stream Count to 2^62 - 1").
const STREAM_COUNT_UNKNOWN: u64 = (1 << 62) - 1;

/// What a request stream of a peer is for.
///
/// `peer_finished`: the peer finished its direction of the stream while the
/// request goes on, and nothing more is read from it. The requests that can
/// go on that way are the ones that carry the mark; a PUBLISH is over with
/// its publisher's FIN.
#[derive(Debug)]
enum RequestSlot {
    Subscribe {
        request: PeerRequestId,
        track: FullTrackName,
        peer_finished: bool,
    },
    Publish {
        request: PeerRequestId,
        track: FullTrackName,
    },
    /// The namespace is in the relay's list of announced namespaces.
    PublishNamespace {
        request: PeerRequestId,
        peer_finished: bool,
    },
    /// A SUBSCRIBE that waits for the relay's own SUBSCRIBE to a publisher to
    /// be answered (§9.4).
    Waiting {
        request: PeerRequestId,
        track: FullTrackName,
        peer_finished: bool,
    },
    /// The relay has said its last on the stream. The request is given when the
    /// session still holds state for it: what the peer sends until the stream
    /// closes still goes to the session, which ignores it and keeps count of
    /// the Request IDs it uses up.
    Finished(Option<PeerRequestId>),
}

impl RequestSlot {
    fn peer_finished(&self) -> bool {
        match self {
            Self::Subscribe { peer_finished, .. }
            | Self::PublishNamespace { peer_finished, .. }
            | Self::Waiting { peer_finished, .. } => *peer_finished,
            Self::Publish { .. } | Self::Finished(_) => false,
        }
    }

    /// Notes that the peer finished its direction. `false` for a request
    /// that does not go on after that.
    fn note_peer_finished(&mut self) -> bool {
        match self {
            Self::Subscribe { peer_finished, .. }
            | Self::PublishNamespace { peer_finished, .. }
            | Self::Waiting { peer_finished, .. } => {
                *peer_finished = true;
                true
            }
            Self::Publish { .. } | Self::Finished(_) => false,
        }
    }
}

/// A subgroup stream that arrived before the PUBLISH that names its alias.
#[derive(Debug)]
struct EarlyStream {
    stream: StreamKey,
    header: SubgroupHeader,
    objects: Vec<SubgroupObject>,
    end: Option<StreamEnd>,
}

#[derive(Debug)]
struct SessionState {
    moqt: Session,
    requests: HashMap<StreamKey, RequestSlot>,
    /// Subgroup streams of this session that feed a track, by stream.
    in_streams: HashMap<StreamKey, FullTrackName>,
    early: VecDeque<EarlyStream>,
    /// Requests the relay opened on this session.
    own: HashMap<OwnRequestKey, OwnRequest>,
}

impl SessionState {
    /// Marks a request stream of the peer as one the relay has said its last
    /// on, with no state left for it in the session.
    ///
    /// WHY the slot goes at once when the peer had finished its direction: a
    /// slot is otherwise taken away when the stream is reported closed, and
    /// that report came already. It would stay for the life of the session.
    ///
    /// WHY the other places put `Finished` in directly: they answer a request
    /// in the step that opened it, or end a PUBLISH, and neither can have the
    /// mark; a FIN on a PUBLISH ends it there and then. A new place that
    /// answers a request later than it was opened belongs here.
    fn finish_slot(&mut self, stream: StreamKey) {
        if self.peer_finished(stream) {
            self.requests.remove(&stream);
        } else {
            self.requests.insert(stream, RequestSlot::Finished(None));
        }
    }

    /// Whether the peer finished its direction of a request stream while the
    /// request goes on.
    fn peer_finished(&self, stream: StreamKey) -> bool {
        self.requests
            .get(&stream)
            .is_some_and(RequestSlot::peer_finished)
    }

    /// Lets go of the early streams that carry `track_alias`, when the PUBLISH
    /// that would have explained them was refused.
    fn drop_early_streams(
        &mut self,
        session: SessionKey,
        track_alias: u64,
        actions: &mut Vec<Action>,
    ) {
        self.early.retain(|early| {
            if early.header.track_alias != track_alias {
                return true;
            }
            if early.end.is_none() {
                actions.push(Action::StopDataStream {
                    session,
                    stream: early.stream,
                    code: stream_reset_code::CANCELLED,
                });
            }
            false
        });
    }
}

#[derive(Debug)]
struct Subscriber {
    session: SessionKey,
    /// The request stream of the SUBSCRIBE.
    stream: StreamKey,
    request: PeerRequestId,
    /// The alias the relay uses towards this subscriber (§11.1).
    track_alias: u64,
    /// The Forward State (§5.1): Objects are sent only when true.
    forward: bool,
    /// Subgroup streams the relay started for the subscription.
    streams_started: u64,
    /// Whether one of them may never have been opened, so that the number
    /// above is not known to be the number of streams the subscriber sees.
    count_unknown: bool,
    /// The streams of the subscription that are not done yet, in the order
    /// they were started.
    outs: BTreeSet<OutStreamKey>,
}

/// A subgroup stream the relay writes to a subscriber, until the I/O side
/// reports it done.
#[derive(Debug)]
struct OutStream {
    track: FullTrackName,
    session: SessionKey,
    /// Whether the I/O side has reported it opened.
    opened: bool,
}

/// A subgroup stream of the publisher that is being forwarded.
#[derive(Debug)]
struct InStream {
    header: SubgroupHeader,
    /// The Subgroup ID, once known. With the mode "first Object ID" (§11.4.2)
    /// that is when the first Object arrives.
    subgroup_id: Option<u64>,
    /// Whether an Object was forwarded. A subscriber that joins after that
    /// gets a stream that does not start with the Subgroup's first Object.
    started: bool,
    /// The stream each subscriber gets this one on, by session.
    outs: HashMap<SessionKey, OutStreamKey>,
}

/// Where the Objects of a track come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackSource {
    /// A PUBLISH that a peer sent and the relay accepted.
    Publisher {
        session: SessionKey,
        /// The request stream of the PUBLISH.
        stream: StreamKey,
        /// The alias the publisher chose (§11.1).
        track_alias: u64,
    },
    /// A SUBSCRIBE the relay sent to a publisher of a matching namespace, and
    /// the publisher accepted (§9.5).
    Subscription {
        session: SessionKey,
        /// The relay's request.
        request: OwnRequestKey,
        /// The alias the publisher chose in `SUBSCRIBE_OK` (§11.1).
        track_alias: u64,
    },
}

impl TrackSource {
    /// Whether this is the PUBLISH on that request stream of that session.
    fn is_publish(self, session: SessionKey, stream: StreamKey) -> bool {
        matches!(
            self,
            Self::Publisher {
                session: source,
                stream: publish_stream,
                ..
            } if source == session && publish_stream == stream
        )
    }

    /// Whether this is that request of the relay on that session.
    fn is_subscription(self, session: SessionKey, request: OwnRequestKey) -> bool {
        matches!(
            self,
            Self::Subscription {
                session: source,
                request: own,
                ..
            } if source == session && own == request
        )
    }

    /// The session the Objects arrive on, and the alias they carry there.
    fn session_and_alias(self) -> (SessionKey, u64) {
        match self {
            Self::Publisher {
                session,
                track_alias,
                ..
            }
            | Self::Subscription {
                session,
                track_alias,
                ..
            } => (session, track_alias),
        }
    }
}

/// One Established source of a track, and how far it has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Source {
    from: TrackSource,
    /// The source said `PUBLISH_DONE`; it is over when its last streams are.
    draining: bool,
}

impl Source {
    fn is_publish(self, session: SessionKey, stream: StreamKey) -> bool {
        self.from.is_publish(session, stream)
    }

    fn is_subscription(self, session: SessionKey, request: OwnRequestKey) -> bool {
        self.from.is_subscription(session, request)
    }

    fn session_and_alias(self) -> (SessionKey, u64) {
        self.from.session_and_alias()
    }
}

/// A track with a source: an Established upstream subscription (§9.4), or
/// one whose last source drains.
#[derive(Debug)]
struct Track {
    /// FIXME: several of them; see the top of `routing.rs`. What is asked of
    /// "the sources" below is written so that it holds for several.
    source: Source,
    /// What is still asked of other publishers. Empty today: one publisher
    /// is asked, and its answer is what gave the track its source.
    asks: Vec<Ask>,
    /// Track Properties of the source; §9.6 has the relay pass them on.
    properties: Vec<KeyValuePair>,
    /// The largest Location known for the track (§10.2.11): what the source
    /// said, or a larger one seen since.
    largest: Option<Location>,
    subscribers: Vec<Subscriber>,
    /// By the session and the stream they arrive on: stream numbers repeat
    /// from session to session.
    in_streams: HashMap<(SessionKey, StreamKey), InStream>,
}

impl Track {
    /// The one place a track with a source is made.
    fn established(
        from: TrackSource,
        properties: Vec<KeyValuePair>,
        largest: Option<Location>,
    ) -> Self {
        Self {
            source: Source {
                from,
                draining: false,
            },
            asks: Vec::new(),
            properties,
            largest,
            subscribers: Vec::new(),
            in_streams: HashMap::new(),
        }
    }

    fn sources(&self) -> impl Iterator<Item = &Source> {
        std::iter::once(&self.source)
    }

    fn sources_mut(&mut self) -> impl Iterator<Item = &mut Source> {
        std::iter::once(&mut self.source)
    }

    /// WHY every source: a track goes on for as long as one of its sources
    /// does, and ends when the last of them has drained. A track never has
    /// no source; one that did would not be draining but gone.
    fn is_draining(&self) -> bool {
        let mut sources = self.sources().peekable();
        sources.peek().is_some() && sources.all(|source| source.draining)
    }

    /// Whether the source that is this request of the relay on that session
    /// said `PUBLISH_DONE`; `None` when the track has no such source.
    fn subscription_is_draining(
        &self,
        session: SessionKey,
        request: OwnRequestKey,
    ) -> Option<bool> {
        self.sources()
            .find(|source| source.is_subscription(session, request))
            .map(|source| source.draining)
    }
}

/// Where a track the relay knows is: the three states a SUBSCRIBE for it can
/// meet. A fourth case is a track the relay does not know, which is no
/// `TrackPhase` at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackPhase {
    /// It has no source, and publishers were asked for it; subscribers wait.
    Pending,
    /// It has an Established upstream subscription (§9.4).
    Established,
    /// Its sources said `PUBLISH_DONE` and their last streams are awaited.
    Draining,
}

/// How a track is kept: without a source and with one. What is asked of
/// publishers ([`Ask`]) is kept in both.
#[derive(Debug)]
enum TrackEntry {
    Pending(PendingTrack),
    Live(Track),
}

/// Every track the relay knows, by name.
///
/// WHY one map for the tracks that wait for a publisher's answer and the ones
/// that have a source: a name is in one of the states or in none, and two
/// maps would leave that to every place that writes to them.
#[derive(Debug, Default)]
struct Tracks(HashMap<FullTrackName, TrackEntry>);

impl Tracks {
    fn phase(&self, name: &FullTrackName) -> Option<TrackPhase> {
        Some(match self.0.get(name)? {
            TrackEntry::Pending(_) => TrackPhase::Pending,
            TrackEntry::Live(track) if track.is_draining() => TrackPhase::Draining,
            TrackEntry::Live(_) => TrackPhase::Established,
        })
    }

    fn live(&self, name: &FullTrackName) -> Option<&Track> {
        match self.0.get(name)? {
            TrackEntry::Live(track) => Some(track),
            TrackEntry::Pending(_) => None,
        }
    }

    fn live_mut(&mut self, name: &FullTrackName) -> Option<&mut Track> {
        match self.0.get_mut(name)? {
            TrackEntry::Live(track) => Some(track),
            TrackEntry::Pending(_) => None,
        }
    }

    fn pending(&self, name: &FullTrackName) -> Option<&PendingTrack> {
        match self.0.get(name)? {
            TrackEntry::Pending(pending) => Some(pending),
            TrackEntry::Live(_) => None,
        }
    }

    fn pending_mut(&mut self, name: &FullTrackName) -> Option<&mut PendingTrack> {
        match self.0.get_mut(name)? {
            TrackEntry::Pending(pending) => Some(pending),
            TrackEntry::Live(_) => None,
        }
    }

    /// What is asked of publishers for the track, with or without a source.
    fn asks(&self, name: &FullTrackName) -> &[Ask] {
        match self.0.get(name) {
            Some(TrackEntry::Pending(pending)) => &pending.asks,
            Some(TrackEntry::Live(track)) => &track.asks,
            None => &[],
        }
    }

    fn asks_mut(&mut self, name: &FullTrackName) -> Option<&mut Vec<Ask>> {
        Some(match self.0.get_mut(name)? {
            TrackEntry::Pending(pending) => &mut pending.asks,
            TrackEntry::Live(track) => &mut track.asks,
        })
    }

    fn all_asks(&self) -> impl Iterator<Item = (&FullTrackName, &Ask)> {
        self.0.iter().flat_map(|(name, entry)| {
            let asks = match entry {
                TrackEntry::Pending(pending) => &pending.asks,
                TrackEntry::Live(track) => &track.asks,
            };
            asks.iter().map(move |ask| (name, ask))
        })
    }

    /// Puts a track with a source under a name that has no entry.
    ///
    /// WHY "no entry": a pending track of the name holds subscribers that
    /// wait and requests that are open, and replacing it would drop both
    /// without a word. Whoever gives a pending track its source takes it out
    /// first ([`Self::take_pending`]) and hands on what it held.
    fn insert_live(&mut self, name: FullTrackName, track: Track) {
        let replaced = self.0.insert(name, TrackEntry::Live(track));
        debug_assert!(replaced.is_none(), "a track was replaced: {replaced:?}");
    }

    /// Puts a track without a source under a name that has no entry.
    fn insert_pending(&mut self, name: FullTrackName, pending: PendingTrack) {
        let replaced = self.0.insert(name, TrackEntry::Pending(pending));
        debug_assert!(replaced.is_none(), "a track was replaced: {replaced:?}");
    }

    /// Removes the track if it has a source.
    fn take_live(&mut self, name: &FullTrackName) -> Option<Track> {
        self.live(name)?;
        match self.0.remove(name)? {
            TrackEntry::Live(track) => Some(track),
            TrackEntry::Pending(_) => None,
        }
    }

    /// Removes the track if it waits for a publisher's answer.
    fn take_pending(&mut self, name: &FullTrackName) -> Option<PendingTrack> {
        self.pending(name)?;
        match self.0.remove(name)? {
            TrackEntry::Pending(pending) => Some(pending),
            TrackEntry::Live(_) => None,
        }
    }

    fn all_live(&self) -> impl Iterator<Item = (&FullTrackName, &Track)> {
        self.0.iter().filter_map(|(name, entry)| match entry {
            TrackEntry::Live(track) => Some((name, track)),
            TrackEntry::Pending(_) => None,
        })
    }

    fn all_pending(&self) -> impl Iterator<Item = (&FullTrackName, &PendingTrack)> {
        self.0.iter().filter_map(|(name, entry)| match entry {
            TrackEntry::Pending(pending) => Some((name, pending)),
            TrackEntry::Live(_) => None,
        })
    }
}

/// What a relay holds, for tests and logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Sessions.
    pub sessions: usize,
    /// Tracks with a source.
    pub tracks: usize,
    /// Subscriptions of peers to those tracks.
    pub subscriptions: usize,
    /// Requests of peers the sessions hold state for.
    pub requests: usize,
    /// Request streams of peers the relay keeps an entry for: the ones above,
    /// and the ones it has answered and that are not reported closed.
    pub request_streams: usize,
    /// Of those, the ones whose peer finished its direction while the
    /// request goes on.
    pub half_closed_streams: usize,
    /// Subgroup streams of sources that are being forwarded.
    pub in_streams: usize,
    /// Subgroup streams to subscribers that are not done.
    pub out_streams: usize,
    /// Subgroup streams held for a Track Alias that is not known yet.
    pub early_streams: usize,
    /// Namespaces announced with `PUBLISH_NAMESPACE` and not withdrawn.
    pub namespaces: usize,
    /// Tracks the relay asked a publisher for and has no answer about.
    pub pending_tracks: usize,
    /// Requests the relay opened and has not let go of, as the relay counts
    /// them. The I/O side has a task for each that is not answered or is
    /// Established; a test that compares the two numbers then sees a request
    /// one side forgot. A request whose publisher announced a PUBLISH with
    /// `DUPLICATE_SUBSCRIPTION` is counted here until that wait is over, and
    /// has no task any more.
    pub own_requests: usize,
}

/// All sessions and tracks of one relay.
#[derive(Debug)]
pub struct Relay {
    implementation: String,
    sessions: HashMap<SessionKey, SessionState>,
    tracks: Tracks,
    out_streams: HashMap<OutStreamKey, OutStream>,
    next_out_stream: u64,
    /// In the order they were announced.
    namespaces: Vec<Announcement>,
    next_own_request: u64,
}

impl Relay {
    /// A relay without sessions. `implementation` is sent as the
    /// `MOQT_IMPLEMENTATION` Setup Option (§10.3.1.5).
    #[must_use]
    pub fn new(implementation: &str) -> Self {
        Self {
            implementation: implementation.to_string(),
            sessions: HashMap::new(),
            tracks: Tracks::default(),
            out_streams: HashMap::new(),
            next_out_stream: 0,
            namespaces: Vec::new(),
            next_own_request: 0,
        }
    }

    /// Takes one event and returns what to do about it, in order.
    ///
    /// An event for a session or stream the relay no longer knows is ignored:
    /// the I/O side reports what it read, and some of that is read after the
    /// relay let go.
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        let mut actions = Vec::new();
        match event {
            Event::SessionOpened { session, role } => {
                self.open_session(session, role, &mut actions);
            }
            Event::ControlMessage { session, message } => {
                let Some(state) = self.sessions.get_mut(&session) else {
                    return actions;
                };
                if let Err(error) = state.moqt.on_control_message(message) {
                    self.fail_session(session, &error, &mut actions);
                }
            }
            Event::RequestOpened {
                session,
                stream,
                first,
            } => self.on_request_opened(session, stream, first, &mut actions),
            Event::RequestMessage {
                session,
                stream,
                message,
            } => self.on_request_message(session, stream, message, &mut actions),
            Event::RequestClosed {
                session,
                stream,
                how,
            } => match how {
                RequestEnd::Fin => self.on_request_half_closed(session, stream, &mut actions),
                RequestEnd::Aborted => self.on_request_closed(session, stream, true, &mut actions),
            },
            Event::OwnRequestMessage {
                session,
                request,
                message,
            } => self.on_own_request_message(session, request, message, &mut actions),
            Event::OwnRequestClosed {
                session,
                request,
                how,
            } => self.on_own_request_closed(session, request, how, &mut actions),
            Event::UpstreamTimeout { session, request } => {
                self.on_upstream_timeout(session, request, &mut actions);
            }
            Event::DataStreamOpened {
                session,
                stream,
                header,
            } => self.on_data_stream_opened(session, stream, header, &mut actions),
            Event::Object {
                session,
                stream,
                object,
            } => self.on_object(session, stream, object, &mut actions),
            Event::DataStreamClosed {
                session,
                stream,
                end,
            } => self.on_data_stream_closed(session, stream, end, &mut actions),
            Event::TrackMalformed { session, stream } => {
                let track = self
                    .sessions
                    .get(&session)
                    .and_then(|state| state.in_streams.get(&stream))
                    .cloned();
                if let Some(track) = track {
                    self.end_track(&track, TrackEnd::Malformed, &mut actions);
                }
            }
            Event::OutStreamOpened { out_stream } => {
                if let Some(out) = self.out_streams.get_mut(&out_stream) {
                    out.opened = true;
                }
            }
            Event::OutStreamClosed { out_stream } => self.forget_out_stream(out_stream),
            Event::SendQueueFull { out_stream } => {
                self.on_send_queue_full(out_stream, &mut actions);
            }
            Event::DrainTimeout {
                session,
                track_alias,
            } => self.on_drain_timeout(session, track_alias, &mut actions),
            Event::EarlyStreamTimeout { session, stream } => {
                self.on_early_stream_timeout(session, stream, &mut actions);
            }
            Event::RequestTimeout { session, stream } => {
                self.on_request_timeout(session, stream, &mut actions);
            }
            Event::SessionFailed { session, error } => {
                self.fail_session(session, &error, &mut actions);
            }
            Event::SessionClosed { session } => self.drop_session(session, &mut actions),
        }
        actions
    }

    fn on_send_queue_full(&mut self, out_stream: OutStreamKey, actions: &mut Vec<Action>) {
        let Some(out) = self.out_streams.get(&out_stream) else {
            return;
        };
        let (track, subscriber) = (out.track.clone(), out.session);
        // WHY end the one subscription: its queue is the only place the delay
        // can go without holding back the publisher and the other subscribers
        // (§8 names TOO_FAR_BEHIND for it).
        self.end_subscription(
            &track,
            subscriber,
            publish_done_status::TOO_FAR_BEHIND,
            stream_reset_code::TOO_FAR_BEHIND,
            actions,
        );
    }

    fn on_drain_timeout(
        &mut self,
        session: SessionKey,
        track_alias: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let track = match state.moqt.subscribed_alias(track_alias) {
            SubscribedAlias::Draining(track) => track.clone(),
            _ => return,
        };
        state.moqt.on_drain_timeout(track_alias);
        self.end_track(&track, TrackEnd::TimedOut, actions);
    }

    /// Lets go of a request the relay ended with `PUBLISH_DONE` and whose
    /// stream the peer has not closed in time.
    fn on_request_timeout(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(RequestSlot::Finished(Some(request))) = state.requests.get(&stream) else {
            return;
        };
        state.moqt.on_peer_request_closed(*request);
        // The slot stays until the stream really closes, so that what the peer
        // still sends on it is ignored.
        state.requests.insert(stream, RequestSlot::Finished(None));
        // The relay's direction was finished with the PUBLISH_DONE; this ends
        // the peer's.
        actions.push(Action::StopRequest {
            session,
            stream,
            code: stream_reset_code::CANCELLED,
        });
    }

    /// Lets go of a stream whose Track Alias did not become known in time.
    fn on_early_stream_timeout(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let held = state.early.iter().position(|early| early.stream == stream);
        if let Some(early) = held.and_then(|position| state.early.remove(position))
            && early.end.is_none()
        {
            actions.push(Action::StopDataStream {
                session,
                stream,
                code: stream_reset_code::CANCELLED,
            });
        }
    }

    fn open_session(&mut self, session: SessionKey, role: Role, actions: &mut Vec<Action>) {
        let mut moqt = Session::new(role, Transport::WebTransport);
        let option = KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            self.implementation.as_bytes().to_vec(),
        );
        // An implementation name too long for a Setup Option is left out; the
        // option is a SHOULD (§10.3.1.5).
        let options = option.map(|option| vec![option]).unwrap_or_default();
        match moqt.setup(options) {
            Ok(message) => actions.push(Action::SendControl { session, message }),
            Err(error) => {
                actions.push(internal_error(session, &error));
                return;
            }
        }
        self.sessions.insert(
            session,
            SessionState {
                moqt,
                requests: HashMap::new(),
                in_streams: HashMap::new(),
                early: VecDeque::new(),
                own: HashMap::new(),
            },
        );
    }

    fn on_request_opened(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        first: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // A PUBLISH the session refuses by itself is not handed back, so the
        // alias its early streams wait for is taken down here.
        let offered_alias = match &first {
            ControlMessage::Publish(publish) => Some(publish.track_alias),
            _ => None,
        };
        let (request, peer_request) = match state.moqt.on_peer_request_opened(first) {
            Ok(PeerRequestOpened::Request(request, peer_request)) => (request, peer_request),
            Ok(PeerRequestOpened::Refused { response }) => {
                state.requests.insert(stream, RequestSlot::Finished(None));
                if let Some(track_alias) = offered_alias {
                    state.drop_early_streams(session, track_alias, actions);
                }
                actions.push(Action::SendRequest {
                    session,
                    stream,
                    message: response,
                });
                actions.push(Action::FinishRequest { session, stream });
                return;
            }
            Err(error) => {
                self.fail_session(session, &error, actions);
                return;
            }
        };
        match peer_request {
            PeerRequest::Subscribe(subscribe) => {
                self.on_subscribe(session, stream, request, &subscribe, actions);
            }
            PeerRequest::Publish(publish) => {
                self.on_publish(session, stream, request, publish, actions);
            }
            PeerRequest::PublishNamespace(publish_namespace) => {
                if !self.add_namespace(session, stream, publish_namespace.track_namespace) {
                    let too_many = Refusal::code(request_error_code::EXCESSIVE_LOAD);
                    self.refuse(session, stream, request, &too_many, actions);
                    return;
                }
                let Some(state) = self.sessions.get_mut(&session) else {
                    return;
                };
                let response = state.moqt.accept_publish_namespace(request);
                state.requests.insert(
                    stream,
                    RequestSlot::PublishNamespace {
                        request,
                        peer_finished: false,
                    },
                );
                self.respond(session, stream, response, actions);
            }
        }
    }

    fn on_subscribe(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        subscribe: &Subscribe,
        actions: &mut Vec<Action>,
    ) {
        let track_name = &subscribe.full_track_name;
        let forward = params::forward(&subscribe.parameters);
        let waiter = Waiter::new(session, stream, request, forward);
        let waits = match self.tracks.phase(track_name) {
            // §9.5: "When a Relay receives an authorized SUBSCRIBE for a Track
            // with one or more Established upstream subscriptions, it MUST
            // reply with SUBSCRIBE_OK."
            Some(TrackPhase::Established) => {
                self.establish(track_name, &waiter, actions);
                return;
            }
            // The source said PUBLISH_DONE and the track still drains: it has
            // no Established upstream subscription (§9.4), and the publisher
            // cannot be asked again while the old alias drains.
            Some(TrackPhase::Draining) => Err(request_error_code::DOES_NOT_EXIST),
            Some(TrackPhase::Pending) => self.wait_with_the_others(track_name, waiter),
            // §9.5: "If there are no Established upstream subscriptions for
            // the requested Track, the Relay MUST send a SUBSCRIBE request to
            // each publisher that has published the subscription's namespace
            // or prefix thereof." The first of them by the order of
            // announcing; see the FIXME at the top of `routing.rs` for the
            // others.
            None => {
                let first = self.matching_publishers(track_name).next();
                match first {
                    Some(publisher) => self.ask(publisher, track_name, waiter, actions),
                    // §10.2.6: without RENDEZVOUS_TIMEOUT, or with 0, "MUST
                    // immediately return REQUEST_ERROR with error code
                    // DOES_NOT_EXIST if no publisher is available".
                    None => Err(request_error_code::DOES_NOT_EXIST),
                }
            }
        };
        match waits {
            Ok(()) => {
                if let Some(state) = self.sessions.get_mut(&session) {
                    state.requests.insert(
                        stream,
                        RequestSlot::Waiting {
                            request,
                            track: track_name.clone(),
                            peer_finished: false,
                        },
                    );
                }
            }
            Err(error_code) => {
                self.refuse(
                    session,
                    stream,
                    request,
                    &Refusal::code(error_code),
                    actions,
                );
            }
        }
    }

    fn on_publish(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        publish: Publish,
        actions: &mut Vec<Action>,
    ) {
        // §5.1: "If a subscriber receives a PUBLISH for a Track with a
        // subscription in the Pending (Subscriber) state, it MUST ensure the
        // subscription it initiated transitions to the Terminated state
        // before sending PUBLISH_OK." The relay's SUBSCRIBE to this session
        // gives way, and the PUBLISH becomes the source of the track.
        //
        // WHY only for the session that was asked: a subscription is between
        // two endpoints, so the sentence is about the PUBLISH of the session
        // the SUBSCRIBE went to. With another session the relay has no pending
        // subscription, and its PUBLISH is a second publisher of the track.
        let asked_here = self.is_asked_of(&publish.full_track_name, session);
        // A track the relay has asked another publisher for counts as taken.
        let taken = match self.tracks.phase(&publish.full_track_name) {
            Some(TrackPhase::Established | TrackPhase::Draining) => true,
            Some(TrackPhase::Pending) => !asked_here,
            None => false,
        };
        if taken {
            if let Some(state) = self.sessions.get_mut(&session) {
                state.drop_early_streams(session, publish.track_alias, actions);
            }
            // WHY UNINTERESTED: it is the code §10.6.2 gives a subscriber for
            // declining a PUBLISH, and the relay is the subscriber here. §9.3
            // lets a relay refuse a publisher it cannot take. The PUBLISH
            // would be accepted once the other publisher is gone.
            let uninterested = Refusal::code(request_error_code::UNINTERESTED);
            self.refuse(session, stream, request, &uninterested, actions);
            return;
        }
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // WHY Forward State 1 from the start: §9.5 allows 0 until a subscriber
        // comes, which would need a REQUEST_UPDATE towards the publisher later.
        // Objects that arrive while nobody subscribes are dropped instead.
        let response = state
            .moqt
            .accept_publish(request, vec![Parameter::Forward(true)])
            .map(|(ok, _)| ok);
        state.requests.insert(
            stream,
            RequestSlot::Publish {
                request,
                track: publish.full_track_name.clone(),
            },
        );
        // §10.2.11: the relay's LARGEST_OBJECT starts from the one the
        // publisher gave.
        let largest = publish
            .parameters
            .iter()
            .find_map(|parameter| match parameter {
                Parameter::LargestObject(location) => Some(*location),
                _ => None,
            });
        let track_name = publish.full_track_name;
        let published = TrackSource::Publisher {
            session,
            stream,
            track_alias: publish.track_alias,
        };
        let mut track = Track::established(published, publish.track_properties, largest);
        // Before the PUBLISH_OK, as §5.1 has it. And before the track gets
        // its entry: the pending track of the name is taken out with its
        // waiters, and what it had asked of other publishers goes on under
        // the new one.
        let waiters = if asked_here {
            let (waiters, still_asked) = self.give_way_to_publish(&track_name, session, actions);
            track.asks = still_asked;
            waiters
        } else {
            Vec::new()
        };
        self.tracks.insert_live(track_name.clone(), track);
        self.respond(session, stream, response, actions);
        // §9.4: the track has an Established upstream subscription now.
        for waiter in &waiters {
            self.establish(&track_name, waiter, actions);
        }
        self.replay_early_streams(session, publish.track_alias, actions);
    }

    /// Feeds the streams that were waiting for this alias back in.
    fn replay_early_streams(
        &mut self,
        session: SessionKey,
        track_alias: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let (ready, waiting): (VecDeque<_>, VecDeque<_>) = std::mem::take(&mut state.early)
            .into_iter()
            .partition(|early| early.header.track_alias == track_alias);
        state.early = waiting;
        for early in ready {
            self.on_data_stream_opened(session, early.stream, early.header, actions);
            for object in early.objects {
                self.on_object(session, early.stream, object, actions);
            }
            if let Some(end) = early.end {
                self.on_data_stream_closed(session, early.stream, end, actions);
            }
        }
    }

    fn on_request_message(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let (request, track) = match state.requests.get(&stream) {
            Some(
                RequestSlot::Subscribe { request, track, .. }
                | RequestSlot::Publish { request, track }
                | RequestSlot::Waiting { request, track, .. },
            ) => (*request, Some(track.clone())),
            Some(
                RequestSlot::PublishNamespace { request, .. }
                | RequestSlot::Finished(Some(request)),
            ) => (*request, None),
            Some(RequestSlot::Finished(None)) | None => return,
        };
        let event = match state.moqt.on_peer_request_message(request, message) {
            Ok(event) => event,
            Err(RequestFault::Peer(error)) => {
                self.fail_session(session, &error, actions);
                return;
            }
            Err(RequestFault::Usage(error)) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
                return;
            }
        };
        match event {
            PeerRequestEvent::PublishDone { track_alias, .. } => {
                // The session has let go of the request already.
                state.requests.insert(stream, RequestSlot::Finished(None));
                let Some(track_name) = track else {
                    return;
                };
                self.source_done(
                    &track_name,
                    |source| source.is_publish(session, stream),
                    session,
                    track_alias,
                    actions,
                );
            }
            PeerRequestEvent::UpdateRefused { response, then } => {
                actions.push(Action::SendRequest {
                    session,
                    stream,
                    message: response,
                });
                match (then, track) {
                    (AfterRefusal::Nothing, _) | (AfterRefusal::PublishDone, None) => {}
                    // §10.9.1: a refused update ends the subscription.
                    (AfterRefusal::PublishDone, Some(track)) => self.end_subscription(
                        &track,
                        session,
                        publish_done_status::UPDATE_FAILED,
                        stream_reset_code::CANCELLED,
                        actions,
                    ),
                    (AfterRefusal::CloseStream, _) => {
                        actions.push(Action::FinishRequest { session, stream });
                        // The session has ended the request; what the request
                        // held in the relay goes the way of a closed stream.
                        // The slot stays until the stream really closes, so
                        // that what the peer still sends on it is ignored.
                        self.on_request_closed(session, stream, false, actions);
                        if let Some(state) = self.sessions.get_mut(&session) {
                            state.requests.insert(stream, RequestSlot::Finished(None));
                        }
                    }
                }
            }
            PeerRequestEvent::UpdateAlreadyRefused | PeerRequestEvent::LateMessage => {}
        }
    }

    /// A source of a track said `PUBLISH_DONE`, on the request stream of its
    /// PUBLISH or of the relay's SUBSCRIBE: it drains from here on, and the
    /// track ends once nothing more can arrive for it (§10.11). `said_by`
    /// picks the source; `session` and `track_alias` are where its streams
    /// arrive.
    fn source_done(
        &mut self,
        track_name: &FullTrackName,
        said_by: impl Fn(&Source) -> bool,
        session: SessionKey,
        track_alias: u64,
        actions: &mut Vec<Action>,
    ) {
        if let Some(track) = self.tracks.live_mut(track_name) {
            for source in track.sources_mut().filter(|source| said_by(source)) {
                source.draining = true;
            }
        }
        if !self.end_track_if_drained(track_name, actions) {
            actions.push(Action::StartDrainTimer {
                session,
                track_alias,
            });
        }
    }

    /// Lets go of what a request stream stood for. With `finish`, the relay's
    /// direction of the stream is finished if the relay has not ended it yet:
    /// the peer ended its own, and nothing more will be said.
    ///
    /// The exception is a PUBLISH whose publisher said `PUBLISH_DONE`: the
    /// relay's direction stays open until the track's last streams arrived,
    /// and how it ends then tells the publisher whether they did
    /// ([`Self::release_source`]).
    fn on_request_closed(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        finish: bool,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let slot = state.requests.remove(&stream);
        if finish
            && matches!(
                slot,
                Some(
                    RequestSlot::Subscribe { .. }
                        | RequestSlot::Publish { .. }
                        | RequestSlot::PublishNamespace { .. }
                        | RequestSlot::Waiting { .. }
                )
            )
        {
            actions.push(Action::FinishRequest { session, stream });
        }
        match slot {
            Some(RequestSlot::Subscribe { request, track, .. }) => {
                state.moqt.on_peer_request_closed(request);
                // The subscriber cancelled (§3.3.2, §5.1.1 "MUST reset any open
                // streams associated with the SUBSCRIBE"). No PUBLISH_DONE can
                // follow on a stream that is gone.
                self.remove_subscriber(&track, session, stream_reset_code::CANCELLED, actions);
            }
            Some(RequestSlot::Publish { request, track }) => {
                state.moqt.on_peer_request_closed(request);
                let cancelled = self.tracks.live(&track).is_some_and(|track| {
                    track
                        .sources()
                        .any(|source| source.is_publish(session, stream) && !source.draining)
                });
                if cancelled {
                    self.end_track(&track, TrackEnd::SourceGone, actions);
                }
            }
            Some(RequestSlot::Waiting { request, track, .. }) => {
                state.moqt.on_peer_request_closed(request);
                self.waiters_left(&track, |waiter| waiter.is_on(session, stream), actions);
            }
            Some(RequestSlot::PublishNamespace { request, .. }) => {
                state.moqt.on_peer_request_closed(request);
                // §9.5: cancelling the request withdraws the namespace.
                self.withdraw_namespace(session, stream);
            }
            Some(RequestSlot::Finished(Some(request))) => {
                state.moqt.on_peer_request_closed(request);
            }
            Some(RequestSlot::Finished(None)) | None => {}
        }
    }

    /// The peer finished its direction of a request stream and did nothing
    /// else.
    ///
    /// WHY that ends a PUBLISH and not the others: §5.1 has a subscriber end
    /// a subscription with `STOP_SENDING`, and §6.2 has a `PUBLISH_NAMESPACE`
    /// withdrawn by cancelling the request, which §3.3.2 does by a reset or
    /// `STOP_SENDING`. A requester that only finishes its direction can say
    /// nothing more (no `REQUEST_UPDATE`), and has asked for nothing to end.
    /// A publisher ends its PUBLISH with `PUBLISH_DONE` on that direction
    /// (§5.1); once the direction is finished without one, none can come, so
    /// the track is over. §6.1 says of `SUBSCRIBE_NAMESPACE` and
    /// `SUBSCRIBE_TRACKS` alone that a FIN cancels them.
    fn on_request_half_closed(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let goes_on = state
            .requests
            .get_mut(&stream)
            .is_some_and(RequestSlot::note_peer_finished);
        if !goes_on {
            self.on_request_closed(session, stream, true, actions);
        }
    }

    fn on_data_stream_opened(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        header: SubgroupHeader,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let track_name = match state.moqt.subscribed_alias(header.track_alias) {
            // §10.11: Objects of a track that PUBLISH_DONE ended still belong
            // to it while the alias drains.
            SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => track.clone(),
            // §11.1: late streams of an ended subscription are unwanted.
            SubscribedAlias::Retired => {
                actions.push(Action::StopDataStream {
                    session,
                    stream,
                    code: stream_reset_code::CANCELLED,
                });
                return;
            }
            SubscribedAlias::Unknown => {
                if state.early.len() >= MAX_EARLY_STREAMS {
                    actions.push(Action::StopDataStream {
                        session,
                        stream,
                        code: stream_reset_code::INTERNAL_ERROR,
                    });
                } else {
                    state.early.push_back(EarlyStream {
                        stream,
                        header,
                        objects: Vec::new(),
                        end: None,
                    });
                    actions.push(Action::StartEarlyTimer { session, stream });
                }
                return;
            }
        };
        let Some(track) = self.tracks.live_mut(&track_name) else {
            // The alias still drains and the track is gone, as after a
            // malformed track: nothing of it is wanted.
            actions.push(Action::StopDataStream {
                session,
                stream,
                code: stream_reset_code::CANCELLED,
            });
            return;
        };
        state.in_streams.insert(stream, track_name.clone());
        let subgroup_id = match header.subgroup_id {
            SubgroupId::Zero => Some(0),
            SubgroupId::Explicit(id) => Some(id),
            SubgroupId::FirstObjectId => None,
        };
        let mut in_stream = InStream {
            header,
            subgroup_id,
            started: false,
            outs: HashMap::new(),
        };
        // Every subscriber gets a stream now, also for a Subgroup that turns
        // out to hold no Object: the header alone can say something, such as
        // END_OF_GROUP (§11.4.2).
        for subscriber in &mut track.subscribers {
            let out = OutStreamAllocator {
                next: &mut self.next_out_stream,
                out_streams: &mut self.out_streams,
            };
            out.open(&track_name, subscriber, &mut in_stream, header, actions);
        }
        track.in_streams.insert((session, stream), in_stream);
    }

    fn on_object(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        object: SubgroupObject,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(track_name) = state.in_streams.get(&stream) else {
            hold_early_object(state, session, stream, object, actions);
            return;
        };
        let Some(track) = self.tracks.live_mut(track_name) else {
            return;
        };
        let Some(in_stream) = track.in_streams.get_mut(&(session, stream)) else {
            return;
        };
        let location = Location {
            group: in_stream.header.group_id,
            object: object.object_id,
        };
        if track.largest.is_none_or(|largest| location > largest) {
            track.largest = Some(location);
        }
        // §11.4.2, mode 0b01: the Subgroup ID is the Object ID of the first
        // Object of the Subgroup.
        let subgroup_id = *in_stream.subgroup_id.get_or_insert(object.object_id);

        // A subscriber that came after the stream started gets a stream of its
        // own from here on. Its header must not repeat two things that are
        // only true of the Subgroup's first Object: the mode that derives the
        // Subgroup ID from it, and FIRST_OBJECT (§11.4.2).
        let late_header = SubgroupHeader {
            subgroup_id: SubgroupId::Explicit(subgroup_id),
            first_object: false,
            ..in_stream.header
        };
        let header = if in_stream.started {
            late_header
        } else {
            in_stream.header
        };
        for subscriber in &mut track.subscribers {
            if !in_stream.outs.contains_key(&subscriber.session) {
                let out = OutStreamAllocator {
                    next: &mut self.next_out_stream,
                    out_streams: &mut self.out_streams,
                };
                out.open(track_name, subscriber, in_stream, header, actions);
            }
        }
        in_stream.started = true;

        // §9.7: the Object goes out as it came in. §9.4: in the order it came.
        // In the order the subscribers came, so that the same events always
        // lead to the same actions.
        let object = Arc::new(object);
        for subscriber in &track.subscribers {
            if let Some(out_stream) = in_stream.outs.get(&subscriber.session) {
                actions.push(Action::SendObject {
                    out_stream: *out_stream,
                    object: Arc::clone(&object),
                });
            }
        }
    }

    fn on_data_stream_closed(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        end: StreamEnd,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(track_name) = state.in_streams.remove(&stream) else {
            if let Some(early) = state.early.iter_mut().find(|early| early.stream == stream) {
                early.end = Some(end);
            }
            return;
        };
        let Some(track) = self.tracks.live_mut(&track_name) else {
            return;
        };
        let (_, track_alias) = track.source.session_and_alias();
        state.moqt.on_data_stream_closed(track_alias);
        if let Some(in_stream) = track.in_streams.remove(&(session, stream)) {
            for subscriber in &mut track.subscribers {
                let Some(out_stream) = in_stream.outs.get(&subscriber.session).copied() else {
                    continue;
                };
                match end {
                    // The stream stays in the relay's books until the I/O side
                    // has written it out.
                    StreamEnd::Fin => actions.push(Action::FinishDataStream { out_stream }),
                    // §11.4.3: a stream that will not carry the rest of its
                    // Subgroup is reset. The publisher's code says why, when
                    // there is one.
                    StreamEnd::Aborted(code) => {
                        let code = code.unwrap_or(stream_reset_code::INTERNAL_ERROR);
                        reset_out_stream(
                            &mut self.out_streams,
                            Some(subscriber),
                            out_stream,
                            code,
                            actions,
                        );
                    }
                    // WHY CANCELLED: the publisher is gone, and that is the
                    // code its other open streams get when the end of its
                    // session is what the relay sees first. Which of the two
                    // comes first is a race of the I/O side, and the
                    // subscriber is to see one code for one cause.
                    StreamEnd::Lost => reset_out_stream(
                        &mut self.out_streams,
                        Some(subscriber),
                        out_stream,
                        stream_reset_code::CANCELLED,
                        actions,
                    ),
                }
            }
        }
        self.end_track_if_drained(&track_name, actions);
    }

    /// The I/O side is done with a stream: nothing more is to be said about
    /// it.
    fn forget_out_stream(&mut self, out_stream: OutStreamKey) {
        let Some(out) = self.out_streams.remove(&out_stream) else {
            return;
        };
        let Some(track) = self.tracks.live_mut(&out.track) else {
            return;
        };
        if let Some(subscriber) = track
            .subscribers
            .iter_mut()
            .find(|subscriber| subscriber.session == out.session)
        {
            subscriber.outs.remove(&out_stream);
            // Closed without ever being opened, for instance because the
            // connection would not take another stream.
            subscriber.count_unknown |= !out.opened;
        }
        for in_stream in track.in_streams.values_mut() {
            if in_stream.outs.get(&out.session) == Some(&out_stream) {
                in_stream.outs.remove(&out.session);
            }
        }
    }

    /// Ends a track whose source said `PUBLISH_DONE` once nothing more can
    /// arrive for it. Returns whether the track is gone.
    fn end_track_if_drained(
        &mut self,
        track_name: &FullTrackName,
        actions: &mut Vec<Action>,
    ) -> bool {
        let Some(track) = self.tracks.live(track_name) else {
            return true;
        };
        if !track.is_draining() {
            return false;
        }
        let (source, track_alias) = track.source.session_and_alias();
        let retired = self.sessions.get(&source).is_none_or(|state| {
            state.moqt.subscribed_alias(track_alias) == SubscribedAlias::Retired
        });
        if retired {
            self.end_track(track_name, TrackEnd::Done, actions);
        }
        retired
    }

    /// Removes a track and ends every subscription to it.
    fn end_track(&mut self, track_name: &FullTrackName, end: TrackEnd, actions: &mut Vec<Action>) {
        let Some(mut track) = self.tracks.take_live(track_name) else {
            return;
        };
        let (status_code, reset_code) = match end {
            TrackEnd::Done | TrackEnd::TimedOut | TrackEnd::SourceGone | TrackEnd::Unwanted => (
                publish_done_status::TRACK_ENDED,
                stream_reset_code::CANCELLED,
            ),
            TrackEnd::Malformed => (
                publish_done_status::MALFORMED_TRACK,
                stream_reset_code::MALFORMED_TRACK,
            ),
        };
        for source in track.sources() {
            self.release_source(&track, *source, end, reset_code, actions);
        }
        // What was still asked of other publishers is of no use to a track
        // that is over.
        for ask in &track.asks {
            self.cancel_own_request(ask.publisher, ask.request, actions);
        }

        // Towards the subscribers. §10.11: PUBLISH_DONE goes out after every
        // stream of the subscription is closed, so the resets come first.
        // Streams whose source stream ended in order are not touched, however
        // the track ends: each is a whole Subgroup that only waits to be
        // written out. For a malformed track they are reset as well. §2.4.2
        // only has the relay end the subscriptions; cutting off what is still
        // queued of a track that broke the object model is this relay's
        // choice.
        let cut_short: HashSet<OutStreamKey> = track
            .in_streams
            .values()
            .flat_map(|in_stream| in_stream.outs.values().copied())
            .collect();
        for subscriber in &mut track.subscribers {
            let own: Vec<OutStreamKey> = subscriber
                .outs
                .iter()
                .filter(|out_stream| end == TrackEnd::Malformed || cut_short.contains(out_stream))
                .copied()
                .collect();
            for out_stream in own {
                reset_out_stream(
                    &mut self.out_streams,
                    Some(subscriber),
                    out_stream,
                    reset_code,
                    actions,
                );
            }
        }
        for subscriber in &track.subscribers {
            self.send_publish_done(subscriber, status_code, actions);
        }
    }

    /// Tells the source of a track that is over that nothing of it is wanted
    /// any more. The one place where ending a track reaches into the session
    /// its Objects came from.
    fn release_source(
        &mut self,
        track: &Track,
        source: Source,
        end: TrackEnd,
        reset_code: u32,
        actions: &mut Vec<Action>,
    ) {
        let (session, _) = source.session_and_alias();
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // WHY sorted: the streams are kept in a `HashMap`, whose order differs
        // from run to run. The same events are to lead to the same actions,
        // so that a run can be compared with another.
        let streams: BTreeSet<u64> = track
            .in_streams
            .keys()
            .filter(|(from, _)| *from == session)
            .map(|(_, stream)| stream.0)
            .collect();
        for stream in streams.into_iter().map(StreamKey) {
            state.in_streams.remove(&stream);
            actions.push(Action::StopDataStream {
                session,
                stream,
                code: reset_code,
            });
        }
        match source.from {
            TrackSource::Publisher {
                stream: publish_stream,
                ..
            } => self.release_publish(session, publish_stream, end, reset_code, actions),
            TrackSource::Subscription { request, .. } => {
                self.release_subscription(session, request, end, reset_code, actions);
            }
        }
    }

    /// What the publisher of a PUBLISH is told when its track is over.
    ///
    /// WHY answer a `PUBLISH_DONE` on the request stream: the publisher may
    /// want to close its session next, and a stream that arrived and was not
    /// accepted yet can be lost with the connection. A FIN says that every
    /// stream the Stream Count named has ended here; a reset says the relay
    /// stopped waiting for them. The draft gives the subscriber's direction no
    /// meaning after `PUBLISH_DONE` (§10.11, §5.1.1), so a peer that knows
    /// nothing of this sees an ordinary end of the stream.
    fn release_publish(
        &mut self,
        session: SessionKey,
        publish_stream: StreamKey,
        end: TrackEnd,
        reset_code: u32,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        match end {
            TrackEnd::Done => actions.push(Action::FinishRequest {
                session,
                stream: publish_stream,
            }),
            TrackEnd::TimedOut => actions.push(Action::ResetRequest {
                session,
                stream: publish_stream,
                code: stream_reset_code::CANCELLED,
            }),
            TrackEnd::Malformed => {
                // §2.4.2: the receiver of a malformed track cancels the
                // subscription.
                if let Some(RequestSlot::Publish { request, .. }) =
                    state.requests.remove(&publish_stream)
                {
                    state.moqt.on_peer_request_closed(request);
                }
                state
                    .requests
                    .insert(publish_stream, RequestSlot::Finished(None));
                actions.push(Action::ResetRequest {
                    session,
                    stream: publish_stream,
                    code: reset_code,
                });
            }
            // The publisher ended the request itself, or is gone. A track
            // that came with PUBLISH is never unwanted: its publisher sends
            // unasked.
            TrackEnd::SourceGone | TrackEnd::Unwanted => {}
        }
    }

    /// What becomes of the relay's own SUBSCRIBE when its track is over.
    fn release_subscription(
        &mut self,
        session: SessionKey,
        request: OwnRequestKey,
        end: TrackEnd,
        reset_code: u32,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let own = state.own.remove(&request);
        match end {
            TrackEnd::Done => actions.push(Action::FinishOwnRequest { session, request }),
            // §3.3.2: the relay cancels its own request. For a malformed
            // track §2.4.2 asks for exactly that.
            TrackEnd::TimedOut
            | TrackEnd::Malformed
            | TrackEnd::SourceGone
            | TrackEnd::Unwanted => {
                if let Some(own) = own {
                    state.moqt.cancel(own.id);
                }
                actions.push(Action::CancelOwnRequest {
                    session,
                    request,
                    code: reset_code,
                });
            }
        }
    }

    /// Ends one subscription from the relay's side, with `PUBLISH_DONE`.
    fn end_subscription(
        &mut self,
        track_name: &FullTrackName,
        session: SessionKey,
        status_code: u64,
        reset_code: u32,
        actions: &mut Vec<Action>,
    ) {
        if let Some(subscriber) = self.remove_subscriber(track_name, session, reset_code, actions) {
            self.send_publish_done(&subscriber, status_code, actions);
        }
    }

    /// Takes a subscriber off a track and resets the streams it has that are
    /// not done, finished or not.
    fn remove_subscriber(
        &mut self,
        track_name: &FullTrackName,
        session: SessionKey,
        reset_code: u32,
        actions: &mut Vec<Action>,
    ) -> Option<Subscriber> {
        let track = self.tracks.live_mut(track_name)?;
        let position = track
            .subscribers
            .iter()
            .position(|subscriber| subscriber.session == session)?;
        let mut subscriber = track.subscribers.remove(position);
        for in_stream in track.in_streams.values_mut() {
            in_stream.outs.remove(&session);
        }
        let own: Vec<OutStreamKey> = subscriber.outs.iter().copied().collect();
        for out_stream in own {
            reset_out_stream(
                &mut self.out_streams,
                Some(&mut subscriber),
                out_stream,
                reset_code,
                actions,
            );
        }
        // WHY cancel the relay's own SUBSCRIBE when the last subscriber is
        // gone: the publisher would go on sending Objects nobody gets. A track
        // that came with PUBLISH stays; its publisher sends unasked. The
        // draft leaves the lifetime of an aggregated upstream subscription to
        // the relay (§9.4).
        let unwanted = track.subscribers.is_empty()
            && !track.is_draining()
            && matches!(track.source.from, TrackSource::Subscription { .. });
        if unwanted {
            self.end_track(track_name, TrackEnd::Unwanted, actions);
        }
        Some(subscriber)
    }

    fn send_publish_done(
        &mut self,
        subscriber: &Subscriber,
        status_code: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&subscriber.session) else {
            return;
        };
        // §10.11: the exact number of streams opened, or 2^62 - 1.
        let stream_count = if subscriber.count_unknown {
            STREAM_COUNT_UNKNOWN
        } else {
            subscriber.streams_started
        };
        let done = state.moqt.subscribe_done(
            subscriber.request,
            status_code,
            stream_count,
            ReasonPhrase::default(),
        );
        if state.peer_finished(subscriber.stream) {
            // The subscriber finished its direction earlier: nothing more
            // comes from it, and with the relay's FIN below the stream is
            // over.
            state.moqt.on_peer_request_closed(subscriber.request);
            state.requests.remove(&subscriber.stream);
            self.respond(subscriber.session, subscriber.stream, done, actions);
            actions.push(Action::FinishRequest {
                session: subscriber.session,
                stream: subscriber.stream,
            });
            return;
        }
        // The session keeps the request until the stream closes: what the
        // subscriber sent before it saw the PUBLISH_DONE still arrives.
        state.requests.insert(
            subscriber.stream,
            RequestSlot::Finished(Some(subscriber.request)),
        );
        self.respond(subscriber.session, subscriber.stream, done, actions);
        // §10.11: "SHOULD send FIN on the subscription's bidi stream
        // immediately after sending PUBLISH_DONE".
        actions.push(Action::FinishRequest {
            session: subscriber.session,
            stream: subscriber.stream,
        });
        // WHY a timer: nothing in §10.11 or §5.1.1 makes the subscriber close
        // its direction of the stream, and until it does the request counts
        // against what the peer may have open.
        actions.push(Action::StartRequestTimer {
            session: subscriber.session,
            stream: subscriber.stream,
        });
    }

    /// Queues a message the session produced, or closes the session when the
    /// session refused to produce it, which is a bug of this crate.
    fn respond(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        message: Result<ControlMessage, UsageError>,
        actions: &mut Vec<Action>,
    ) {
        match message {
            Ok(message) => actions.push(Action::SendRequest {
                session,
                stream,
                message,
            }),
            Err(error) => {
                actions.push(internal_error(session, &error));
                self.drop_session(session, actions);
            }
        }
    }

    /// Closes a session over a protocol error of its peer. §9.5: what closed
    /// the session is not passed on to other sessions; they only see their
    /// subscriptions and tracks end.
    fn fail_session(&mut self, session: SessionKey, error: &Error, actions: &mut Vec<Action>) {
        if !self.sessions.contains_key(&session) {
            return;
        }
        // Every error the session and the decoders of control streams return
        // closes the session; the ones that do not are sorted out by the I/O
        // side before they get here.
        let code = error
            .termination_code()
            .unwrap_or(TerminationCode::InternalError)
            .code();
        actions.push(Action::CloseSession {
            session,
            code,
            reason: error.to_string(),
        });
        self.drop_session(session, actions);
    }

    /// Forgets a session and everything it held in other sessions.
    fn drop_session(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        if self.sessions.remove(&session).is_none() {
            return;
        }
        let own_streams: HashSet<OutStreamKey> = self
            .out_streams
            .iter()
            .filter(|(_, out)| out.session == session)
            .map(|(out_stream, _)| *out_stream)
            .collect();
        // By track name, here and below: see `release_source` for why.
        let sourced: BTreeSet<FullTrackName> = self
            .tracks
            .all_live()
            .filter(|(_, track)| {
                track
                    .sources()
                    .any(|source| source.session_and_alias().0 == session)
            })
            .map(|(name, _)| name.clone())
            .collect();
        for track in sourced {
            self.end_track(&track, TrackEnd::SourceGone, actions);
        }
        self.drop_routing_of(session, actions);
        let subscribed: BTreeSet<FullTrackName> = self
            .tracks
            .all_live()
            .filter(|(_, track)| track.subscribers.iter().any(|s| s.session == session))
            .map(|(name, _)| name.clone())
            .collect();
        for track in subscribed {
            self.remove_subscriber(&track, session, stream_reset_code::SESSION_CLOSED, actions);
        }
        // Nothing can be done on streams of a session that is gone; the I/O
        // side lets go of them with the session.
        actions.retain(|action| match action.target() {
            Target::Session(target) => target != session,
            Target::OutStream(out_stream) => !own_streams.contains(&out_stream),
            Target::Closing => true,
        });
        self.out_streams.retain(|_, out| out.session != session);
    }

    /// What the relay holds.
    #[must_use]
    pub fn counts(&self) -> Counts {
        Counts {
            sessions: self.sessions.len(),
            tracks: self.tracks.all_live().count(),
            subscriptions: self
                .tracks
                .all_live()
                .map(|(_, track)| track.subscribers.len())
                .sum(),
            requests: self
                .sessions
                .values()
                .map(|state| state.moqt.peer_request_count())
                .sum(),
            request_streams: self
                .sessions
                .values()
                .map(|state| state.requests.len())
                .sum(),
            half_closed_streams: self
                .sessions
                .values()
                .map(|state| {
                    let finished = |slot: &&RequestSlot| slot.peer_finished();
                    state.requests.values().filter(finished).count()
                })
                .sum(),
            in_streams: self
                .tracks
                .all_live()
                .map(|(_, track)| track.in_streams.len())
                .sum(),
            out_streams: self.out_streams.len(),
            early_streams: self.sessions.values().map(|state| state.early.len()).sum(),
            namespaces: self.namespaces.len(),
            pending_tracks: self.tracks.all_pending().count(),
            own_requests: self.sessions.values().map(|state| state.own.len()).sum(),
        }
    }
}

/// Starts a subgroup stream towards a subscriber. The one place where a
/// stream is added to a subscription, so that what `PUBLISH_DONE` counts and
/// what is reset with the subscription cannot drift apart.
struct OutStreamAllocator<'a> {
    next: &'a mut u64,
    out_streams: &'a mut HashMap<OutStreamKey, OutStream>,
}

impl OutStreamAllocator<'_> {
    fn open(
        self,
        track: &FullTrackName,
        subscriber: &mut Subscriber,
        in_stream: &mut InStream,
        header: SubgroupHeader,
        actions: &mut Vec<Action>,
    ) {
        if !subscriber.forward {
            return;
        }
        let out_stream = OutStreamKey(*self.next);
        *self.next = self.next.saturating_add(1);
        subscriber.streams_started = subscriber.streams_started.saturating_add(1);
        subscriber.outs.insert(out_stream);
        in_stream.outs.insert(subscriber.session, out_stream);
        self.out_streams.insert(
            out_stream,
            OutStream {
                track: track.clone(),
                session: subscriber.session,
                opened: false,
            },
        );
        actions.push(Action::OpenDataStream {
            session: subscriber.session,
            out_stream,
            header: SubgroupHeader {
                track_alias: subscriber.track_alias,
                ..header
            },
        });
    }
}

/// Resets a stream that is not done and takes it out of the relay's books. A
/// stream that was not reported opened may never be: the count of the
/// subscription is then no longer exact (§10.11).
fn reset_out_stream(
    out_streams: &mut HashMap<OutStreamKey, OutStream>,
    subscriber: Option<&mut Subscriber>,
    out_stream: OutStreamKey,
    code: u32,
    actions: &mut Vec<Action>,
) {
    let Some(out) = out_streams.remove(&out_stream) else {
        return;
    };
    if let Some(subscriber) = subscriber {
        subscriber.outs.remove(&out_stream);
        subscriber.count_unknown |= !out.opened;
    }
    actions.push(Action::ResetDataStream { out_stream, code });
}

/// What an Object counts for against [`MAX_EARLY_BYTES`]: its payload and its
/// properties.
///
/// WHY the properties too: an Object can carry them without a payload, 64 of
/// them with values of up to 64 KiB each.
fn held_size(object: &SubgroupObject) -> usize {
    // What a pair takes apart from the bytes of its value: the type and a
    // varint value, 8 bytes each.
    const PAIR: usize = 16;
    let properties = object.properties.iter().map(|pair| match pair.value() {
        Value::Varint(_) => PAIR,
        Value::Bytes(bytes) => PAIR.saturating_add(bytes.len()),
    });
    properties.fold(object.payload.len(), usize::saturating_add)
}

/// Keeps an Object of a stream that waits for its Track Alias, within the
/// limits of the session.
fn hold_early_object(
    state: &mut SessionState,
    session: SessionKey,
    stream: StreamKey,
    object: SubgroupObject,
    actions: &mut Vec<Action>,
) {
    let Some(position) = state.early.iter().position(|early| early.stream == stream) else {
        return;
    };
    let held = state.early.iter().flat_map(|early| &early.objects);
    let (objects, bytes) = held.fold((0usize, 0usize), |(objects, bytes), object| {
        (objects + 1, bytes.saturating_add(held_size(object)))
    });
    let over =
        objects >= MAX_EARLY_OBJECTS || bytes.saturating_add(held_size(&object)) > MAX_EARLY_BYTES;
    if over {
        state.early.remove(position);
        actions.push(Action::StopDataStream {
            session,
            stream,
            code: stream_reset_code::INTERNAL_ERROR,
        });
    } else if let Some(early) = state.early.get_mut(position) {
        early.objects.push(object);
    }
}

/// Why a track ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackEnd {
    /// `PUBLISH_DONE`, and every stream its Stream Count named has ended.
    Done,
    /// `PUBLISH_DONE`, and the wait for the streams it named is over.
    TimedOut,
    /// The source cancelled or its session is gone.
    SourceGone,
    /// The last subscriber of a track the relay itself subscribed to is gone.
    Unwanted,
    /// The track broke the object model (§2.4.2).
    Malformed,
}

/// Where an action is carried out.
enum Target {
    Session(SessionKey),
    OutStream(OutStreamKey),
    /// The action closes a session, which is still to be done after the relay
    /// forgot the session.
    Closing,
}

impl Action {
    fn target(&self) -> Target {
        match self {
            Self::SendControl { session, .. }
            | Self::SendRequest { session, .. }
            | Self::FinishRequest { session, .. }
            | Self::ResetRequest { session, .. }
            | Self::OpenDataStream { session, .. }
            | Self::StopDataStream { session, .. }
            | Self::StartDrainTimer { session, .. }
            | Self::StartEarlyTimer { session, .. }
            | Self::StartRequestTimer { session, .. }
            | Self::StopRequest { session, .. }
            | Self::OpenRequest { session, .. }
            | Self::FinishOwnRequest { session, .. }
            | Self::CancelOwnRequest { session, .. }
            | Self::StartUpstreamTimer { session, .. }
            | Self::StopUpstreamTimer { session, .. } => Target::Session(*session),
            Self::SendObject { out_stream, .. }
            | Self::FinishDataStream { out_stream }
            | Self::ResetDataStream { out_stream, .. } => Target::OutStream(*out_stream),
            Self::CloseSession { .. } => Target::Closing,
        }
    }
}

fn internal_error(session: SessionKey, error: &dyn std::fmt::Display) -> Action {
    Action::CloseSession {
        session,
        code: TerminationCode::InternalError.code(),
        reason: error.to_string(),
    }
}
