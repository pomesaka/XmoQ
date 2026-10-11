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
//! Several publishers of one track (§9.3, §9.5) are handled in part: a
//! [`Track`] takes several sources ([`TrackSource`]), and the publishers of
//! its namespace are asked for it ([`Relay::ask_the_publishers`]). Five
//! things the draft asks for are not done, and
//! `docs/moqt-implementation-notes.md` has them under its known gaps: no
//! more publishers are asked than the track has room for; one that was
//! passed over for the requests open on it is not asked when one ends; one
//! that was asked and did not become a source is not asked again until it
//! announces anew; a `PUBLISH_NAMESPACE` does not get its publisher asked
//! for a track nobody is subscribed to; and a SUBSCRIBE for a track whose
//! sources all drain is refused.
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
mod fetch;
mod requests;
mod routing;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

pub use events::{
    Action, Event, FetchPipeEnd, OutStreamKey, OwnRequestEnd, OwnRequestKey, RequestEnd,
    SessionKey, StreamEnd, StreamKey,
};
pub use fetch::fetch_reset_passed_on;
use requests::{Requests, Subscribed};
use routing::{Announcement, Ask, Declined, OwnRequest, PendingTrack, Refusal, Waiter};
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
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
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

/// Groups of a track, counted back from the newest one seen, for which the
/// relay remembers which stream it took each Subgroup from. Provisional.
///
/// WHY remember at all: §2.2 has "Objects from the same Subgroup MUST NOT be
/// sent on different streams, unless one of the streams was reset
/// prematurely", and the relay is the publisher towards its subscribers. A
/// second stream of a Subgroup that arrives must not become a second stream
/// downstream, and to know that it is a second one the first has to be
/// remembered. WHY not for ever: a track can go on for as long as its
/// publisher likes. What becomes of a stream of a Group older than these
/// is at [`Subgroups::admit`].
pub const MAX_REMEMBERED_GROUPS: u64 = 8;

/// Subgroups of one track the relay remembers at most, over all the Groups
/// above: one Group can have as many Subgroups as its publisher likes. Past
/// that the ones of the oldest Group are forgotten first, and within a Group
/// the ones with the smallest Subgroup ID, whether their stream is still open
/// or not. Provisional.
pub const MAX_REMEMBERED_SUBGROUPS: usize = 256;

/// Sources one track has at most: publishers whose PUBLISH the relay
/// accepted, and publishers that accepted the relay's SUBSCRIBE.
/// Provisional.
///
/// WHY a limit: §9.3 has "Relays MUST handle Objects for the same Track from
/// multiple publishers", and every source costs the relay a subscription and
/// its streams for as long as it sends, whether anything of it is passed on
/// or not. §9.3 allows the limit: "An implementation can use mechanisms such
/// as `REQUEST_ERROR` or unsubscribing ... if it cannot accept an additional
/// publisher due to implementation constraints."
pub const MAX_SOURCES: usize = 4;

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
    /// A FETCH the relay asked a publisher for.
    Fetch {
        request: PeerRequestId,
        peer_finished: bool,
    },
    /// The relay has said its last on the stream. The request is given when the
    /// session still holds state for it: what the peer sends until the stream
    /// closes still goes to the session, which ignores it and keeps count of
    /// the Request IDs it uses up.
    Finished(Option<PeerRequestId>),
}

impl RequestSlot {
    /// For the slot of a SUBSCRIBE: its request, and how far it has come.
    fn subscription(&self) -> Option<(PeerRequestId, Subscribed)> {
        match self {
            Self::Subscribe { request, .. } => Some((*request, Subscribed::Accepted)),
            Self::Waiting { request, .. } => Some((*request, Subscribed::Waiting)),
            Self::PublishNamespace { .. }
            | Self::Fetch { .. }
            | Self::Publish { .. }
            | Self::Finished(_) => None,
        }
    }

    fn subscribed(&self) -> Option<Subscribed> {
        self.subscription().map(|(_, subscribed)| subscribed)
    }

    fn peer_finished(&self) -> bool {
        match self {
            Self::Subscribe { peer_finished, .. }
            | Self::PublishNamespace { peer_finished, .. }
            | Self::Waiting { peer_finished, .. }
            | Self::Fetch { peer_finished, .. } => *peer_finished,
            Self::Publish { .. } | Self::Finished(_) => false,
        }
    }

    /// Notes that the peer finished its direction. `false` for a request
    /// that does not go on after that.
    fn note_peer_finished(&mut self) -> bool {
        match self {
            Self::Subscribe { peer_finished, .. }
            | Self::PublishNamespace { peer_finished, .. }
            | Self::Waiting { peer_finished, .. }
            | Self::Fetch { peer_finished, .. } => {
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
    requests: Requests,
    /// Subgroup streams of this session that feed a track, by stream.
    in_streams: HashMap<StreamKey, FullTrackName>,
    early: VecDeque<EarlyStream>,
    /// SUBSCRIBE requests the relay opened on this session.
    own: HashMap<OwnRequestKey, OwnRequest>,
    /// FETCH requests the relay opened on this session.
    own_fetches: HashMap<OwnRequestKey, fetch::OwnFetch>,
}

impl SessionState {
    /// Requests the relay has open on this session: SUBSCRIBE and FETCH
    /// together, which share [`MAX_OWN_REQUESTS_PER_SESSION`].
    fn own_requests(&self) -> usize {
        self.own.len() + self.own_fetches.len()
    }

    /// Whether the peer finished its direction of a request stream while the
    /// request goes on.
    fn peer_finished(&self, stream: StreamKey) -> bool {
        self.requests.peer_finished(stream)
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
    /// The Joining Location (§5.1): the `LARGEST_OBJECT` of the `SUBSCRIBE_OK`
    /// this subscriber got, which a Joining FETCH counts its range from.
    /// `None` when no Object had been published.
    joining: Option<Location>,
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

/// What a subgroup stream of a source is to its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Passage {
    /// Which Subgroup it is of is not known yet: the header leaves the
    /// Subgroup ID to the first Object (§11.4.2, mode 0b01), and none has
    /// come. Nothing is opened downstream for it.
    Undetermined,
    /// It is the stream its Subgroup is taken from.
    Passed,
    /// Its Subgroup is taken from another stream, or was delivered whole
    /// already. It was stopped, and what still arrives on it is dropped; it
    /// is kept until it is reported closed, because it counts as a stream of
    /// the source's subscription (§10.11, Stream Count).
    Discarded,
}

/// A subgroup stream of a source.
#[derive(Debug)]
struct InStream {
    header: SubgroupHeader,
    /// The Subgroup ID, once known. With the mode "first Object ID" (§11.4.2)
    /// that is when the first Object arrives.
    subgroup_id: Option<u64>,
    /// The Publisher Priority of the Subgroup: the one in the header, or
    /// the default of the source the stream came from when it was opened.
    /// `None` where the relay cannot tell which subscription the stream is
    /// of, and so not which default it has: see
    /// [`Relay::on_data_stream_opened`].
    priority: Option<u8>,
    passage: Passage,
    /// Whether an Object was forwarded. A subscriber that joins after that
    /// gets a stream that does not start with the Subgroup's first Object.
    started: bool,
    /// Whether it was open when a new subscription took the Track Alias it
    /// carries: a stream of the subscription before, which the publisher
    /// does not count among the streams of the new one (§10.11, Stream
    /// Count).
    of_the_subscription_before: bool,
    /// The stream each subscriber gets this one on, by session.
    outs: HashMap<SessionKey, OutStreamKey>,
}

/// A subgroup stream of a source: the session it arrives on, and its number
/// there.
type InStreamKey = (SessionKey, StreamKey);

/// A source of a track: the session its Objects arrive on, and the Track
/// Alias they carry there. WHY the alias as well: a publisher that said
/// `PUBLISH_DONE` can publish the track anew while the relay still waits for
/// the streams of the first time, and the two are then told apart by it.
type SourceId = (SessionKey, u64);

/// How the stream a Subgroup was taken from stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TakenFrom {
    /// The stream is open. It carries that Track Alias.
    Open(InStreamKey, u64),
    /// The stream, of that source, ended without a FIN: the Subgroup may
    /// have more Objects.
    CutShort(SourceId),
    /// The stream ended with a FIN: the Subgroup was delivered whole
    /// (§11.4.3).
    Whole,
}

/// A Subgroup the relay has taken from a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    from: TakenFrom,
    /// The Publisher Priority of that stream ([`InStream::priority`]).
    priority: Option<u8>,
}

/// What became of a stream that arrived for a Subgroup of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taking {
    /// The Subgroup is taken from it.
    Taken,
    /// It was stopped: the Subgroup is taken from elsewhere.
    Left,
    /// It makes the track malformed; ending the track is the caller's.
    Malformed,
}

/// What to do with a stream that arrives for a Subgroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// Take the Subgroup from it.
    Pass,
    /// Take the Subgroup from it, in place of this stream that is still open.
    Replace(InStreamKey),
    /// Leave it: the Subgroup was delivered whole, is another source's, or
    /// is of a Group that is too old to tell.
    Discard,
    /// The track is malformed: the Subgroup came before with another
    /// Publisher Priority (§2.4.2).
    Malformed,
}

/// Which stream each Subgroup of a track is taken from, for the newest
/// Groups ([`MAX_REMEMBERED_GROUPS`], [`MAX_REMEMBERED_SUBGROUPS`]).
///
/// The three cases of a second stream for a Subgroup, from the same source:
///
/// * After the first was cut short: it passes. §2.2 allows it by name
///   ("unless one of the streams was reset prematurely"); it is the source
///   sending the Subgroup again.
/// * While the first is open: the one whose header arrived later takes the
///   place of the other. §11.4 has "A publisher SHOULD NOT open more than one
///   stream at a time with the same Subgroup Header field values", so either
///   the source goes against that, or it reset the first stream and the
///   relay has not seen the reset yet, since QUIC keeps no order between
///   streams. WHY the later one: in the second case the earlier one is over
///   and keeping it would lose the Subgroup that was sent again. WHY NOT
///   the stream that was opened later: the relay knows the order in which
///   the headers were read and not the QUIC stream IDs, so when the header of
///   the first stream is read after that of the second, it is the second, the
///   one the source goes on with, that is stopped. The stream of the other is
///   reset downstream before a new one is opened, so that no subscriber has
///   two streams for the Subgroup of which neither was reset (§2.2), nor two
///   open at a time with the same header (§11.4).
/// * After the first ended with a FIN: it is left. The Subgroup was delivered
///   whole (§11.4.3: "An MOQT implementation that processes a stream FIN is
///   assured it has received all objects in a subgroup"), and a second
///   stream for it downstream is what §2.2 forbids. A first stream that was
///   finished without an Object counts the same: the relay passed the FIN
///   on, and §11.4.3 has "If a sender closes the stream before delivering all
///   such objects to the QUIC stream, it MUST reset the stream", so a later
///   stream that passed would make the FIN the relay sent untrue.
///
/// A stream of another source than the one the Subgroup was taken from is
/// left in all three cases: first come, first taken.
///
/// * While the first is open: the same Subgroup from two sources is what
///   several publishers of a track send, and one of the two is enough. WHY
///   the first: the subscribers have its start already.
/// * After the first was cut short: WHY NOT go on with the other source,
///   which §2.2 allows as well: the relay would have to know that the other
///   source's stream starts where the first one stopped, and it has stopped
///   that stream when it arrived, if it arrived before. Whether to do it is
///   an open question for Phase 2. So a Subgroup ends for the subscribers
///   where its source ends, and the next Subgroup comes from whichever source
///   sends it first.
///
/// A stream for a Subgroup that came before with another Publisher Priority
/// makes the track malformed, from the same source or another, and however
/// the first stream stands. §2.4.2 has it as its first example: "An Object
/// with a particular Subgroup ID is received, but its Publisher Priority is
/// different from that of the previous Object with the same Subgroup ID".
/// WHY the priority and nothing else of the header: `END_OF_GROUP` is a hint
/// a publisher need not give (§11.4.2), `PROPERTIES` says how the Objects
/// are laid out, and what the Objects hold is not compared at all.
///
/// FIXME: detect an Object past the last one of a Subgroup that ended with a
/// FIN, and a Subgroup that comes on two streams that end with a FIN with
/// different last Objects (§2.4.2, the second and third example of a
/// malformed track). A stream that is left or replaced is stopped unread.
#[derive(Debug, Default)]
struct Subgroups {
    /// By Group ID and Subgroup ID.
    taken: BTreeMap<(u64, u64), Taken>,
    newest_group: Option<u64>,
}

impl Subgroups {
    /// A stream arrived for a Subgroup: what becomes of it. The Subgroup is
    /// then taken from it, unless the answer is [`Admission::Discard`].
    /// `several_sources`: the track has, or has had, more than one source at
    /// a time ([`Track::had_several_sources`]).
    ///
    /// A stream of a Group the relay has forgotten passes for a track that
    /// only ever had one source at a time, which may send an old Group late,
    /// and is left otherwise. WHY: with several sources, one that is 8
    /// Groups behind another sends what the subscribers have got already, as
    /// a matter of course, and the relay can no longer tell; passing it on
    /// would put the same Subgroup on a second stream of which neither was
    /// reset (§2.2). WHY NOT leave it with one source as well: nothing has
    /// sent that Group but the source itself. What is lost: a source that
    /// sends an old Group late, for a reason of its own, is not heard once
    /// its track has had a second source.
    fn admit(
        &mut self,
        group: u64,
        subgroup: u64,
        stream: InStreamKey,
        track_alias: u64,
        priority: Option<u8>,
        several_sources: bool,
    ) -> Admission {
        let newest = self.newest_group.map_or(group, |newest| newest.max(group));
        self.newest_group = Some(newest);
        let forgotten = |group: u64| newest.saturating_sub(group) >= MAX_REMEMBERED_GROUPS;
        if forgotten(group) {
            return if several_sources {
                Admission::Discard
            } else {
                Admission::Pass
            };
        }
        let source: SourceId = (stream.0, track_alias);
        let taken = self.taken.get(&(group, subgroup));
        // Two priorities that are known and differ. One the relay is not
        // sure of is not held against the track.
        let differs = |taken: &Taken| match (taken.priority, priority) {
            (Some(before), Some(now)) => before != now,
            _ => false,
        };
        if taken.is_some_and(differs) {
            return Admission::Malformed;
        }
        let admission = match taken.map(|taken| taken.from) {
            Some(TakenFrom::Open(open, _)) if open == stream => Admission::Pass,
            Some(TakenFrom::Open(open, alias)) if (open.0, alias) == source => {
                Admission::Replace(open)
            }
            Some(TakenFrom::CutShort(by)) if by == source => Admission::Pass,
            // From another source, or delivered whole.
            Some(TakenFrom::Open(..) | TakenFrom::CutShort(_) | TakenFrom::Whole) => {
                return Admission::Discard;
            }
            None => Admission::Pass,
        };
        let from = TakenFrom::Open(stream, track_alias);
        self.taken
            .insert((group, subgroup), Taken { from, priority });
        while let Some(((oldest, _), _)) = self.taken.first_key_value()
            && (forgotten(*oldest) || self.taken.len() > MAX_REMEMBERED_SUBGROUPS)
        {
            self.taken.pop_first();
        }
        admission
    }

    /// The stream a Subgroup is taken from ended, with a FIN or without.
    fn ended(&mut self, group: u64, subgroup: u64, stream: InStreamKey, fin: bool) {
        if let Some(taken) = self.taken.get_mut(&(group, subgroup))
            && let TakenFrom::Open(open, track_alias) = taken.from
            && open == stream
        {
            taken.from = if fin {
                TakenFrom::Whole
            } else {
                TakenFrom::CutShort((stream.0, track_alias))
            };
        }
    }
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
    /// The wait for those streams, as [`Action::StartDrainTimer`] numbers
    /// it, once one runs.
    wait: Option<u64>,
    /// The Publisher Priority of its Subgroups that carry none (§11.4.2:
    /// such a Subgroup "inherits the Publisher Priority specified in the
    /// control message that established the subscription").
    default_priority: u8,
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

    /// Whether a stream that arrived on that session under that Track Alias
    /// is one of this source.
    fn sent(self, stream_of: SourceId) -> bool {
        self.session_and_alias() == stream_of
    }
}

/// A track with a source: an Established upstream subscription (§9.4), or
/// one whose last source drains.
#[derive(Debug)]
struct Track {
    /// Never empty: a track whose last source is over is taken out of
    /// [`Tracks`]. No more than [`MAX_SOURCES`], at most one of a session
    /// and a Track Alias, and at most one of a session that does not drain;
    /// [`Tracks::add_source`] refuses or replaces what would break these.
    ///
    /// The streams of a source are told from those of another by their
    /// session and the Track Alias they carry. While a publisher's
    /// subscription lasts, the session keeps a second one from coming about:
    /// the relay is the subscriber of every source, §5.1 has an endpoint hold
    /// at most one subscription to a track as the subscriber, and the session
    /// refuses a PUBLISH where the relay's SUBSCRIBE or an earlier PUBLISH is
    /// Established (`DUPLICATE_SUBSCRIPTION`). A SUBSCRIBE of the relay that
    /// has no answer yet gives way to the PUBLISH
    /// ([`Relay::give_way_to_publish`]). After its own `PUBLISH_DONE` a
    /// publisher may send PUBLISH anew while the relay still waits for the
    /// streams that counted: a session then has a source that drains and
    /// one that goes on, or several that drain.
    sources: Vec<Source>,
    /// The track has had more than one source at a time, now or earlier.
    ///
    /// WHY "or earlier", and never taken back: the source that was ahead can
    /// be the one that goes. The one that is left then sends Groups the
    /// subscribers have had from the other, and that is the case two sources
    /// are there for. Which Groups those are is not known once they are older
    /// than what the relay remembers.
    had_several_sources: bool,
    /// What is asked of publishers and not answered yet: of the ones that
    /// were asked with the publisher that became the first source, and of
    /// the ones asked since. Each ends with the answer, with its time, with
    /// the publisher, with the last subscriber and with the track.
    asks: Vec<Ask>,
    /// The sessions that were asked for the track and did not become a
    /// source: they refused, did not answer in time, or closed the request.
    /// They are not asked again until they announce a namespace of the track
    /// anew or become a source.
    declined: Declined,
    /// Track Properties, as the source that came last gave them; §9.6 has
    /// the relay pass them on. §2.5: "the most recent set SHOULD replace any
    /// cached values" and "Relays MUST NOT attempt to merge sets of unknown
    /// properties received in different messages".
    properties: Vec<KeyValuePair>,
    /// The largest Location known for the track (§10.2.11): what the source
    /// said, or a larger one seen since.
    largest: Option<Location>,
    subscribers: Vec<Subscriber>,
    /// By the session and the stream they arrive on: stream numbers repeat
    /// from session to session.
    in_streams: HashMap<InStreamKey, InStream>,
    subgroups: Subgroups,
}

impl Track {
    /// A track with its first source; [`Tracks::add_source`] is the one
    /// place that makes one.
    fn established(
        from: TrackSource,
        properties: Vec<KeyValuePair>,
        largest: Option<Location>,
    ) -> Self {
        Self {
            sources: vec![Source {
                from,
                draining: false,
                wait: None,
                default_priority: default_publisher_priority(&properties),
            }],
            had_several_sources: false,
            asks: Vec::new(),
            declined: Declined::default(),
            properties,
            largest,
            subscribers: Vec::new(),
            in_streams: HashMap::new(),
            subgroups: Subgroups::default(),
        }
    }

    /// A stream of a source turned out to be of this Subgroup. Says whether
    /// the Subgroup is taken from it; when it is left, the stream is stopped.
    /// A stream it takes the place of is stopped, and what was opened for
    /// that one downstream is reset. `priority` is the Publisher Priority of
    /// the stream ([`InStream::priority`]).
    ///
    /// §11.4.3 for the stop: "A subscriber MAY send a QUIC `STOP_SENDING`
    /// frame for a subgroup stream if the Group or Subgroup is no longer of
    /// interest to it."
    fn admit(
        &mut self,
        subgroup: (u64, u64),
        stream: InStreamKey,
        (track_alias, priority): (u64, Option<u8>),
        out_streams: &mut HashMap<OutStreamKey, OutStream>,
        actions: &mut Vec<Action>,
    ) -> Taking {
        let stop = |(session, stream): InStreamKey| Action::StopDataStream {
            session,
            stream,
            code: stream_reset_code::CANCELLED,
        };
        let several = self.had_several_sources;
        let (group, subgroup) = subgroup;
        match self
            .subgroups
            .admit(group, subgroup, stream, track_alias, priority, several)
        {
            Admission::Pass => Taking::Taken,
            Admission::Malformed => Taking::Malformed,
            Admission::Discard => {
                actions.push(stop(stream));
                Taking::Left
            }
            Admission::Replace(old) => {
                if let Some(replaced) = self.in_streams.get_mut(&old) {
                    replaced.passage = Passage::Discarded;
                    // In the order the subscribers came, as everywhere.
                    for subscriber in &mut self.subscribers {
                        if let Some(out_stream) = replaced.outs.remove(&subscriber.session) {
                            reset_out_stream(
                                out_streams,
                                Some(subscriber),
                                out_stream,
                                stream_reset_code::CANCELLED,
                                actions,
                            );
                        }
                    }
                    actions.push(stop(old));
                }
                Taking::Taken
            }
        }
    }

    /// A stream of a source is over: the Subgroup it carried ends for the
    /// subscribers as the stream did, and the track forgets the stream.
    fn in_stream_ended(
        &mut self,
        stream: InStreamKey,
        end: StreamEnd,
        out_streams: &mut HashMap<OutStreamKey, OutStream>,
        actions: &mut Vec<Action>,
    ) {
        let Some(in_stream) = self.in_streams.remove(&stream) else {
            return;
        };
        if let (Passage::Passed, Some(subgroup_id)) = (in_stream.passage, in_stream.subgroup_id) {
            let group = in_stream.header.group_id;
            self.subgroups
                .ended(group, subgroup_id, stream, end == StreamEnd::Fin);
        }
        for subscriber in &mut self.subscribers {
            let Some(out_stream) = in_stream.outs.get(&subscriber.session).copied() else {
                continue;
            };
            let code = match end {
                // The stream stays in the relay's books until the I/O side
                // has written it out.
                StreamEnd::Fin => {
                    actions.push(Action::FinishDataStream { out_stream });
                    continue;
                }
                // §11.4.3: a stream that will not carry the rest of its
                // Subgroup is reset. The publisher's code says why, when
                // there is one.
                StreamEnd::Aborted(code) => code.unwrap_or(stream_reset_code::INTERNAL_ERROR),
                // WHY CANCELLED: the publisher is gone, and that is the code
                // its other open streams get when the end of its session is
                // what the relay sees first. Which of the two comes first is
                // a race of the I/O side, and the subscriber is to see one
                // code for one cause.
                StreamEnd::Lost => stream_reset_code::CANCELLED,
            };
            reset_out_stream(out_streams, Some(subscriber), out_stream, code, actions);
        }
    }

    fn sources(&self) -> impl Iterator<Item = &Source> {
        self.sources.iter()
    }

    fn sources_mut(&mut self) -> impl Iterator<Item = &mut Source> {
        self.sources.iter_mut()
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

    /// Gives a track a source: an Established upstream subscription (§9.4).
    /// A track of the name that waited for one becomes a track with a source
    /// and keeps what is still asked of other publishers. Returns the
    /// subscribers that waited; answering them is the caller's.
    ///
    /// A track that has a source gets another. Its Track Properties are the
    /// ones of the new source from then on, and its largest Location the
    /// larger of the two (§10.2.11: "the largest of the following").
    ///
    /// WHY one place for a name without an entry and one that waits: what a
    /// pending track holds, subscribers that wait and requests that are open,
    /// must not be dropped without a word, and a caller that makes the entry
    /// itself has to remember that.
    ///
    /// # Errors
    ///
    /// The track cannot take the source, and is left as it was.
    fn add_source(
        &mut self,
        name: &FullTrackName,
        from: TrackSource,
        properties: Vec<KeyValuePair>,
        largest: Option<Location>,
    ) -> Result<Added, NoSource> {
        if let Some(track) = self.live_mut(name) {
            let new = Source {
                from,
                draining: false,
                wait: None,
                default_priority: default_publisher_priority(&properties),
            };
            let (session, track_alias) = new.session_and_alias();
            // The same session under the same alias: the alias is the new
            // subscription's from here on, and the one that had it is over
            // (it drained; the session refuses a PUBLISH where one goes on).
            // Its streams and what the relay remembers of its Subgroups are
            // the new one's, since nothing tells them apart.
            let same = |source: &&mut Source| source.session_and_alias() == new.session_and_alias();
            let replaced = if let Some(old) = track.sources.iter_mut().find(same) {
                // WHY marked: the end of such a stream would count towards
                // the Stream Count of the new subscription, which then ends
                // before its own streams have. WHY NOT stop them: what they
                // still carry is the track's.
                for (key, in_stream) in &mut track.in_streams {
                    if key.0 == session && in_stream.header.track_alias == track_alias {
                        in_stream.of_the_subscription_before = true;
                    }
                }
                Some(std::mem::replace(old, new))
            } else {
                let going_on =
                    |source: &Source| source.session_and_alias().0 == session && !source.draining;
                if track.sources().any(going_on) {
                    return Err(NoSource::OneOnTheSession);
                }
                // A source that drains still holds its streams, and counts.
                if track.sources.len() >= MAX_SOURCES {
                    return Err(NoSource::TooMany);
                }
                // WHY of another session: a publisher that publishes anew
                // while its first subscription drains is one publisher.
                let of_another_session = track
                    .sources()
                    .any(|source| source.session_and_alias().0 != session);
                track.had_several_sources |= of_another_session;
                track.sources.push(new);
                None
            };
            track.declined.forget(session);
            track.properties = properties;
            track.largest = track.largest.max(largest);
            return Ok(Added {
                waiters: Vec::new(),
                replaced,
            });
        }
        let mut track = Track::established(from, properties, largest);
        let waiters = match self.0.remove(name) {
            Some(TrackEntry::Pending(pending)) => {
                track.asks = pending.asks;
                track.declined = pending.declined;
                track.declined.forget(from.session_and_alias().0);
                pending.waiters
            }
            // Not a track with a source: that was the case above.
            Some(TrackEntry::Live(_)) | None => Vec::new(),
        };
        self.0.insert(name.clone(), TrackEntry::Live(track));
        Ok(Added {
            waiters,
            replaced: None,
        })
    }

    /// Puts back a track that [`Self::take_live`] took out.
    fn put_live(&mut self, name: &FullTrackName, track: Track) {
        self.0.insert(name.clone(), TrackEntry::Live(track));
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

    /// The sessions that were asked for the track and did not become a
    /// source.
    fn declined(&self, name: &FullTrackName) -> impl Iterator<Item = SessionKey> + '_ {
        let declined = match self.0.get(name) {
            Some(TrackEntry::Pending(pending)) => Some(&pending.declined),
            Some(TrackEntry::Live(track)) => Some(&track.declined),
            None => None,
        };
        declined.into_iter().flat_map(Declined::sessions)
    }

    /// Remembers that a session was asked for the track and did not become
    /// a source. The one place that does; see [`Declined`].
    ///
    /// WHY not a session that is a source: it became one another way, with
    /// a PUBLISH, and is not asked while it is one in any case.
    fn decline(&mut self, name: &FullTrackName, session: SessionKey) {
        match self.0.get_mut(name) {
            Some(TrackEntry::Pending(pending)) => pending.declined.remember(session),
            Some(TrackEntry::Live(track)) => {
                let is_a_source = track
                    .sources()
                    .any(|source| source.session_and_alias().0 == session);
                if !is_a_source {
                    track.declined.remember(session);
                }
            }
            None => {}
        }
    }

    /// The tracks in a namespace that somebody is subscribed to or waits
    /// for, in an order that does not depend on the map.
    ///
    /// WHY not a track that has a source and no subscriber: the relay holds
    /// it for a publisher that sent PUBLISH, and nobody wants more of it.
    fn subscribed_in(&self, namespace: &TrackNamespace) -> BTreeSet<FullTrackName> {
        let subscribed = |entry: &TrackEntry| match entry {
            TrackEntry::Pending(_) => true,
            TrackEntry::Live(track) => !track.subscribers.is_empty(),
        };
        self.0
            .iter()
            .filter(|(name, entry)| namespace.is_prefix_of(name.namespace()) && subscribed(entry))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Lets a session be asked again for the tracks `of` picks. Every track
    /// is looked at, for each namespace that is announced and each session
    /// that goes; see [`Relay::end_sources_under`] for when that is to be
    /// looked up another way.
    fn forget_declined(&mut self, session: SessionKey, of: impl Fn(&FullTrackName) -> bool) {
        for (name, entry) in &mut self.0 {
            let declined = match entry {
                TrackEntry::Pending(pending) => &mut pending.declined,
                TrackEntry::Live(track) => &mut track.declined,
            };
            if of(name) {
                declined.forget(session);
            }
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

/// What adding a source to a track brought with it.
#[derive(Debug)]
struct Added {
    /// The subscribers that waited for the track to have a source.
    waiters: Vec<Waiter>,
    /// The source of the same session and Track Alias that the new one took
    /// the place of. The caller tells its publisher that the relay no longer
    /// waits for it.
    replaced: Option<Source>,
}

/// Why a track does not take a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoSource {
    /// It has [`MAX_SOURCES`] of them.
    TooMany,
    /// It has one on that session that goes on. The session keeps a second
    /// subscription from coming about while the first lasts (§5.1), so this
    /// is not expected.
    OneOnTheSession,
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
    /// Sources, over all tracks ([`MAX_SOURCES`] bounds them for each).
    pub sources: usize,
    /// What is asked of publishers and not answered, over all tracks, with a
    /// source and without.
    pub asks: usize,
    /// Publishers remembered as asked for a track without becoming a source,
    /// over all tracks, with a source and without.
    pub declined: usize,
    /// Subgroup streams held for a Track Alias that is not known yet.
    pub early_streams: usize,
    /// Subgroups the tracks remember the stream of, over all tracks
    /// ([`MAX_REMEMBERED_SUBGROUPS`] bounds them for each).
    pub remembered_subgroups: usize,
    /// Namespaces announced with `PUBLISH_NAMESPACE` and not withdrawn.
    pub namespaces: usize,
    /// Tracks the relay asked a publisher for and has no answer about.
    pub pending_tracks: usize,
    /// Requests the relay opened and has not let go of, as the relay counts
    /// them. The I/O side has a task for each that is not answered or is
    /// Established; a test that compares the two numbers then sees a request
    /// one side forgot. A request whose publisher announced a PUBLISH with
    /// `DUPLICATE_SUBSCRIPTION` is counted here until that wait is over, and
    /// has no task any more. The relay's FETCH requests are among them.
    pub own_requests: usize,
    /// FETCH requests of subscribers that are not over: asked of a publisher,
    /// waiting for the SUBSCRIBE they join, or answered by the relay with a
    /// fetch stream that is not reported written.
    pub fetches: usize,
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
    /// FETCH requests of subscribers.
    fetches: fetch::Fetches,
    next_drain_wait: u64,
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
            fetches: fetch::Fetches::default(),
            next_drain_wait: 0,
            next_own_request: 0,
        }
    }

    /// Takes one event and returns what to do about it, in order.
    ///
    /// An event for a session or stream the relay no longer knows is ignored:
    /// the I/O side reports what it read, and some of that is read after the
    /// relay let go.
    pub fn handle(&mut self, event: Event) -> Vec<Action> {
        // A request of the peer and a message on one use a Request ID of the
        // peer (§10.1, a REQUEST_UPDATE as well): a Joining FETCH that waits
        // for the SUBSCRIBE with that ID to come has its answer then.
        let used_a_request_id = match &event {
            Event::RequestOpened { session, .. } | Event::RequestMessage { session, .. } => {
                Some(*session)
            }
            _ => None,
        };
        let mut actions = self.handle_event(event);
        if let Some(session) = used_a_request_id {
            self.settle_early_fetches(session, None, &mut actions);
        }
        actions
    }

    fn handle_event(&mut self, event: Event) -> Vec<Action> {
        let mut actions = Vec::new();
        // What has to do with a FETCH the relay passes on goes its own way.
        let Some(event) = self.on_fetch_event(event, &mut actions) else {
            return actions;
        };
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
            // Taken by `on_fetch_event` above.
            Event::FetchStreamOpened { .. }
            | Event::FetchPipeEnded { .. }
            | Event::JoiningFetchTimeout { .. }
            | Event::FetchStreamWritten { .. } => {}
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
                self.on_track_malformed(session, stream, &mut actions);
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
                wait,
            } => self.on_drain_timeout(session, track_alias, wait, &mut actions),
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
        wait: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let track = match state.moqt.subscribed_alias(track_alias) {
            SubscribedAlias::Draining(track) => track.clone(),
            _ => return,
        };
        // The timer of a wait that is over, for a subscription that had the
        // alias before: the source that drains under it now has its own.
        let of_an_earlier_wait = self.tracks.live(&track).is_some_and(|track| {
            track
                .sources()
                .any(|source| source.sent((session, track_alias)) && source.wait != Some(wait))
        });
        if of_an_earlier_wait {
            return;
        }
        // WHY without a look at the number where no source has the alias:
        // the session still holds an alias that drains, nothing in the relay
        // will end it, and no later wait can be meant.
        state.moqt.on_drain_timeout(track_alias);
        let timed_out = |source: &Source| source.session_and_alias() == (session, track_alias);
        self.source_gone(&track, timed_out, TrackEnd::TimedOut, actions);
    }

    /// A stream of a source broke the object model (§2.4.2): the track
    /// ends, and it is held against the source whose alias the stream
    /// carries.
    fn on_track_malformed(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(track) = self
            .sessions
            .get(&session)
            .and_then(|state| state.in_streams.get(&stream))
            .cloned()
        else {
            return;
        };
        let in_stream = self
            .tracks
            .live(&track)
            .and_then(|live| live.in_streams.get(&(session, stream)));
        let Some(track_alias) = in_stream.map(|in_stream| in_stream.header.track_alias) else {
            return;
        };
        let end = TrackEnd::Malformed {
            by: (session, track_alias),
        };
        // Before the subscriptions end: a Joining FETCH would otherwise go
        // with its subscription, and with another code than §2.4.2 has.
        self.fetches_of_a_malformed_track(&track, actions);
        self.end_track(&track, end, actions);
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
        let Some(RequestSlot::Finished(Some(request))) = state.requests.get(stream) else {
            return;
        };
        state.moqt.on_peer_request_closed(*request);
        // The slot stays until the stream really closes, so that what the peer
        // still sends on it is ignored.
        self.put_slot(session, stream, RequestSlot::Finished(None), actions);
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
                requests: Requests::new(),
                in_streams: HashMap::new(),
                early: VecDeque::new(),
                own: HashMap::new(),
                own_fetches: HashMap::new(),
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
                if let Some(track_alias) = offered_alias {
                    state.drop_early_streams(session, track_alias, actions);
                }
                self.put_slot(session, stream, RequestSlot::Finished(None), actions);
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
            PeerRequest::Fetch(fetch) => self.on_fetch(session, stream, request, &fetch, actions),
            PeerRequest::Subscribe(subscribe) => {
                self.on_subscribe(session, stream, request, &subscribe, actions);
            }
            PeerRequest::Publish(publish) => {
                self.on_publish(session, stream, request, publish, actions);
            }
            PeerRequest::PublishNamespace(publish_namespace) => {
                let namespace = publish_namespace.track_namespace;
                if !self.add_namespace(session, stream, namespace.clone()) {
                    let too_many = Refusal::code(request_error_code::EXCESSIVE_LOAD);
                    self.refuse(session, stream, request, &too_many, actions);
                    return;
                }
                let Some(state) = self.sessions.get_mut(&session) else {
                    return;
                };
                let response = state.moqt.accept_publish_namespace(request);
                self.put_slot(
                    session,
                    stream,
                    RequestSlot::PublishNamespace {
                        request,
                        peer_finished: false,
                    },
                    actions,
                );
                self.respond(session, stream, response, actions);
                // §9.5: "When a relay receives an authorized
                // PUBLISH_NAMESPACE for a namespace that matches one or more
                // existing subscriptions to other upstream sessions, it MUST
                // send a SUBSCRIBE to the publisher that sent the
                // PUBLISH_NAMESPACE for each matching subscription."
                for track in self.tracks.subscribed_in(&namespace) {
                    self.ask_the_publishers(&track, Some(session), actions);
                }
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
            // §9.5 has the SUBSCRIBE go to the publishers of the namespace
            // as well: "Relays MUST send SUBSCRIBE messages to all matching
            // publishers. This includes matching both Established
            // subscriptions on the Full Track Name and Namespace Prefix
            // Matching against published Namespaces". WHY read so, though the
            // next paragraph gives sending only for a track without an
            // Established upstream subscription: a PUBLISH_NAMESPACE that
            // comes later is asked for the track in any case, and whether a
            // publisher is asked must not depend on that order.
            Some(TrackPhase::Established) => {
                self.establish(track_name, &waiter, actions);
                self.ask_the_publishers(track_name, None, actions);
                return;
            }
            // Every source said PUBLISH_DONE and the track still drains: it
            // has no Established upstream subscription (§9.4). §9.5 has the
            // publishers of the namespace asked then; the subscriber is
            // refused, because a track with a source has no place for one
            // that waits (known gap). WHY a PUBLISH_NAMESPACE that arrives
            // now does have its publisher asked: the track has subscribers,
            // who are there already and stay if that publisher accepts.
            Some(TrackPhase::Draining) => Err(request_error_code::DOES_NOT_EXIST),
            Some(TrackPhase::Pending) => {
                let waits = self.wait_with_the_others(track_name, waiter);
                if waits.is_ok() {
                    self.ask_the_publishers(track_name, None, actions);
                }
                waits
            }
            // §9.5: "If there are no Established upstream subscriptions for
            // the requested Track, the Relay MUST send a SUBSCRIBE request to
            // each publisher that has published the subscription's namespace
            // or prefix thereof."
            None => {
                let asked = self.ask_the_publishers(track_name, None, actions);
                if asked.asks.is_empty() {
                    // §10.2.6: without RENDEZVOUS_TIMEOUT, or with 0, "MUST
                    // immediately return REQUEST_ERROR with error code
                    // DOES_NOT_EXIST if no publisher is available". Where
                    // one is there and could not be asked, the code says
                    // why.
                    Err(asked
                        .nobody_because
                        .unwrap_or(request_error_code::DOES_NOT_EXIST))
                } else {
                    let pending = PendingTrack {
                        asks: asked.asks,
                        waiters: vec![waiter],
                        declined: Declined::default(),
                    };
                    self.tracks.insert_pending(track_name.clone(), pending);
                    Ok(())
                }
            }
        };
        match waits {
            Ok(()) => {
                let slot = RequestSlot::Waiting {
                    request,
                    track: track_name.clone(),
                    peer_finished: false,
                };
                self.put_slot(session, stream, slot, actions);
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
        if !self.sessions.contains_key(&session) {
            return;
        }
        let track_alias = publish.track_alias;
        let (track_name, waiters) = match self.take_publish(session, stream, publish, actions) {
            Ok(taken) => taken,
            Err(refusal) => {
                if let Some(state) = self.sessions.get_mut(&session) {
                    state.drop_early_streams(session, track_alias, actions);
                }
                self.refuse(session, stream, request, &refusal, actions);
                return;
            }
        };
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
        self.put_slot(
            session,
            stream,
            RequestSlot::Publish {
                request,
                track: track_name.clone(),
            },
            actions,
        );
        self.respond(session, stream, response, actions);
        // §9.4: the track has an Established upstream subscription now.
        for waiter in &waiters {
            self.establish(&track_name, waiter, actions);
        }
        self.replay_early_streams(session, track_alias, actions);
    }

    /// Makes the publisher of a PUBLISH a source of its track. Returns the
    /// track and the subscribers that waited for it, or what the PUBLISH is
    /// refused with. A SUBSCRIBE of the relay to the same publisher for the
    /// track has given way when this returns; the `PUBLISH_OK` is the caller's.
    fn take_publish(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        publish: Publish,
        actions: &mut Vec<Action>,
    ) -> Result<(FullTrackName, Vec<Waiter>), Refusal> {
        // §5.1: "If a subscriber receives a PUBLISH for a Track with a
        // subscription in the Pending (Subscriber) state, it MUST ensure the
        // subscription it initiated transitions to the Terminated state
        // before sending PUBLISH_OK." The relay's SUBSCRIBE to this session
        // gives way, and the PUBLISH becomes the source of the track.
        //
        // WHY only for the session that was asked: a subscription is between
        // two endpoints, so the sentence is about the PUBLISH of the session
        // the SUBSCRIBE went to. What the relay asked of other sessions is
        // still asked, and an answer that comes makes a further source.
        let asked_here = self.is_asked_of(&publish.full_track_name, session);
        // A track takes the source in whatever state it is (§9.3, §9.5: "When
        // it receives an authorized PUBLISH message for a Track that has
        // Established downstream subscriptions, it MUST respond with
        // PUBLISH_OK"; the relay does so with and without subscribers). One
        // that waited for a publisher's answer has its source now, and the
        // subscribers that waited are answered. One whose sources have all
        // said `PUBLISH_DONE` goes on with this one, for the subscribers it
        // has and for new ones.
        //
        // §10.2.11: the relay's LARGEST_OBJECT starts from the one the
        // publisher gave.
        let largest = largest_object(&publish.parameters);
        let track_name = publish.full_track_name;
        let published = TrackSource::Publisher {
            session,
            stream,
            track_alias: publish.track_alias,
        };
        let added = self
            .tracks
            .add_source(&track_name, published, publish.track_properties, largest)
            .map_err(|refused| match refused {
                // WHY EXCESSIVE_LOAD: "The responder is overloaded and cannot
                // process the request at this time" (§10.6.2), the code of
                // the relay's other limits. UNINTERESTED would say that the
                // relay does not want the track.
                NoSource::TooMany => Refusal::code(request_error_code::EXCESSIVE_LOAD),
                // Not expected: the session has refused the PUBLISH of a
                // publisher whose subscription for the track goes on. WHY
                // UNINTERESTED: it is the code §10.6.2 gives a subscriber
                // for declining a PUBLISH, and the relay is the subscriber
                // here.
                NoSource::OneOnTheSession => Refusal::code(request_error_code::UNINTERESTED),
            })?;
        self.alias_was_taken_over(&track_name, published, added.replaced, actions);
        let waiters = added.waiters;
        // Before the PUBLISH_OK, as §5.1 has it. What was asked of other
        // publishers goes on under the track with a source. WHY after the
        // source is there: a PUBLISH that is refused leaves what the relay
        // asked as it was.
        if asked_here {
            self.give_way_to_publish(&track_name, session, actions);
        }
        Ok((track_name, waiters))
    }

    /// A source was added under a Track Alias its session had used for
    /// another subscription, which drained: that one is over (a publisher may
    /// use an alias again once it said `PUBLISH_DONE`, and the alias then
    /// means the new subscription; implementation notes, row on a reused
    /// Track Alias). `replaced` is the one on the same track, whose place
    /// the new source took.
    ///
    /// WHY told as when the wait ran out, with a reset and not a FIN: the
    /// relay stops waiting for the streams the old subscription counted, and
    /// a FIN would say that they all came.
    fn alias_was_taken_over(
        &mut self,
        track_name: &FullTrackName,
        new: TrackSource,
        replaced: Option<Source>,
        actions: &mut Vec<Action>,
    ) {
        let (session, track_alias) = new.session_and_alias();
        if let Some(old) = replaced {
            let code = stream_reset_code::CANCELLED;
            match old.from {
                TrackSource::Publisher { stream, .. } => {
                    self.release_publish(session, stream, TrackEnd::TimedOut, code, actions);
                }
                TrackSource::Subscription { request, .. } => {
                    self.release_subscription(session, request, TrackEnd::TimedOut, code, actions);
                }
            }
        }
        self.end_sources_under(session, track_alias, Some(track_name), actions);
    }

    /// Ends the sources whose streams arrive on that session under that
    /// Track Alias, except the one of the track `kept`: the session has
    /// given the alias to another subscription.
    fn end_sources_under(
        &mut self,
        session: SessionKey,
        track_alias: u64,
        kept: Option<&FullTrackName>,
        actions: &mut Vec<Action>,
    ) {
        let id = (session, track_alias);
        // By track name, for the reason `release_source` gives. Every track is looked at for
        // every source that is added, which costs nothing with the handful
        // of tracks a relay has now and is to be looked up by session and
        // alias once a relay carries many tracks.
        let others: BTreeSet<FullTrackName> = self
            .tracks
            .all_live()
            .filter(|(name, track)| {
                Some(*name) != kept && track.sources().any(|source| source.sent(id))
            })
            .map(|(name, _)| name.clone())
            .collect();
        for other in others {
            let had_the_alias = |source: &Source| source.sent(id);
            self.source_gone(&other, had_the_alias, TrackEnd::TimedOut, actions);
        }
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
        let (request, track) = match state.requests.get(stream) {
            Some(
                RequestSlot::Subscribe { request, track, .. }
                | RequestSlot::Publish { request, track }
                | RequestSlot::Waiting { request, track, .. },
            ) => (*request, Some(track.clone())),
            Some(
                RequestSlot::PublishNamespace { request, .. }
                | RequestSlot::Fetch { request, .. }
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
                self.put_slot(session, stream, RequestSlot::Finished(None), actions);
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
                        self.put_slot(session, stream, RequestSlot::Finished(None), actions);
                    }
                }
            }
            // For a FETCH that has its FETCH_OK. Nothing of it changes.
            PeerRequestEvent::UpdateAccepted { response } => actions.push(Action::SendRequest {
                session,
                stream,
                message: response,
            }),
            PeerRequestEvent::UpdateHeld => self.on_fetch_update_held(session, stream, actions),
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
        self.retire_drained_sources(track_name, actions);
        // Still a source: streams it counted are yet to end.
        let waits = self
            .tracks
            .live(track_name)
            .is_some_and(|track| track.sources().any(&said_by));
        if waits {
            let wait = self.next_drain_wait;
            self.next_drain_wait = self.next_drain_wait.saturating_add(1);
            if let Some(track) = self.tracks.live_mut(track_name) {
                for source in track.sources_mut().filter(|source| said_by(source)) {
                    source.wait = Some(wait);
                }
            }
            actions.push(Action::StartDrainTimer {
                session,
                track_alias,
                wait,
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
        let slot = self.take_slot(session, stream, actions);
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
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
            // §5.2: the subscriber cancelled its FETCH.
            Some(RequestSlot::Fetch { .. }) => self.on_fetch_cancelled(session, stream, actions),
            Some(RequestSlot::Subscribe { request, track, .. }) => {
                state.moqt.on_peer_request_closed(request);
                // The subscriber cancelled (§3.3.2, §5.1.1 "MUST reset any open
                // streams associated with the SUBSCRIBE"). No PUBLISH_DONE can
                // follow on a stream that is gone.
                self.remove_subscriber(&track, session, stream_reset_code::CANCELLED, actions);
            }
            Some(RequestSlot::Publish { request, track }) => {
                state.moqt.on_peer_request_closed(request);
                // After PUBLISH_DONE the stream may close, and the source
                // goes on draining.
                let cancelled =
                    |source: &Source| source.is_publish(session, stream) && !source.draining;
                self.source_gone(&track, cancelled, TrackEnd::SourceGone, actions);
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
        let goes_on = state.requests.note_peer_finished(stream);
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
        // The alias still drains and the track is gone, as after a malformed
        // track: nothing of it is wanted. Nor is it when a track of the name
        // has come about since, from another publisher: this session is no
        // source of that one. WHY looked at here: such a stream would be
        // taken for an Object of the new track, and no source would be
        // there to let go of it with.
        let id = (session, header.track_alias);
        let of_its_source = self.tracks.live_mut(&track_name).and_then(|track| {
            let source = track.sources().find(|source| source.sent(id))?;
            Some((source.default_priority, source.draining, track))
        });
        let Some((default_priority, draining, track)) = of_its_source else {
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
        // WHY not at once for the mode that leaves the Subgroup ID to the
        // first Object: until then the relay cannot tell whether it has taken
        // that Subgroup from another stream already.
        // §11.4.2: a header without a Publisher Priority has the one of the
        // subscription the stream came on, which is that of its source.
        //
        // WHY not where the source drains: the publisher may have sent
        // PUBLISH anew under the alias, with another default, and this
        // stream ahead of it (§5.1); it carries nothing that says which of
        // the two it is for. Taking the wrong default would end a track
        // that nothing is wrong with, for every subscriber. WHY NOT compare
        // all the same: what is lost is a difference that is not found,
        // which a relay that remembers 8 Groups does not promise anyway.
        let of_the_source = (!draining).then_some(default_priority);
        let priority = header.publisher_priority.or(of_the_source);
        let mut malformed = false;
        let passage = match subgroup_id {
            None => Passage::Undetermined,
            Some(id) => {
                let subgroup = (header.group_id, id);
                let key = (session, stream);
                let of = (header.track_alias, priority);
                match track.admit(subgroup, key, of, &mut self.out_streams, actions) {
                    Taking::Taken => Passage::Passed,
                    Taking::Left => Passage::Discarded,
                    Taking::Malformed => {
                        malformed = true;
                        Passage::Discarded
                    }
                }
            }
        };
        let mut in_stream = InStream {
            header,
            subgroup_id,
            priority,
            passage,
            started: false,
            of_the_subscription_before: false,
            outs: HashMap::new(),
        };
        // Where the header gives the Subgroup ID, every subscriber gets a
        // stream now, also for a Subgroup that turns out to hold no Object: a
        // header that is followed by a reset says that there is a Subgroup
        // the source will not send (§11.4.3).
        if passage == Passage::Passed {
            for subscriber in &mut track.subscribers {
                let out = OutStreamAllocator {
                    next: &mut self.next_out_stream,
                    out_streams: &mut self.out_streams,
                };
                out.open(&track_name, subscriber, &mut in_stream, header, actions);
            }
        }
        track.in_streams.insert((session, stream), in_stream);
        if malformed {
            // WHY stopped here: the track ends below, and what ends it stops
            // the streams it took Objects from, not one it had left.
            actions.push(Action::StopDataStream {
                session,
                stream,
                code: stream_reset_code::MALFORMED_TRACK,
            });
            self.on_track_malformed(session, stream, actions);
        }
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
        let key = (session, stream);
        let (group, of, known) = match track.in_streams.get(&key) {
            Some(in_stream) if in_stream.passage != Passage::Discarded => (
                in_stream.header.group_id,
                (in_stream.header.track_alias, in_stream.priority),
                in_stream.subgroup_id,
            ),
            _ => return,
        };
        // §11.4.2, mode 0b01: "the Subgroup ID is the Object ID of the first
        // Object transmitted in this Subgroup". The relay reads that as the
        // first Object on this stream, whatever FIRST_OBJECT says: it is the
        // one reading a subscriber can follow from the stream alone, and the
        // header goes downstream as it came, so both arrive at the same ID.
        let taking = if known.is_some() {
            Taking::Taken
        } else {
            let subgroup = (group, object.object_id);
            track.admit(subgroup, key, of, &mut self.out_streams, actions)
        };
        let Some(in_stream) = track.in_streams.get_mut(&key) else {
            return;
        };
        match taking {
            Taking::Taken => {}
            Taking::Left => {
                in_stream.passage = Passage::Discarded;
                return;
            }
            Taking::Malformed => {
                self.on_track_malformed(session, stream, actions);
                return;
            }
        }
        in_stream.passage = Passage::Passed;
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
        // The alias the stream came under: a session can have two sources
        // of the track, one of which drains.
        if let Some(in_stream) = track.in_streams.get(&(session, stream))
            && !in_stream.of_the_subscription_before
        {
            state
                .moqt
                .on_data_stream_closed(in_stream.header.track_alias);
        }
        track.in_stream_ended((session, stream), end, &mut self.out_streams, actions);
        self.retire_drained_sources(&track_name, actions);
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

    /// Lets go of every source of a track that said `PUBLISH_DONE` and for
    /// which nothing more can arrive: every stream its Stream Count named has
    /// ended (§10.11). The track ends with the last of them.
    fn retire_drained_sources(&mut self, track_name: &FullTrackName, actions: &mut Vec<Action>) {
        let Some(track) = self.tracks.live(track_name) else {
            return;
        };
        let drained: Vec<Source> = track
            .sources()
            .filter(|source| {
                let (session, track_alias) = source.session_and_alias();
                source.draining
                    && self.sessions.get(&session).is_none_or(|state| {
                        state.moqt.subscribed_alias(track_alias) == SubscribedAlias::Retired
                    })
            })
            .copied()
            .collect();
        self.source_gone(
            track_name,
            |source| drained.contains(source),
            TrackEnd::Done,
            actions,
        );
    }

    /// A source of a track is over and will send nothing more: it drained,
    /// it cancelled, its session is gone, the streams it counted did not
    /// come, or nobody wants what the relay asked it for. `is_gone` picks the
    /// source; this is the one place that tells the last source of a track
    /// from one of several. The track ends with its last source
    /// ([`Self::end_track`]); while it has another, it goes on, and only what
    /// the relay took from this one is cut ([`Self::let_go_of_sources`]).
    fn source_gone(
        &mut self,
        track_name: &FullTrackName,
        is_gone: impl Fn(&Source) -> bool,
        end: TrackEnd,
        actions: &mut Vec<Action>,
    ) {
        let Some(track) = self.tracks.live(track_name) else {
            return;
        };
        let (gone, kept): (Vec<Source>, Vec<Source>) =
            track.sources().partition(|source| is_gone(source));
        if gone.is_empty() {
            return;
        }
        if kept.is_empty() {
            self.end_track(track_name, end, actions);
        } else {
            self.let_go_of_sources(track_name, &gone, end, actions);
        }
    }

    /// Takes sources off a track that keeps others. Each is told that nothing
    /// more of it is wanted, as when its track ends. The Subgroups that were
    /// being taken from it end for the subscribers where they are: their
    /// streams are reset, and no other source goes on with them (see
    /// [`Subgroups`]). The subscriptions to the track are not touched.
    fn let_go_of_sources(
        &mut self,
        track_name: &FullTrackName,
        gone: &[Source],
        end: TrackEnd,
        actions: &mut Vec<Action>,
    ) {
        let Some(mut track) = self.tracks.take_live(track_name) else {
            return;
        };
        track.sources.retain(|source| !gone.contains(source));
        for source in gone {
            let (_, reset_code) = end.codes();
            self.release_source(&track, *source, end, reset_code, actions);
            let (session, _) = source.session_and_alias();
            // By stream number, for the reason `release_source` gives.
            let streams: BTreeSet<u64> = track
                .in_streams
                .iter()
                .filter(|(key, in_stream)| source.sent((key.0, in_stream.header.track_alias)))
                .map(|((_, stream), _)| stream.0)
                .collect();
            // For the track, the streams of a source that is let go end as
            // the streams of a publisher that is gone do.
            for stream in streams {
                let stream = (session, StreamKey(stream));
                track.in_stream_ended(stream, StreamEnd::Lost, &mut self.out_streams, actions);
            }
        }
        self.tracks.put_live(track_name, track);
    }

    /// Removes a track and ends every subscription to it.
    fn end_track(&mut self, track_name: &FullTrackName, end: TrackEnd, actions: &mut Vec<Action>) {
        let Some(mut track) = self.tracks.take_live(track_name) else {
            return;
        };
        let (status_code, reset_code) = end.codes();
        for source in track.sources() {
            // §2.4.2: "it MUST cancel any corresponding subscription or
            // fetches for that Track from that publisher". The track is over,
            // so the other sources are cancelled as well; they did nothing
            // wrong, and are not told that they did. That goes for another
            // source of the same session too, one that drains: what was
            // malformed came under the other alias.
            let to_the_source = match end {
                TrackEnd::Malformed { by } if !source.sent(by) => stream_reset_code::CANCELLED,
                _ => reset_code,
            };
            self.release_source(&track, *source, end, to_the_source, actions);
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
                .filter(|out_stream| {
                    matches!(end, TrackEnd::Malformed { .. }) || cut_short.contains(out_stream)
                })
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
        let streams: BTreeSet<(u64, bool)> = track
            .in_streams
            .iter()
            .filter(|(key, in_stream)| source.sent((key.0, in_stream.header.track_alias)))
            .map(|((_, stream), in_stream)| (stream.0, in_stream.passage == Passage::Discarded))
            .collect();
        for (stream, stopped_already) in streams {
            let stream = StreamKey(stream);
            state.in_streams.remove(&stream);
            if !stopped_already {
                actions.push(Action::StopDataStream {
                    session,
                    stream,
                    code: reset_code,
                });
            }
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
            TrackEnd::Malformed { .. } => {
                // §2.4.2: the receiver of a malformed track cancels the
                // subscription.
                if let Some(RequestSlot::Publish { request, .. }) =
                    state.requests.get(publish_stream)
                {
                    state.moqt.on_peer_request_closed(*request);
                }
                self.put_slot(
                    session,
                    publish_stream,
                    RequestSlot::Finished(None),
                    actions,
                );
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
            | TrackEnd::Malformed { .. }
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
        //
        // With sources of both kinds, the relay's SUBSCRIBEs are cancelled
        // and the track stays for its publishers. A SUBSCRIBE whose publisher
        // said PUBLISH_DONE is left to drain. A SUBSCRIBE that has no answer
        // yet is cancelled as well: it asks for what nobody would get.
        if track.subscribers.is_empty() {
            let unanswered = std::mem::take(&mut track.asks);
            for ask in unanswered {
                self.cancel_own_request(ask.publisher, ask.request, actions);
            }
            let unwanted = |source: &Source| {
                matches!(source.from, TrackSource::Subscription { .. }) && !source.draining
            };
            self.source_gone(track_name, unwanted, TrackEnd::Unwanted, actions);
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
            self.respond(subscriber.session, subscriber.stream, done, actions);
            actions.push(Action::FinishRequest {
                session: subscriber.session,
                stream: subscriber.stream,
            });
            // The subscription is over, and its Joining FETCHes hear of it.
            self.take_slot(subscriber.session, subscriber.stream, actions);
            return;
        }
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
        // The session keeps the request until the stream closes: what the
        // subscriber sent before it saw the PUBLISH_DONE still arrives. Put
        // last, as in the branch above: the Joining FETCHes of the
        // subscription hear of its end after the PUBLISH_DONE went out.
        self.put_slot(
            subscriber.session,
            subscriber.stream,
            RequestSlot::Finished(Some(subscriber.request)),
            actions,
        );
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
            let on_the_session = |source: &Source| source.session_and_alias().0 == session;
            self.source_gone(&track, on_the_session, TrackEnd::SourceGone, actions);
        }
        self.drop_routing_of(session, actions);
        self.drop_fetches_of(session, actions);
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
                    state
                        .requests
                        .iter()
                        .map(|(_, slot)| slot)
                        .filter(finished)
                        .count()
                })
                .sum(),
            in_streams: self
                .tracks
                .all_live()
                .map(|(_, track)| track.in_streams.len())
                .sum(),
            out_streams: self.out_streams.len(),
            sources: self
                .tracks
                .all_live()
                .map(|(_, track)| track.sources.len())
                .sum(),
            asks: self.tracks.all_asks().count(),
            declined: self
                .tracks
                .0
                .values()
                .map(|entry| match entry {
                    TrackEntry::Pending(pending) => pending.declined.len(),
                    TrackEntry::Live(track) => track.declined.len(),
                })
                .sum(),
            early_streams: self.sessions.values().map(|state| state.early.len()).sum(),
            remembered_subgroups: self
                .tracks
                .all_live()
                .map(|(_, track)| track.subgroups.taken.len())
                .sum(),
            namespaces: self.namespaces.len(),
            pending_tracks: self.tracks.all_pending().count(),
            own_requests: self.sessions.values().map(SessionState::own_requests).sum(),
            fetches: self.fetch_count(),
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
    /// A source of the track broke the object model (§2.4.2).
    Malformed { by: SourceId },
}

impl TrackEnd {
    /// The Status Code of the `PUBLISH_DONE` the subscribers get, and the
    /// code streams are reset and stopped with.
    fn codes(self) -> (u64, u32) {
        match self {
            Self::Done | Self::TimedOut | Self::SourceGone | Self::Unwanted => (
                publish_done_status::TRACK_ENDED,
                stream_reset_code::CANCELLED,
            ),
            Self::Malformed { .. } => (
                publish_done_status::MALFORMED_TRACK,
                stream_reset_code::MALFORMED_TRACK,
            ),
        }
    }
}

/// DEFAULT PUBLISHER PRIORITY (§12.4), a Track Property, and what holds
/// without it: "If omitted, the Default Publisher Priority is 128."
const DEFAULT_PUBLISHER_PRIORITY_PROPERTY: u64 = 0x0e;
const DEFAULT_PUBLISHER_PRIORITY: u8 = 128;

/// The Publisher Priority of the Subgroups of a subscription that carry
/// none, from the Track Properties of the PUBLISH or `SUBSCRIBE_OK` that
/// established it.
///
/// WHY a value above 255 counts as none: §12.4 has "Priorities above 255
/// are invalid" and does not say what a receiver does with one; the relay
/// passes the property on as it came and only has to compare with it. Of
/// two such properties in one message the first counts; nothing refuses a
/// second.
fn default_publisher_priority(properties: &[KeyValuePair]) -> u8 {
    properties
        .iter()
        .find(|property| property.ty() == DEFAULT_PUBLISHER_PRIORITY_PROPERTY)
        .and_then(|property| match property.value() {
            Value::Varint(priority) => u8::try_from(*priority).ok(),
            Value::Bytes(_) => None,
        })
        .unwrap_or(DEFAULT_PUBLISHER_PRIORITY)
}

/// The `LARGEST_OBJECT` among the parameters of a PUBLISH or a
/// `SUBSCRIBE_OK` (§10.2.11).
pub(crate) fn largest_object(parameters: &[Parameter]) -> Option<Location> {
    parameters.iter().find_map(|parameter| match parameter {
        Parameter::LargestObject(location) => Some(*location),
        _ => None,
    })
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
            | Self::StartJoiningFetchTimer { session, .. }
            | Self::StopRequest { session, .. }
            | Self::OpenRequest { session, .. }
            | Self::FinishOwnRequest { session, .. }
            | Self::CancelOwnRequest { session, .. }
            | Self::StartUpstreamTimer { session, .. }
            | Self::StartFetchStreamTimer { session, .. }
            | Self::StopUpstreamTimer { session, .. }
            | Self::PipeFetch { session, .. }
            | Self::AbortFetchPipe { session, .. }
            | Self::WriteFetchStream { session, .. } => Target::Session(*session),
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
