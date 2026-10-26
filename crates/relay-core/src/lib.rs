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
//! A subscriber gets the Objects from the Start Location of its Subscription
//! Filter on, and of an `AbsoluteRange` filter those up to its End Group
//! (§5.1.2; `routing::filter_start`, `routing::filter_end_group`).
//!
//! FIXME: enforce `OBJECT_DELIVERY_TIMEOUT`, and both delivery timeouts as a
//! Track Property (§8). Acted on is the `SUBGROUP_DELIVERY_TIMEOUT` of a
//! SUBSCRIBE and of a `REQUEST_UPDATE`; the rest is ignored.
//!
//! FIXME: share one encoding of an Object between subscribers. Each subscriber
//! gets its own copy of the payload today.

mod cache;
mod discovery;
mod events;
mod fetch;
mod own;
mod parent;
mod requests;
mod routing;
#[cfg(test)]
mod tests;
mod track_status;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

pub use cache::CachedObject;
pub use events::{
    Action, Event, FetchPiece, FetchPipeEnd, GoingAway, OutStreamKey, OwnRequestEnd, OwnRequestKey,
    RequestEnd, SendLimit, SessionKey, StreamEnd, StreamKey,
};
pub use fetch::{
    MAX_BYTES_HELD_BEYOND_THE_CACHE, MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER, fetch_reset_passed_on,
};
use own::OwnEnd;
use requests::{Requests, Subscribed};
pub use routing::{
    ANNOUNCEMENT_EVERY_MS, LimitsOfASession, MAX_ANNOUNCEMENTS_AT_ONCE,
    MAX_HELD_SUBSCRIBES_PER_SESSION, MAX_HELD_TRACKS, MAX_NAMESPACES_PER_SESSION,
    MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER, MAX_OWN_REQUESTS_PER_SESSION, MAX_PASSED_ON_REASON_LEN,
    MAX_RENDEZVOUS_MS, MAX_WAITING_SUBSCRIBERS,
};
use routing::{Announcement, Ask, Declined, OwnRequest, PendingTrack, Refusal, Waiter};
use xmoq_moqt::data::{ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject};
use xmoq_moqt::early::{self, Arrival, Settle};
use xmoq_moqt::immutable::{Of, searched_properties};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::message::{
    ControlMessage, Publish, ReasonPhrase, Subscribe, publish_done_status, request_error_code,
    setup_option, track_property,
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
/// not known yet: the PUBLISH or the `SUBSCRIBE_OK` travels on another stream
/// and may be read later. §11.4.2 lets a receiver "buffer it for a brief
/// period"; the I/O side runs that period ([`Action::StartEarlyTimer`]).
/// What is held is counted in bytes as a client counts it, up to
/// [`early::MOST_HELD_BYTES`] ([`early::arrival`]).
///
/// Provisional: the draft sets no limit, and `XmoQ` has not decided one.
///
/// WHY a limit of streams, one of Objects ([`MAX_EARLY_OBJECTS`]) and a time
/// on top of the bytes a client holds by: the relay is a shared place, and
/// what it takes in is not to be looser than what a client takes in.
pub const MAX_EARLY_STREAMS: usize = 8;

/// Objects held across the early streams of one session. Provisional, as
/// [`MAX_EARLY_STREAMS`].
pub const MAX_EARLY_OBJECTS: usize = 32;

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
///
/// 8 Groups are 8 Objects for a publisher that sends one Object a Group, as
/// the own clients do. Sources that do not number their Groups alike fall
/// under the rule of `Subgroups::admit`: the one with the smaller numbers
/// has all its streams stopped, and all it is told is `STOP_SENDING` with
/// `CANCELLED`. A publisher on another session that takes over a track at
/// Group 50 and counts from 0 has every stream stopped until it is at Group
/// 43, 7 below the newest the old source sent; so has one that joins a track
/// at Group 1000 and counts from 0, and a second publisher that starts with
/// larger numbers than the first silences the first. Two publishers of a
/// track have to agree on the Group IDs; nothing in the relay helps them to.
/// MSF has the same as a rule for the publisher (`draft-ietf-moq-msf-01`
/// §6.1): "Group IDs for a track MUST be unique and MUST increase
/// monotonically".
pub const MAX_REMEMBERED_GROUPS: u64 = 8;

/// Subgroups of one track the relay remembers at most, over all the Groups
/// above: one Group can have as many Subgroups as its publisher likes. Past
/// that the ones of the oldest Group are forgotten first, and within a Group
/// the ones with the smallest Subgroup ID, whether their stream is still open
/// or not. Provisional.
pub const MAX_REMEMBERED_SUBGROUPS: usize = 256;

/// Second streams of a source for a Subgroup it delivered whole that one
/// track reads at once ([`Passage::Checked`]). Past it such a stream is
/// stopped unread, and what it would have shown is not found.
/// **Provisional.** WHY a limit: such a stream is read for nothing but the
/// check, and a source could open as many as it likes.
pub const MAX_CHECKED_STREAMS_PER_TRACK: usize = 4;

/// How long such a stream is read at the most, in milliseconds; then it is
/// stopped, and what it would have shown is not found. **Provisional.**
pub const CHECKED_STREAM_WAIT_MS: u64 = 10_000;

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
///
/// Beyond it a PUBLISH gets `EXCESSIVE_LOAD`, a draining source counting:
/// §9.3 names no code, and it is the code of the relay's other limits. The
/// room of a track is counted when the relay asks
/// (`Relay::ask_the_publishers`), and a PUBLISH is accepted without a look
/// at what is asked, so a track can have sources and open requests that are
/// more than this together.
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
    /// A `TRACK_STATUS` the relay asked a publisher for (`track_status`).
    TrackStatus {
        request: PeerRequestId,
        peer_finished: bool,
    },
    /// A `SUBSCRIBE_NAMESPACE` the relay accepted, whose stream gets the
    /// NAMESPACE and `NAMESPACE_DONE` messages (`discovery`). A FIN of the
    /// peer ends it (§6.1).
    SubscribeNamespace { request: PeerRequestId },
    /// The relay has said its last on the stream. The request is given when the
    /// session still holds state for it: what the peer sends until the stream
    /// closes still goes to the session, which ignores it and keeps count of
    /// the Request IDs it uses up.
    Finished(Option<PeerRequestId>),
    /// The relay has stopped the peer's direction, after the time the peer
    /// had to end it. What was read before the stop took effect is dropped,
    /// and nothing more is done about the stream.
    Stopped,
}

impl RequestSlot {
    /// For the slot of a SUBSCRIBE: its request, and how far it has come.
    fn subscription(&self) -> Option<(PeerRequestId, Subscribed)> {
        match self {
            Self::Subscribe { request, .. } => Some((*request, Subscribed::Accepted)),
            Self::Waiting { request, .. } => Some((*request, Subscribed::Waiting)),
            Self::PublishNamespace { .. }
            | Self::SubscribeNamespace { .. }
            | Self::Fetch { .. }
            | Self::TrackStatus { .. }
            | Self::Publish { .. }
            | Self::Finished(_)
            | Self::Stopped => None,
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
            | Self::Fetch { peer_finished, .. }
            | Self::TrackStatus { peer_finished, .. } => *peer_finished,
            Self::Publish { .. }
            | Self::SubscribeNamespace { .. }
            | Self::Finished(_)
            | Self::Stopped => false,
        }
    }

    /// Notes that the peer finished its direction. `false` for a request
    /// that does not go on after that.
    fn note_peer_finished(&mut self) -> bool {
        match self {
            Self::Subscribe { peer_finished, .. }
            | Self::PublishNamespace { peer_finished, .. }
            | Self::Waiting { peer_finished, .. }
            | Self::Fetch { peer_finished, .. }
            | Self::TrackStatus { peer_finished, .. } => {
                *peer_finished = true;
                true
            }
            Self::Publish { .. }
            | Self::SubscribeNamespace { .. }
            | Self::Finished(_)
            | Self::Stopped => false,
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
    own_fetches: HashMap<OwnRequestKey, fetch::FetchAskedOfAPublisher>,
    /// `TRACK_STATUS` requests the relay opened on this session.
    own_track_statuses: HashMap<OwnRequestKey, track_status::OwnTrackStatus>,
    /// `PUBLISH_NAMESPACE` requests the session may still send now
    /// ([`MAX_ANNOUNCEMENTS_AT_ONCE`]).
    announcements_left: u32,
    /// The timer that gives one of them back runs.
    announcement_timer: bool,
}

impl SessionState {
    /// Requests the relay has open on this session that count towards
    /// [`MAX_OWN_REQUESTS_PER_SESSION`]: SUBSCRIBE, FETCH and `TRACK_STATUS`
    /// together; a `PUBLISH_NAMESPACE` to the parent does not
    /// (`own::Counted`).
    fn own_requests(&self) -> usize {
        self.own.len() + self.own_fetches.len() + self.own_track_statuses.len()
    }

    /// Whether the peer finished its direction of a request stream while the
    /// request goes on.
    fn peer_finished(&self, stream: StreamKey) -> bool {
        self.requests.peer_finished(stream)
    }

    /// Holds a stream whose Track Alias is not known yet, within
    /// [`MAX_EARLY_STREAMS`].
    fn hold_early_stream(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        header: SubgroupHeader,
        actions: &mut Vec<Action>,
    ) {
        if self.early.len() >= MAX_EARLY_STREAMS {
            actions.push(Action::StopDataStream {
                session,
                stream,
                code: stream_reset_code::INTERNAL_ERROR,
            });
        } else {
            self.early.push_back(EarlyStream {
                stream,
                header,
                objects: Vec::new(),
                end: None,
            });
            actions.push(Action::StartEarlyTimer { session, stream });
        }
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
            actions.push(Action::StopEarlyTimer {
                session,
                stream: early.stream,
            });
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
    /// The first Location the subscription passes (§5.1.2), counted from the
    /// same `LARGEST_OBJECT` where the filter is relative to it. `None`:
    /// nothing passes. "A publisher MUST NOT send objects from outside the
    /// requested range."
    start: Option<Location>,
    /// The last Group the subscription passes, of an `AbsoluteRange` filter;
    /// `None` for a subscription that is open ended.
    ///
    /// WHY the subscription goes on after it: §9.4 has "A subscription with
    /// a filter can reach a state where all possible Objects matching the
    /// filter have been delivered to the subscriber. Since tracking this
    /// can be prohibitively expensive, Relays are not required or expected
    /// to do so". A Group at or before the End Group can still come late.
    end_group: Option<u64>,
    /// The `SUBGROUP_DELIVERY_TIMEOUT` of the SUBSCRIBE, or of the last
    /// `REQUEST_UPDATE` that had one, in milliseconds (§8, §10.2.3, §10.9);
    /// `None` without one, and for 0. Handed to the I/O side with every
    /// stream of the subscription ([`Action::OpenDataStream`]), which acts
    /// on it.
    subgroup_delivery_timeout: Option<u64>,
    /// The `SUBSCRIBER_PRIORITY` of the SUBSCRIBE (§10.2.7); 128 without
    /// one. As the SUBSCRIBE had it: an update of it is refused.
    priority: u8,
    /// Subgroup streams the relay started for the subscription.
    streams_started: u64,
    /// Whether one of them may never have been opened, so that the number
    /// above is not known to be the number of streams the subscriber sees.
    count_unknown: bool,
    /// The streams of the subscription that are not done yet, in the order
    /// they were started.
    outs: BTreeSet<OutStreamKey>,
}

impl Subscriber {
    /// Whether the subscription passes the Object at `location` (§5.1.2).
    fn passes(&self, location: Location) -> bool {
        self.start.is_some_and(|start| location >= start)
            && self.end_group.is_none_or(|end| location.group <= end)
    }

    /// Whether it passes every Object the Group can have.
    fn passes_all_of(&self, group: u64) -> bool {
        self.passes(Location { group, object: 0 })
    }
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
    /// A second stream of the source for a Subgroup it delivered whole: read
    /// for the check of §2.4.2 alone ([`Subgroups`]), and nothing of it goes
    /// downstream or into the cache.
    Checked,
    /// Its Subgroup is taken from another stream, or was delivered whole
    /// already by another source, or it was not to be read for the check, or
    /// longer than it may be ([`MAX_CHECKED_STREAMS_PER_TRACK`],
    /// [`CHECKED_STREAM_WAIT_MS`]). It was stopped, and what still arrives on it is dropped; it
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
    /// The Object ID of the last Object read on it, a status among them.
    last_object: Option<u64>,
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
    /// The subscriptions, by session and Track Alias, that stopped the
    /// stream they got this one on: they get no other for it (§11.4.3,
    /// "SHOULD NOT attempt to open a new stream to deliver additional
    /// Objects in that Subgroup"). WHY here and not with the Subgroup: a
    /// Subgroup taken anew from another stream of its source is what the
    /// relay already lets reach a subscriber twice (§2.2, the gap on a
    /// second stream for a Subgroup); this covers the stream that goes on.
    /// WHY by Track Alias as well: a later subscription of the same session
    /// has another alias and asked for the track anew.
    stopped: HashSet<(SessionKey, u64)>,
    /// The Object whose first byte arrived and that is not whole yet
    /// ([`Event::ObjectBegun`]).
    receiving: Option<Receiving>,
}

/// An Object of a stream of a source that the relay is receiving.
#[derive(Debug)]
struct Receiving {
    object_id: u64,
    /// The subscriptions without a filter, by session and Track Alias, that
    /// were accepted while it was received: they do not get it.
    ///
    /// WHY: §5.1.2 has "Subscriptions without a filter pass all Objects
    /// published or received via upstream subscriptions", and the relay
    /// counts an Object as received from its first byte, as it does for the
    /// Largest Object in `SUBSCRIBE_OK` that a Joining FETCH ends at
    /// (§10.12.2.1: "contiguous and non-overlapping"). A subscriber that
    /// joins with a FETCH gets the Object from the FETCH, and would get it
    /// twice. What is lost: a subscriber without a filter that sends no
    /// FETCH does not get it, as it does not get an Object that was whole
    /// just before it came. No more than the subscriptions of the track.
    accepted_during: HashSet<(SessionKey, u64)>,
}

impl InStream {
    /// The header of a stream opened downstream for this one, whose
    /// Subgroup is `subgroup_id`, at the Object that is passed on now.
    ///
    /// A subscriber that came after the stream started gets a stream of its
    /// own from here on. Its header must not repeat two things that are only
    /// true of the Subgroup's first Object: the mode that derives the
    /// Subgroup ID from it, and `FIRST_OBJECT` (§11.4.2).
    fn header_downstream(&self, subgroup_id: u64) -> SubgroupHeader {
        if !self.started {
            return self.header;
        }
        SubgroupHeader {
            subgroup_id: SubgroupId::Explicit(subgroup_id),
            first_object: false,
            ..self.header
        }
    }

    /// Whether the subscriber has no stream for this one and may be given
    /// one: it did not stop the one it had ([`Self::stopped`]).
    fn lacks_a_stream_for(&self, subscriber: &Subscriber) -> bool {
        !self.outs.contains_key(&subscriber.session)
            && !self
                .stopped
                .contains(&(subscriber.session, subscriber.track_alias))
    }
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
    /// The stream, of that source, ended with a FIN: the Subgroup was
    /// delivered whole (§11.4.3).
    Whole(SourceId),
}

/// A Subgroup the relay has taken from a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Taken {
    from: TakenFrom,
    /// The Publisher Priority of that stream ([`InStream::priority`]).
    priority: Option<u8>,
    /// Once the stream ended with a FIN: the Object ID of the last Object
    /// on it, the final Object of the Subgroup (§2.4.2). `None` before, and
    /// where it had no Object.
    final_object: Option<u64>,
}

/// What became of a stream that arrived for a Subgroup of a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Taking {
    /// The Subgroup is taken from it.
    Taken,
    /// It was stopped: the Subgroup is taken from elsewhere.
    Left,
    /// It is read for the check alone ([`Passage::Checked`]).
    Checked,
    /// It makes the track malformed; ending the track is the caller's.
    Malformed,
}

impl Taking {
    /// What the stream is from now on. One that makes the track malformed
    /// is left: the track ends.
    const fn passage(self) -> Passage {
        match self {
            Self::Taken => Passage::Passed,
            Self::Checked => Passage::Checked,
            Self::Left | Self::Malformed => Passage::Discarded,
        }
    }
}

/// What to do with a stream that arrives for a Subgroup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admission {
    /// Take the Subgroup from it.
    Pass,
    /// Take the Subgroup from it, in place of this stream that is still open.
    Replace(InStreamKey),
    /// Leave it: the Subgroup was delivered whole by another source, is
    /// another source's, or is of a Group that is too old to tell.
    Discard,
    /// Read it for the check alone: the same source delivered the Subgroup
    /// whole.
    Check,
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
/// * After the first ended with a FIN: nothing of it goes downstream, and it
///   is read for the check below. The Subgroup was delivered whole (§11.4.3:
///   "An MOQT implementation that processes a stream FIN is assured it has
///   received all objects in a subgroup"), and a second stream for it
///   downstream is what §2.2 forbids. A first stream that was
///   finished without an Object counts the same: the relay passed the FIN
///   on, and §11.4.3 has "If a sender closes the stream before delivering all
///   such objects to the QUIC stream, it MUST reset the stream", so a later
///   stream that passed would make the FIN the relay sent untrue.
///
/// A stream of another source than the one the Subgroup was taken from is
/// left, and stopped unread, in all three cases: first come, first taken.
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
/// WHY `CANCELLED` for the stop: it is the code a subscriber that is no
/// longer interested sends, and the publisher cannot tell the two apart; of
/// the ten codes §3.3.3 lists, it is the one that fits. So a publisher's
/// subgroup streams are stopped as a matter of course whenever another
/// publisher of the track was first. What a publisher makes of the stop is
/// its own: the own Rust client reports it and goes on with the next Group;
/// in `@xmoq/moqt`, `write()` on that stream fails with `SubgroupClosedError`
/// and the publication goes on (as read in
/// `packages/moqt/src/webtransport/connection.ts`; not run with two
/// publishers). A publisher that takes a stopped stream for a failure of the
/// whole publication drops out as a source at its first Subgroup that
/// another one sent first: the Rust client of the `moqtail` spike does.
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
/// The priority of a stream is the one in its header or the source's
/// DEFAULT PUBLISHER PRIORITY, 128 without one ([`default_publisher_priority`]).
/// Two headers are enough: the priority in a header is that of every Object
/// of the stream, and the relay stops a second stream for a Subgroup when it
/// has read the header. Which of the two is wrong the relay cannot know; the
/// one that came first is taken as given, so a publisher that sends a
/// Subgroup with a priority of its own can end a track for everyone (known
/// gap on authorization in `docs/moqt-implementation-notes.md`). A publisher
/// that publishes anew under the same alias with another default, and sends
/// a Subgroup of the first subscription again without a priority in the
/// header, ends its own track; one that changes its default has to write the
/// priority into the headers of what it sends again. Two publishers of a
/// track have to agree on the DEFAULT PUBLISHER PRIORITY as they have to
/// agree on the Group IDs ([`MAX_REMEMBERED_GROUPS`]), or write the priority
/// into every header. The relay reads a default as how a header is written
/// short, so that a Subgroup has one priority across subscriptions (§9.1,
/// §7), and not as a number a publisher may choose anew for each
/// subscription (§12.4); §7.1 names a parameter the draft does not have, and
/// the relay reads the Track Property of §12.4. The clients do not compare.
///
/// A second stream of the same source for a Subgroup it delivered whole is
/// read, and nothing of it goes on ([`Passage::Checked`]): the track is
/// malformed where an Object on it is larger than the final Object, or it
/// ends with a FIN after another last Object (§2.4.2, the second and third
/// example: "An Object is received whose Object ID is larger than the final
/// Object in the Subgroup. The final Object in a Subgroup is the last Object
/// received on a Subgroup stream before a FIN", "A Subgroup is received over
/// multiple transport streams terminated by FIN with different final
/// Objects"). A first stream that ended with a FIN and no Object has no
/// final Object, and nothing is held against a stream after it. The final
/// Object is a status as well, which has an Object ID.
///
/// WHY the same source alone, and a stream of another source still stopped
/// unread (decided on 2026-10-25): the stop spares the relay the upstream of
/// a source that is there for redundancy, which reading every such stream to
/// its FIN would give up, for as many sources as a track has; and a
/// redundant source whose Subgroup ends one Object apart from the other's
/// would end the track for every subscriber, which is what a second source
/// is there to keep from happening. WHY NOT in the clients as well: the own
/// relay never sends a second stream after a FIN, so a client behind it never
/// sees either example; a client would find them only with a peer that is
/// not ours, and both `xmoq-moqt` and `@xmoq/moqt` would have to remember the
/// Subgroups of every subscription for it.
///
/// What is not found: a second stream whose header is read before the FIN
/// of the first takes the place of the first, as any second stream of the
/// same source while the first is open (above), and nothing is compared; a
/// stream past [`MAX_CHECKED_STREAMS_PER_TRACK`] or [`CHECKED_STREAM_WAIT_MS`];
/// and a Subgroup that is forgotten.
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
            Some(TakenFrom::Whole(by)) if by == source => return Admission::Check,
            // From another source.
            Some(TakenFrom::Open(..) | TakenFrom::CutShort(_) | TakenFrom::Whole(_)) => {
                return Admission::Discard;
            }
            None => Admission::Pass,
        };
        let from = TakenFrom::Open(stream, track_alias);
        self.taken.insert(
            (group, subgroup),
            Taken {
                from,
                priority,
                final_object: None,
            },
        );
        while let Some(((oldest, _), _)) = self.taken.first_key_value()
            && (forgotten(*oldest) || self.taken.len() > MAX_REMEMBERED_SUBGROUPS)
        {
            self.taken.pop_first();
        }
        admission
    }

    /// The stream a Subgroup is taken from ended, with a FIN or without;
    /// `last_object` is the Object ID of the last Object on it.
    fn ended(
        &mut self,
        (group, subgroup): (u64, u64),
        stream: InStreamKey,
        fin: bool,
        last_object: Option<u64>,
    ) {
        if let Some(taken) = self.taken.get_mut(&(group, subgroup))
            && let TakenFrom::Open(open, track_alias) = taken.from
            && open == stream
        {
            let source = (stream.0, track_alias);
            if fin {
                taken.from = TakenFrom::Whole(source);
                taken.final_object = last_object;
            } else {
                taken.from = TakenFrom::CutShort(source);
            }
        }
    }

    /// The final Object of a Subgroup a source delivered whole, where it is
    /// remembered and had an Object.
    fn final_object(&self, group: u64, subgroup: u64) -> Option<u64> {
        self.taken
            .get(&(group, subgroup))
            .filter(|taken| matches!(taken.from, TakenFrom::Whole(_)))
            .and_then(|taken| taken.final_object)
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
    /// Its PUBLISH or `SUBSCRIBE_OK` had `DYNAMIC_GROUPS` with the value 1
    /// (§12.6): it may be sent a `NEW_GROUP_REQUEST`.
    dynamic_groups: bool,
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
    /// Which of the tracks the relay has had this is, counted from 1.
    ///
    /// WHY: a name can have a track, lose it and have another, and the
    /// second may number its Groups as the first did (a publisher that
    /// begins at 0 again). What outlives a track, a fetch stream the relay
    /// still writes, tells by this which track's Group it carries.
    born: u32,
    /// The `NEW_GROUP_REQUEST` the relay sent a publisher of the track and
    /// that is outstanding: §10.2.13, "After sending a `NEW_GROUP_REQUEST`
    /// upstream, the request is considered outstanding until the Largest
    /// Group increases."
    new_group_asked: Option<u64>,
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
    /// Where an Object with the status `END_OF_TRACK` came, of any source,
    /// or the last Object of a FETCH whose answer said End of Track
    /// ([`Self::ends_at`]): §2.4.2 has "The final Object in a Track is the
    /// Object with Status `END_OF_TRACK` or the last Object sent in a FETCH
    /// whose response indicated End of Track", and an Object larger than it
    /// makes the track malformed. The Object ID is `u64::MAX` where the
    /// FETCH ended with a whole Group: its last Object is not known.
    final_object: Option<Location>,
    subscribers: Vec<Subscriber>,
    /// SUBSCRIBEs that came while every source of the track drained, and
    /// are held for a publisher to appear (§10.2.6): no more than
    /// [`MAX_WAITING_SUBSCRIBERS`]. They become subscribers when the track
    /// gets a source again, and go on waiting under the same name when the
    /// track is over. Empty for a track that does not drain.
    held: Vec<Waiter>,
    /// By the session and the stream they arrive on: stream numbers repeat
    /// from session to session.
    in_streams: HashMap<InStreamKey, InStream>,
    subgroups: Subgroups,
    /// The Objects of the latest Group, to answer a FETCH with.
    cache: cache::Cache,
    /// A source of the track, now or earlier, gave Track Properties with
    /// which its Objects may not be cached (`cache::may_be_cached`).
    ///
    /// WHY of the track, and never taken back: §12.3 has the limit for the
    /// Objects "received through this subscription", and `properties` above
    /// are those of the source that came last. To tell the Objects of one
    /// source from those of another the cache would have to know where each
    /// came from; keeping nothing of a track that had such a source keeps
    /// the MUST NOT without that.
    never_cached: bool,
}

/// An Object at `location` of a track was received, from its first byte on
/// (§5.1.2: "Largest Object updates when the first byte of an Object with a
/// Location larger than the previous value is published or received through
/// a subscription"): the largest Location of the track ([`Track::largest`])
/// is raised to it where it is larger. A later Group answers the
/// `NEW_GROUP_REQUEST` that was outstanding ([`Track::new_group_asked`],
/// §10.2.13).
///
/// WHY never lowered, also for an Object that does not arrive whole: the
/// draft has no rule that lowers it, the publisher has sent the Object, and
/// the value is the largest the relay has observed (§10.2.11), as one a
/// source says in its answer is, ahead of what the relay has.
///
/// WHY the fields of the track and not the track: a stream of the track is
/// held while this is done.
fn raise_largest(
    largest: &mut Option<Location>,
    new_group_asked: &mut Option<u64>,
    location: Location,
) {
    if largest.is_some_and(|largest| location <= largest) {
        return;
    }
    if largest.is_none_or(|largest| location.group > largest.group) {
        *new_group_asked = None;
    }
    *largest = Some(location);
}

/// Puts an Object of `in_stream` that the relay is about to pass on into the
/// cache of its track, unless the track has none (`never_cached`), and says
/// what the Object shows of the track there.
///
/// WHY the fields of the track and not the track: `in_stream` is one of
/// its fields as well.
fn keep(
    cache: &mut cache::Cache,
    never_cached: bool,
    in_stream: &InStream,
    subgroup_id: u64,
    object: &Arc<SubgroupObject>,
    cached_bytes: &mut usize,
) -> cache::Verdict {
    let group_id = in_stream.header.group_id;
    if never_cached {
        cache.clear(cached_bytes);
        return cache::Verdict::Fine;
    }
    let Some(publisher_priority) = in_stream.priority else {
        // A stream whose default priority the relay cannot tell (see
        // `Relay::on_data_stream_opened`) has no priority a fetch stream
        // could give.
        cache.leave_out(group_id, cached_bytes);
        return cache::Verdict::Fine;
    };
    let cached = CachedObject {
        group_id,
        subgroup_id,
        publisher_priority,
        object: Arc::clone(object),
    };
    // §11.4.2: see `Cache::insert`.
    let sure_of_subgroup =
        in_stream.header.subgroup_id != SubgroupId::FirstObjectId || in_stream.header.first_object;
    cache.insert(cached, sure_of_subgroup, cached_bytes)
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
                dynamic_groups: dynamic_groups(&properties),
            }],
            born: 0,
            new_group_asked: None,
            had_several_sources: false,
            asks: Vec::new(),
            declined: Declined::default(),
            never_cached: !cache::may_be_cached(&properties),
            properties,
            largest,
            final_object: None,
            subscribers: Vec::new(),
            held: Vec::new(),
            in_streams: HashMap::new(),
            subgroups: Subgroups::default(),
            cache: cache::Cache::default(),
        }
    }

    /// Whether an Object at `location` is past the end of the track, which
    /// makes it malformed; an Object with the status `END_OF_TRACK` that is not
    /// is the end from then on (§2.4.2, the fifth example: "An Object is received
    /// whose Group and Object ID are larger than the final Object in the
    /// Track. The final Object in a Track is the Object with Status
    /// `END_OF_TRACK`"), for `object` of `group`.
    ///
    /// WHY also an `END_OF_TRACK` below the largest Location known: the
    /// Objects that are larger are larger than the final one, whichever came
    /// first, and the streams of a track keep no order between them. The
    /// largest a source said in its answer counts, as one it said exists.
    /// The end a FETCH said is its End Location ([`Self::ends_at`]), not the
    /// last Object on its stream: the relay does not read into a fetch
    /// stream it passes on.
    fn past_its_end(&mut self, group: u64, object: &SubgroupObject) -> bool {
        let location = Location {
            group,
            object: object.object_id,
        };
        let end_of_track = object.status == ObjectStatus::EndOfTrack;
        let past = self.final_object.is_some_and(|end| location > end)
            || (end_of_track && self.largest.is_some_and(|largest| largest > location));
        if end_of_track && !past {
            self.final_object = Some(location);
        }
        past
    }

    /// Whether a Normal Object at `location` would make the track malformed
    /// by its Location alone: past the final Object of the track, after the
    /// End of Group of what the cache keeps, or in a gap a later Object
    /// announced (`cache::Cache::insert`). Nothing is decided by it: the
    /// track is found malformed where the Object is whole.
    fn malformed_at(&self, location: Location) -> bool {
        self.final_object.is_some_and(|end| location > end)
            || self.cache.refuses_an_object_at(location)
    }

    /// The Locations of the Objects the relay is receiving on the streams
    /// of the track it takes Subgroups from.
    fn receiving(&self) -> impl Iterator<Item = Location> + '_ {
        self.in_streams.values().filter_map(|in_stream| {
            let receiving = in_stream.receiving.as_ref()?;
            (in_stream.passage == Passage::Passed).then_some(Location {
                group: in_stream.header.group_id,
                object: receiving.object_id,
            })
        })
    }

    /// A subscription without a filter was accepted: it does not get the
    /// Objects the relay is receiving now ([`Receiving::accepted_during`]).
    fn accepted_without_a_filter(&mut self, subscription: (SessionKey, u64)) {
        for in_stream in self.in_streams.values_mut() {
            if let Some(receiving) = &mut in_stream.receiving {
                receiving.accepted_during.insert(subscription);
            }
        }
    }

    /// A `FETCH_OK` with End Of Track 1 said the track ends at `last`, its
    /// End Location less one (§10.13: "the End Location is the final Object
    /// in the Track"). Says whether that agrees with what is known; where it
    /// does, `last` is the final Object from now on, unless one is known.
    ///
    /// WHY a `FETCH_OK` that disagrees ends that FETCH alone, and leaves the
    /// track as it is (decided on 2026-10-24): which of the two sources is
    /// wrong is not known, and the Objects already passed on are not to be
    /// taken back. WHY NOT malformed, as §2.4.2 would have it for an Object
    /// larger than the final one: that would end every subscription of the
    /// track over one answer.
    fn ends_at(&mut self, last: Location) -> bool {
        let whole_group = last.object == u64::MAX;
        let agrees = self.largest.is_none_or(|largest| largest <= last)
            && self
                .final_object
                .is_none_or(|known| known == last || (whole_group && known.group == last.group));
        if agrees && self.final_object.is_none() {
            self.final_object = Some(last);
        }
        agrees
    }

    /// What becomes of the stream `key` for `object`, which came whole on it:
    /// `None` for a stream that was stopped (`Passage::Discarded`).
    ///
    /// §11.4.2, mode 0b01: "the Subgroup ID is the Object ID of the first
    /// Object transmitted in this Subgroup". The relay reads that as the
    /// first Object on this stream, whatever `FIRST_OBJECT` says: it is the
    /// one reading a subscriber can follow from the stream alone, and the
    /// header goes downstream as it came, so both arrive at the same ID.
    fn taking_of_whole(
        &mut self,
        key: InStreamKey,
        object: &SubgroupObject,
        out_streams: &mut HashMap<OutStreamKey, OutStream>,
        actions: &mut Vec<Action>,
    ) -> Option<Taking> {
        let (group, of, known, checked) = match self.in_streams.get(&key) {
            Some(in_stream) if in_stream.passage != Passage::Discarded => (
                in_stream.header.group_id,
                (in_stream.header.track_alias, in_stream.priority),
                in_stream.subgroup_id,
                in_stream.passage == Passage::Checked,
            ),
            _ => return None,
        };
        Some(if checked {
            Taking::Checked
        } else if self.past_its_end(group, object) {
            Taking::Malformed
        } else if known.is_some() {
            Taking::Taken
        } else {
            self.admit((group, object.object_id), key, of, out_streams, actions)
        })
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
            Admission::Check if self.checked_streams() < MAX_CHECKED_STREAMS_PER_TRACK => {
                let (session, stream) = stream;
                actions.push(Action::StartCheckTimer {
                    session,
                    stream,
                    wait_ms: CHECKED_STREAM_WAIT_MS,
                });
                Taking::Checked
            }
            Admission::Discard | Admission::Check => {
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

    /// The streams read for the check alone ([`Passage::Checked`]).
    fn checked_streams(&self) -> usize {
        self.in_streams
            .values()
            .filter(|in_stream| in_stream.passage == Passage::Checked)
            .count()
    }

    /// Whether a stream read for the check alone that ended with a FIN
    /// makes the track malformed: its last Object is another than the final
    /// Object of the Subgroup (§2.4.2, the third example). One that is
    /// larger was found when it came ([`Relay::on_object`]).
    fn ends_apart(&self, stream: InStreamKey, end: StreamEnd) -> bool {
        let Some(in_stream) = self.in_streams.get(&stream) else {
            return false;
        };
        let (Passage::Checked, StreamEnd::Fin, Some(subgroup), Some(last)) = (
            in_stream.passage,
            end,
            in_stream.subgroup_id,
            in_stream.last_object,
        ) else {
            return false;
        };
        self.subgroups
            .final_object(in_stream.header.group_id, subgroup)
            .is_some_and(|final_object| final_object != last)
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
            let subgroup = (in_stream.header.group_id, subgroup_id);
            let fin = end == StreamEnd::Fin;
            self.subgroups
                .ended(subgroup, stream, fin, in_stream.last_object);
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
    // Boxed: a track with a source is several times the size of one without.
    Live(Box<Track>),
}

/// Every track the relay knows, by name.
///
/// WHY one map for the tracks that wait for a publisher's answer and the ones
/// that have a source: a name is in one of the states or in none, and two
/// maps would leave that to every place that writes to them.
#[derive(Debug, Default)]
struct Tracks(
    HashMap<FullTrackName, TrackEntry>,
    /// How many tracks with a source there have been: each has the number
    /// it was the how-manieth ([`Track::born`]).
    u32,
);

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
    /// What a publisher that says `PUBLISH_DONE` and publishes anew at once
    /// gets depends on a race: where every stream it counted has ended, its
    /// source is over and the PUBLISH makes a new track; where one is still
    /// under way, the PUBLISH is accepted for the track as it is; and where
    /// the relay reads the PUBLISH before the `PUBLISH_DONE`, which travels on
    /// another stream, it is refused with `DUPLICATE_SUBSCRIPTION` (seen once
    /// in CI, run 38076391019 of PR #48). The refusal says not to try again
    /// (Retry Interval 0; known gap on the Retry Interval in
    /// `docs/moqt-implementation-notes.md`), though the same PUBLISH a moment
    /// later is accepted.
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
                dynamic_groups: dynamic_groups(&properties),
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
            track.never_cached |= !cache::may_be_cached(&properties);
            track.properties = properties;
            track.largest = track.largest.max(largest);
            // WHY forgotten: the new source was not asked, and a request of
            // 0 would else not reach it until it passes the Largest Group of
            // the sources before (§10.2.13 has a request outstanding "until
            // the Largest Group increases").
            track.new_group_asked = None;
            // The publisher they were held for has come.
            return Ok(Added {
                waiters: std::mem::take(&mut track.held),
                replaced,
            });
        }
        let mut track = Track::established(from, properties, largest);
        // WHY not past the largest: 4 billion tracks would take a relay
        // centuries, and a number used twice is better than a panic.
        self.1 = self.1.saturating_add(1);
        track.born = self.1;
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
        self.0
            .insert(name.clone(), TrackEntry::Live(Box::new(track)));
        Ok(Added {
            waiters,
            replaced: None,
        })
    }

    /// Puts back a track that [`Self::take_live`] took out.
    fn put_live(&mut self, name: &FullTrackName, track: Track) {
        self.0
            .insert(name.clone(), TrackEntry::Live(Box::new(track)));
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
            TrackEntry::Live(track) => Some(*track),
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
    /// it for a publisher that sent PUBLISH, and nobody wants more of it;
    /// the publisher that is asked would send it to the relay all the same.
    /// §9.5's "a namespace that matches one or more existing subscriptions
    /// to other upstream sessions" is read as being about tracks somebody is
    /// subscribed to (known gap on several publishers of one track).
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
            TrackEntry::Live(track) => Some((name, &**track)),
            TrackEntry::Pending(_) => None,
        })
    }

    /// The SUBSCRIBEs that wait under the name: for an answer of a
    /// publisher, for a publisher to appear, or, where every source of the
    /// track drains, for one to come back.
    fn waiters(&self, name: &FullTrackName) -> &[Waiter] {
        match self.0.get(name) {
            Some(TrackEntry::Pending(pending)) => &pending.waiters,
            Some(TrackEntry::Live(track)) => &track.held,
            None => &[],
        }
    }

    /// How many names have a SUBSCRIBE that is held for a publisher to
    /// appear.
    fn held(&self) -> usize {
        self.0
            .values()
            .filter(|entry| {
                let waiters = match entry {
                    TrackEntry::Pending(pending) => &pending.waiters,
                    TrackEntry::Live(track) => &track.held,
                };
                waiters.iter().any(Waiter::is_held)
            })
            .count()
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
    /// and the ones it has answered and that are not reported closed. More
    /// than `requests` by the streams whose request no session holds: one
    /// refused because that many were held, one a session let go of when
    /// the peer's time to end the stream was over.
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
    /// Bytes the caches of the tracks hold, over all tracks.
    pub cached_bytes: usize,
    /// Bytes the fetch streams the relay writes keep alive of Groups the
    /// caches have let go of ([`MAX_BYTES_HELD_BEYOND_THE_CACHE`]).
    pub held_beyond_cache_bytes: usize,
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
    /// `TRACK_STATUS` requests of subscribers asked of a publisher.
    pub track_statuses: usize,
    /// Request streams of FETCH requests that were served, which the relay
    /// keeps its direction of open until the subscriber has ended its own,
    /// or the time for that is over.
    pub fetch_ends: usize,
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
    /// The `SUBSCRIBE_NAMESPACE` requests the relay accepted (`discovery`).
    namespace_subscriptions: Vec<discovery::NamespaceSubscription>,
    /// FETCH requests of subscribers.
    fetches: fetch::Fetches,
    /// Of those, the ones held for an Object the relay is receiving
    /// (`fetch::ReceiptWait`), with the range they wait on: one entry for
    /// each such FETCH, so no more than the FETCH requests there are.
    awaiting_receipt: BTreeMap<(SessionKey, StreamKey), fetch::AwaitedRange>,
    /// The subscribers' `TRACK_STATUS` requests asked of a publisher.
    track_statuses: track_status::TrackStatuses,
    /// Bytes the caches of all tracks hold ([`cache::MAX_CACHED_BYTES`]).
    cached_bytes: usize,
    /// The request streams of FETCH requests that were served to the end of
    /// their fetch stream, and that the relay finishes when the subscriber
    /// has ended its direction, or in time (`Relay::end_answered_fetch`).
    fetch_ends: BTreeSet<(SessionKey, StreamKey)>,
    next_drain_wait: u64,
    next_own_request: u64,
    /// Says whether a session and a request are allowed
    /// ([`Self::set_authorizer`]). None: all of them are.
    authorizer: Option<Authorization>,
    /// The New Session URI and the Timeout of the GOAWAY every session gets,
    /// once the relay is to stop ([`Event::GoingAway`]).
    going_away: Option<GoingAway>,
    /// [`Self::set_announcements`].
    announcements_at_once: u32,
    /// [`Self::set_announcements`].
    announcement_every_ms: u64,
    /// [`Self::set_limits_of_a_session`].
    limits_of_a_session: LimitsOfASession,
    /// [`Self::set_share_of_one_subscriber`].
    requests_for_one: usize,
    /// [`Self::set_share_of_one_subscriber`].
    fetches_for_one: usize,
    /// [`Self::set_held_beyond_the_cache`].
    held_beyond_cache_limit: usize,
    /// The cache of a track let go of a Group, or a track ended, while
    /// this event was handled: what the fetch streams keep of it is looked
    /// at before the event is done with.
    cache_let_go: bool,
    /// [`Self::set_upstream_answer_wait_ms`].
    upstream_answer_wait_ms: u64,
    /// [`Self::set_parent`].
    parent: Option<parent::Parent>,
}

/// How long the I/O side waits for a publisher's answer to a request of the
/// relay, where it is not told otherwise
/// ([`Relay::set_upstream_answer_wait_ms`]). Provisional.
pub const UPSTREAM_ANSWER_WAIT_MS: u64 = 5_000;

/// What a request asks to be allowed to do with a namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// PUBLISH and `PUBLISH_NAMESPACE`.
    Publish,
    /// SUBSCRIBE, `TRACK_STATUS` and FETCH.
    Subscribe,
    /// `SUBSCRIBE_NAMESPACE`, of the prefix it names. Apart from
    /// [`Self::Subscribe`]: it tells of every namespace under the prefix,
    /// and the authorization may ask more of it.
    SubscribeNamespace,
}

/// Why something is not allowed, and what ends for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denied {
    /// The request is refused, with this code of `REQUEST_ERROR` (§10.6).
    Request(u64),
    /// The session is closed, with this code (§3.5).
    Session(TerminationCode),
}

/// Decides whether a session and its requests are allowed, from the
/// AUTHORIZATION TOKENs they carry (§10.2.2, §10.3.1.4).
///
/// WHY handed in: checking a token takes a key and a clock, and this crate
/// has neither.
pub trait Authorizer: Send {
    /// A request: what it asks, the namespace it asks it of, and its
    /// parameters. A Joining FETCH names no namespace, and has `None`: it
    /// is for a SUBSCRIBE of its session, which was asked about with one.
    ///
    /// # Errors
    ///
    /// [`Denied`], with what ends for it.
    fn request(
        &self,
        operation: Operation,
        namespace: Option<&TrackNamespace>,
        parameters: &[Parameter],
    ) -> Result<(), Denied>;

    /// The Setup Options of a peer's SETUP.
    ///
    /// # Errors
    ///
    /// The code the session is closed with.
    fn setup(&self, _setup_options: &[KeyValuePair]) -> Result<(), TerminationCode> {
        Ok(())
    }

    /// A token of the relay's own for `operation` in every namespace, as
    /// the value of an AUTHORIZATION TOKEN (a Token structure, §10.2.2),
    /// for what it sends the parent relay: its SETUP and its requests.
    /// `None` where the relay has none to give.
    fn own_token(&self, _operation: Operation) -> Option<Vec<u8>> {
        None
    }
}

struct Authorization(Box<dyn Authorizer>);

impl std::fmt::Debug for Authorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Authorization")
    }
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
            namespace_subscriptions: Vec::new(),
            fetches: fetch::Fetches::default(),
            track_statuses: track_status::TrackStatuses::default(),
            awaiting_receipt: BTreeMap::new(),
            cached_bytes: 0,
            fetch_ends: BTreeSet::new(),
            next_drain_wait: 0,
            next_own_request: 0,
            authorizer: None,
            going_away: None,
            announcements_at_once: MAX_ANNOUNCEMENTS_AT_ONCE,
            announcement_every_ms: ANNOUNCEMENT_EVERY_MS,
            limits_of_a_session: LimitsOfASession::default(),
            requests_for_one: MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER,
            fetches_for_one: MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER,
            held_beyond_cache_limit: fetch::MAX_BYTES_HELD_BEYOND_THE_CACHE,
            cache_let_go: false,
            upstream_answer_wait_ms: UPSTREAM_ANSWER_WAIT_MS,
            parent: None,
        }
    }

    /// How long the I/O side waits for a publisher's answer to a request of
    /// the relay ([`Action::StartUpstreamTimer`]), in place of
    /// [`UPSTREAM_ANSWER_WAIT_MS`]: what a FETCH that was not answered in
    /// time has used of the time its subscriber waits.
    pub fn set_upstream_answer_wait_ms(&mut self, wait_ms: u64) {
        self.upstream_answer_wait_ms = wait_ms;
    }

    /// How many `PUBLISH_NAMESPACE` requests a session may send at once, and
    /// after how many milliseconds it has one back, in place of
    /// [`MAX_ANNOUNCEMENTS_AT_ONCE`] and [`ANNOUNCEMENT_EVERY_MS`]. For the
    /// sessions that come after the call.
    pub fn set_announcements(&mut self, at_once: u32, every_ms: u64) {
        self.announcements_at_once = at_once;
        self.announcement_every_ms = every_ms;
    }

    /// What one session may have, in place of the relay's own numbers
    /// ([`LimitsOfASession`]). For the sessions that open after it.
    pub fn set_limits_of_a_session(&mut self, limits: LimitsOfASession) {
        self.limits_of_a_session = limits;
    }

    /// How many SUBSCRIBE requests, and how many FETCH requests beside them,
    /// the relay has open on one publisher for one subscriber's session, in
    /// place of
    /// [`MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER`] and
    /// [`MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER`].
    pub fn set_share_of_one_subscriber(&mut self, requests: usize, fetches: usize) {
        self.requests_for_one = requests;
        self.fetches_for_one = fetches;
    }

    /// How many bytes the fetch streams the relay writes may keep alive
    /// beyond its cache, in place of
    /// [`fetch::MAX_BYTES_HELD_BEYOND_THE_CACHE`]; one subscriber's session
    /// has an eighth of them (`SHARE_OF_BYTES_HELD_BEYOND_THE_CACHE`).
    pub fn set_held_beyond_the_cache(&mut self, bytes: usize) {
        self.held_beyond_cache_limit = bytes;
    }

    /// A session sends a `PUBLISH_NAMESPACE`: whether it may, by how many
    /// it has sent of late.
    fn takes_an_announcement(&mut self, session: SessionKey, actions: &mut Vec<Action>) -> bool {
        let Some(state) = self.sessions.get_mut(&session) else {
            return false;
        };
        if state.announcements_left == 0 {
            return false;
        }
        state.announcements_left -= 1;
        if !state.announcement_timer {
            state.announcement_timer = true;
            actions.push(Action::StartAnnouncementTimer {
                session,
                wait_ms: self.announcement_every_ms,
            });
        }
        true
    }

    /// The time for one announcement is over: the session has one more.
    fn on_announcement_timeout(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        let at_once = self.announcements_at_once;
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        state.announcements_left = state.announcements_left.saturating_add(1).min(at_once);
        // WHY one timer at a time: with one for each announcement, all that
        // were used at once would come back at once, and the rate would be
        // that of the burst.
        state.announcement_timer = state.announcements_left < at_once;
        if state.announcement_timer {
            actions.push(Action::StartAnnouncementTimer {
                session,
                wait_ms: self.announcement_every_ms,
            });
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
        if std::mem::take(&mut self.cache_let_go) {
            self.trim_what_is_held_beyond_the_cache(&mut actions);
        }
        actions
    }

    fn handle_event(&mut self, event: Event) -> Vec<Action> {
        let mut actions = Vec::new();
        // What has to do with a FETCH the relay passes on goes its own way.
        let Some(event) = self.on_fetch_event(event, &mut actions) else {
            return actions;
        };
        let Some(event) = self.on_track_status_event(event, &mut actions) else {
            return actions;
        };
        let Some(event) = self.on_announcement_event(event, &mut actions) else {
            return actions;
        };
        let Some(event) = self.on_data_event(event, &mut actions) else {
            return actions;
        };
        match event {
            Event::SessionOpened { session, role } => {
                self.open_session(session, role, &mut actions);
            }
            Event::ControlMessage { session, message } => {
                self.on_control_message(session, message, &mut actions);
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
            } => self.on_peer_request_stream_ended(session, stream, how, &mut actions),
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
            // Taken by `on_fetch_event` and `on_data_event` above.
            Event::FetchStreamOpened { .. }
            | Event::FetchPipeEnded { .. }
            | Event::JoiningFetchTimeout { .. }
            | Event::ReceiptTimeout { .. }
            | Event::FetchEndTimeout { .. }
            | Event::FetchStreamWritten { .. }
            | Event::DataStreamOpened { .. }
            | Event::ObjectBegun { .. }
            | Event::Object { .. }
            | Event::DataStreamClosed { .. }
            | Event::TrackMalformed { .. }
            | Event::EarlyStreamTimeout { .. }
            | Event::CheckStreamTimeout { .. }
            | Event::SendQueueFull { .. } => {}
            Event::OutStreamOpened { out_stream } => self.out_stream_opened(out_stream),
            Event::OutStreamStopped { out_stream } => self.out_stream_stopped(out_stream),
            Event::OutStreamClosed { out_stream } => self.forget_out_stream(out_stream),
            Event::DrainTimeout {
                session,
                track_alias,
                wait,
            } => self.on_drain_timeout(session, track_alias, wait, &mut actions),
            Event::NamespaceWritten { session, stream } => {
                self.on_namespace_written(session, stream, &mut actions);
            }
            Event::RequestTimeout { session, stream } => {
                self.on_request_timeout(session, stream, &mut actions);
            }
            Event::AnnouncementTimeout { session } => {
                self.on_announcement_timeout(session, &mut actions);
            }
            Event::ParentAnnouncementTimeout { session } => {
                self.on_parent_announcement_timeout(session, &mut actions);
            }
            Event::RendezvousTimeout { session, stream } => {
                self.on_rendezvous_timeout(session, stream, &mut actions);
            }
            Event::SessionFailed { session, error } => {
                self.fail_session(session, &error, &mut actions);
            }
            Event::SessionClosed { session } => self.drop_session(session, &mut actions),
            Event::ParentFailed => self.parent_failed(&mut actions),
            Event::GoingAway(going_away) => self.on_going_away(going_away, &mut actions),
            Event::SendAway {
                session,
                going_away,
            } => self.send_away(session, going_away, &mut actions),
        }
        actions
    }

    /// The events of the subgroup streams of the sources: taken here, or
    /// handed back for the others.
    fn on_data_event(&mut self, event: Event, actions: &mut Vec<Action>) -> Option<Event> {
        match event {
            Event::DataStreamOpened {
                session,
                stream,
                header,
            } => self.on_data_stream_opened(session, stream, header, actions),
            Event::ObjectBegun {
                session,
                stream,
                object_id,
            } => self.on_object_begun(session, stream, object_id, actions),
            Event::Object {
                session,
                stream,
                object,
            } => self.on_object(session, stream, object, actions),
            Event::DataStreamClosed {
                session,
                stream,
                end,
            } => self.on_data_stream_closed(session, stream, end, actions),
            Event::TrackMalformed { session, stream } => {
                self.on_track_malformed(session, stream, actions);
            }
            Event::EarlyStreamTimeout { session, stream } => {
                self.on_early_stream_timeout(session, stream, actions);
            }
            Event::CheckStreamTimeout { session, stream } => {
                self.on_check_timeout(session, stream, actions);
            }
            Event::SendQueueFull {
                out_stream,
                limit,
                held,
            } => self.on_send_queue_full(out_stream, limit, &held, actions),
            other => return Some(other),
        }
        None
    }

    fn out_stream_opened(&mut self, out_stream: OutStreamKey) {
        if let Some(out) = self.out_streams.get_mut(&out_stream) {
            out.opened = true;
        }
    }

    /// What waits to be written for `out_stream` is over `limit`,
    /// and `held` is what each stream holds by its measure ([`SendLimit`]). One
    /// subscription ends:
    ///
    /// - [`SendLimit::Subscription`]: that of `out_stream`.
    /// - [`SendLimit::Session`]: of the subscriber's session, the one whose
    ///   streams carried the most bytes; that of `out_stream` where none
    ///   carried more.
    /// - [`SendLimit::Relay`]: of any session, the one whose streams have the
    ///   most bytes waiting; the first in `held` where two have as much.
    ///   `out_stream` plays no part: it may be gone already, ended by a
    ///   limit of the session that the same Object reached first.
    ///
    /// WHY end one subscription: its queue is the only place the delay can
    /// go without holding back the publisher and the other subscribers (§8
    /// names `TOO_FAR_BEHIND` for it). WHY the one that carried the most of
    /// the session, and not the one that went over: the limit is the
    /// session's, and the one that went over is as often a small track that
    /// comes often, the sound behind its own video, as the track that fills
    /// it; a viewer behind a link of 2 Mbit/s lost its sound so in one run
    /// of two
    /// (docs/experiments/2026-10-14-three-minutes-behind-a-narrow-link-and-the-relays-limit-for-one-viewer.md).
    /// WHY bytes and not streams: a sound of one piece a stream holds more
    /// streams than its video. WHY the bytes waiting for the relay's limit,
    /// of any session: a subscriber that keeps up has carried the most,
    /// having been sent the most, and what ending one frees of this limit is
    /// what waits.
    ///
    /// Where another subscription ends at the limit of a session,
    /// `out_stream` itself is reset with `TOO_FAR_BEHIND`: the Object that
    /// did not fit was not queued, and a stream without it would carry a
    /// Subgroup with a hole. At the relay's limit the Object was queued
    /// (`HubConfig::send_for_the_relay_bytes`: the Object that goes over is
    /// as often one of a subscriber that keeps up), and nothing else is
    /// reset.
    fn on_send_queue_full(
        &mut self,
        out_stream: OutStreamKey,
        limit: SendLimit,
        held: &[(OutStreamKey, usize)],
        actions: &mut Vec<Action>,
    ) {
        let own = self
            .out_streams
            .get(&out_stream)
            .map(|out| (out.session, out.track.clone()));
        let ended = match limit {
            SendLimit::Relay => self.most_held(held, |_| true).map(|(key, _)| key),
            SendLimit::Subscription => own,
            // The subscription of `out_stream` where none of the session
            // carried more.
            SendLimit::Session => own.map(|own| {
                let session = own.0;
                match self.most_held(held, |of| of == session) {
                    Some((key, bytes)) if bytes > self.held_by(held, &own) => key,
                    _ => own,
                }
            }),
        };
        let Some(ended) = ended else {
            return;
        };
        let own_stream_has_a_hole = limit == SendLimit::Session
            && self
                .out_streams
                .get(&out_stream)
                .is_some_and(|out| (out.session, &out.track) != (ended.0, &ended.1));
        if own_stream_has_a_hole {
            self.reset_for_a_hole(out_stream, actions);
        }
        let (session, track) = ended;
        self.end_subscription(
            &track,
            session,
            publish_done_status::TOO_FAR_BEHIND,
            stream_reset_code::TOO_FAR_BEHIND,
            actions,
        );
    }

    /// Of the subscriptions of the streams in `held` whose session `counts`,
    /// the one that holds the most, summed over its streams, with what it
    /// holds; the first of them where two hold as much.
    fn most_held(
        &self,
        held: &[(OutStreamKey, usize)],
        counts: impl Fn(SessionKey) -> bool,
    ) -> Option<((SessionKey, FullTrackName), usize)> {
        let mut by_subscription: HashMap<(SessionKey, FullTrackName), (usize, usize)> =
            HashMap::new();
        for (order, (stream, bytes)) in held.iter().enumerate() {
            let Some(out) = self.out_streams.get(stream) else {
                continue;
            };
            if !counts(out.session) {
                continue;
            }
            let entry = by_subscription
                .entry((out.session, out.track.clone()))
                .or_insert((order, 0));
            entry.1 = entry.1.saturating_add(*bytes);
        }
        by_subscription
            .into_iter()
            .max_by(|(_, (a_order, a)), (_, (b_order, b))| a.cmp(b).then(b_order.cmp(a_order)))
            .map(|(key, (_, bytes))| (key, bytes))
    }

    /// What the streams of the subscription `of` hold in `held`.
    fn held_by(&self, held: &[(OutStreamKey, usize)], of: &(SessionKey, FullTrackName)) -> usize {
        held.iter()
            .filter(|(stream, _)| {
                self.out_streams
                    .get(stream)
                    .is_some_and(|out| (out.session, &out.track) == (of.0, &of.1))
            })
            .map(|(_, bytes)| *bytes)
            .fold(0, usize::saturating_add)
    }

    /// `out_stream`'s Object did not fit, and another subscription ends: the
    /// stream is reset with `TOO_FAR_BEHIND`, which would carry a Subgroup
    /// with a hole.
    fn reset_for_a_hole(&mut self, out_stream: OutStreamKey, actions: &mut Vec<Action>) {
        let Some(out) = self.out_streams.get(&out_stream) else {
            return;
        };
        let (subscriber, full_track) = (out.session, out.track.clone());
        let subscriber_entry = self.tracks.live_mut(&full_track).and_then(|track| {
            track
                .subscribers
                .iter_mut()
                .find(|entry| entry.session == subscriber)
        });
        reset_out_stream(
            &mut self.out_streams,
            subscriber_entry,
            out_stream,
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
        self.track_malformed_by(&track, (session, stream), actions);
    }

    /// A stream of a track showed it to be malformed: as
    /// [`Self::on_track_malformed`], for a stream the session may have let
    /// go of already.
    fn track_malformed_by(
        &mut self,
        track: &FullTrackName,
        (session, stream): InStreamKey,
        actions: &mut Vec<Action>,
    ) {
        let track = track.clone();
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

    /// Lets go of a request the relay said its last on, with `PUBLISH_DONE`,
    /// a refusal or the end of a FETCH, and whose stream the peer has not
    /// closed in time: the peer's direction is stopped. Also for a request
    /// the session never held (refused because that many were held).
    fn on_request_timeout(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(RequestSlot::Finished(request)) = state.requests.get(stream) else {
            return;
        };
        if let Some(request) = *request {
            state.moqt.on_peer_request_closed(request);
        }
        // The slot stays until the stream really closes: the stream counts
        // as one of the peer's until then. A second timeout finds nothing to
        // do.
        self.put_slot(session, stream, RequestSlot::Stopped, actions);
        // The relay's direction was finished when it said its last; this
        // ends the peer's.
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
        moqt.set_most_peer_requests(self.limits_of_a_session.peer_requests);
        let option = KeyValuePair::bytes(
            setup_option::MOQT_IMPLEMENTATION,
            self.implementation.as_bytes().to_vec(),
        );
        // An implementation name too long for a Setup Option is left out; the
        // option is a SHOULD (§10.3.1.5).
        let mut options = option.map(|option| vec![option]).unwrap_or_default();
        if role == Role::Client {
            options.extend(self.setup_token());
        }
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
                own_track_statuses: HashMap::new(),
                announcements_left: self.announcements_at_once,
                announcement_timer: false,
            },
        );
        // The relay is the client of none but the parent relay
        // ([`Action::ConnectParent`]).
        if role == Role::Client {
            self.parent_opened(session, actions);
        }
        self.go_away(session, actions);
    }

    /// [`Event::RequestClosed`]: the peer's direction of a request stream
    /// ended, with a FIN or by a reset.
    fn on_peer_request_stream_ended(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        how: RequestEnd,
        actions: &mut Vec<Action>,
    ) {
        match how {
            RequestEnd::Fin => self.on_request_half_closed(session, stream, actions),
            RequestEnd::Aborted => {
                // §5.2: a FETCH is cancelled by its request stream. One the
                // relay is still writing the answer of ends there. WHY here
                // and not where a request is over: a subscriber that only
                // finishes its direction has given nothing up, and ends up
                // there as well.
                self.cancel_written_fetch(session, stream, actions);
                self.on_request_closed(session, stream, true, actions);
            }
        }
    }

    /// [`Event::GoingAway`]: every session is sent its GOAWAY, in the order
    /// of their numbers, once.
    fn on_going_away(&mut self, going_away: GoingAway, actions: &mut Vec<Action>) {
        if self.going_away.is_some() {
            return;
        }
        self.going_away = Some(going_away);
        let mut sessions: Vec<SessionKey> = self.sessions.keys().copied().collect();
        sessions.sort_unstable();
        for session in sessions {
            self.go_away(session, actions);
        }
    }

    /// Sends the session its GOAWAY, once the relay is to stop
    /// ([`Event::GoingAway`]). From then on the session refuses every
    /// request of the peer with `GOING_AWAY` (§10.4).
    fn go_away(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        let (Some(going_away), Some(state)) = (&self.going_away, self.sessions.get_mut(&session))
        else {
            return;
        };
        let uri = match state.moqt.role() {
            Role::Server => going_away.new_session_uri.clone(),
            Role::Client => Vec::new(),
        };
        // It fails for a second GOAWAY alone: the relay sent SETUP when the
        // session opened, and a client's GOAWAY has no URI.
        if let Ok(message) = state.moqt.go_away(uri, going_away.timeout_ms) {
            actions.push(Action::SendControl { session, message });
        }
    }

    /// [`Event::SendAway`]. Nothing where the session has had its GOAWAY
    /// already, because the relay is to stop, or is gone.
    fn send_away(&mut self, session: SessionKey, going_away: GoingAway, actions: &mut Vec<Action>) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        if state.moqt.role() != Role::Server {
            return;
        }
        if let Ok(message) = state
            .moqt
            .go_away(going_away.new_session_uri, going_away.timeout_ms)
        {
            actions.push(Action::SendControl { session, message });
        }
    }

    /// From now on a session and a request are asked of `authorizer`
    /// before they are taken. An experiment: without it the relay allows
    /// everything, as before.
    pub fn set_authorizer(&mut self, authorizer: impl Authorizer + 'static) {
        self.authorizer = Some(Authorization(Box::new(authorizer)));
    }

    /// Whether the relay's authorization, where it has one, allows a
    /// request.
    fn allowed(&self, session: SessionKey, request: &PeerRequest) -> Result<(), Denied> {
        let Some(Authorization(authorizer)) = &self.authorizer else {
            return Ok(());
        };
        // WHY a session the relay opened as the client is not checked, its
        // SETUP neither (`on_control_message`): it is the parent relay
        // this relay chose (`--parent` and its pinned certificate hash, or
        // the URI of its GOAWAY) and authenticated with TLS, and what it
        // asks for comes of requests it checked itself, a gatherer's
        // SUBSCRIBE of a participant this relay announced to it. What it
        // costs: a parent that is taken over can ask the publishers here
        // for anything announced up, which by then is a broadcast nobody
        // can trust anyway. WHY NOT have the parent put a token of its own
        // on what it asks: it would put one on its SUBSCRIBEs to every
        // publisher, not knowing which session is a relay, and how peers
        // that are not ours take a parameter they do not expect is not known
        // (moqtail-ts 0.13.0 copies the parameters of a SUBSCRIBE into its
        // SUBSCRIBE_OK, `routing.rs`). WHY NOT check and refuse: the chat's
        // gatherer above could not reach a participant here.
        if self.is_client_session(session) {
            return Ok(());
        }
        let (operation, namespace, parameters) = match request {
            PeerRequest::Subscribe(subscribe) | PeerRequest::TrackStatus(subscribe) => (
                Operation::Subscribe,
                Some(subscribe.full_track_name.namespace()),
                &subscribe.parameters,
            ),
            PeerRequest::Publish(publish) => (
                Operation::Publish,
                Some(publish.full_track_name.namespace()),
                &publish.parameters,
            ),
            PeerRequest::PublishNamespace(publish_namespace) => (
                Operation::Publish,
                Some(&publish_namespace.track_namespace),
                &publish_namespace.parameters,
            ),
            PeerRequest::SubscribeNamespace(subscribe_namespace) => (
                Operation::SubscribeNamespace,
                Some(&subscribe_namespace.track_namespace_prefix),
                &subscribe_namespace.parameters,
            ),
            PeerRequest::Fetch(fetch) => {
                let namespace = match &fetch.range {
                    xmoq_moqt::message::FetchRange::Standalone {
                        full_track_name, ..
                    } => Some(full_track_name.namespace()),
                    xmoq_moqt::message::FetchRange::RelativeJoining { .. }
                    | xmoq_moqt::message::FetchRange::AbsoluteJoining { .. } => None,
                };
                (Operation::Subscribe, namespace, &fetch.parameters)
            }
        };
        authorizer.request(operation, namespace, parameters)
    }

    /// A message on the control stream of a session. The Setup Options of
    /// the peer's SETUP are shown to the relay's authorization once the
    /// session has taken the message.
    fn on_control_message(
        &mut self,
        session: SessionKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let setup_options = match &message {
            ControlMessage::Setup(setup) => Some(setup.setup_options.clone()),
            _ => None,
        };
        let moved_to = match &message {
            ControlMessage::GoAway(go_away) => Some(go_away.new_session_uri.clone()),
            _ => None,
        };
        if let Err(error) = state.moqt.on_control_message(message) {
            self.fail_session(session, &error, actions);
        } else if let Some(uri) = moved_to {
            self.parent_going_away(session, uri, actions);
        } else if let (Some(options), Some(Authorization(authorizer)), false) = (
            setup_options,
            &self.authorizer,
            self.is_client_session(session),
        ) && let Err(code) = authorizer.setup(&options)
        {
            self.close_unauthorized(session, code, actions);
        }
    }

    /// Closes a session the relay's authorization does not allow, or one
    /// whose token asks for it (§10.2.2).
    fn close_unauthorized(
        &mut self,
        session: SessionKey,
        code: TerminationCode,
        actions: &mut Vec<Action>,
    ) {
        if !self.sessions.contains_key(&session) {
            return;
        }
        actions.push(Action::CloseSession {
            session,
            code: code.code(),
            reason: "authorization".to_string(),
        });
        self.drop_session(session, actions);
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
            Ok(PeerRequestOpened::Refused { response, request }) => {
                if let Some(track_alias) = offered_alias {
                    state.drop_early_streams(session, track_alias, actions);
                }
                self.finish_slot(session, stream, request, actions);
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
        match self.allowed(session, &peer_request) {
            Ok(()) => {}
            Err(Denied::Session(code)) => return self.close_unauthorized(session, code, actions),
            Err(Denied::Request(code)) => {
                if let Some(track_alias) = offered_alias
                    && let Some(state) = self.sessions.get_mut(&session)
                {
                    state.drop_early_streams(session, track_alias, actions);
                }
                self.refuse(session, stream, request, &Refusal::code(code), actions);
                return;
            }
        }
        match peer_request {
            PeerRequest::Fetch(fetch) => self.on_fetch(session, stream, request, &fetch, actions),
            PeerRequest::TrackStatus(track_status) => {
                self.on_track_status(session, stream, request, &track_status, actions);
            }
            PeerRequest::Subscribe(subscribe)
                if self.asks_back(session, &subscribe.full_track_name) =>
            {
                let nobody = Refusal::code(request_error_code::DOES_NOT_EXIST);
                self.refuse(session, stream, request, &nobody, actions);
            }
            PeerRequest::Subscribe(subscribe) => {
                self.on_subscribe(session, stream, request, &subscribe, actions);
            }
            PeerRequest::Publish(publish) => {
                self.on_publish(session, stream, request, publish, actions);
            }
            PeerRequest::SubscribeNamespace(subscribe_namespace) => {
                self.on_subscribe_namespace(
                    session,
                    stream,
                    request,
                    &subscribe_namespace,
                    actions,
                );
            }
            PeerRequest::PublishNamespace(publish_namespace) => {
                let namespace = publish_namespace.track_namespace;
                // §10.6.2: "EXCESSIVE_LOAD: The responder is overloaded and
                // cannot process the request at this time." Asked first, so
                // that an announcement that is refused for another reason
                // counts as well.
                if !self.takes_an_announcement(session, actions) {
                    // One is back after that long at the latest (§10.6.2:
                    // "The sender SHOULD use the Retry Interval to indicate
                    // when the request can be retried").
                    let too_often = Refusal::loaded_for_ms(self.announcement_every_ms);
                    self.refuse(session, stream, request, &too_often, actions);
                    return;
                }
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
                    self.ask_the_publishers(&track, Some(session), None, actions);
                }
                self.tell_the_namespace_subscriptions(actions);
                self.pass_announcements_to_parent(actions);
            }
        }
    }

    /// A SUBSCRIBE beyond the [`MAX_WAITING_SUBSCRIBERS`] that wait for one
    /// track gets `EXCESSIVE_LOAD`, where one beyond the limits of the hold
    /// ([`MAX_HELD_SUBSCRIBES_PER_SESSION`], [`MAX_HELD_TRACKS`]) gets
    /// `DOES_NOT_EXIST` while nobody is asked: the two codes are not alike.
    /// A viewer that asks anew in the session it watched in gets
    /// `DUPLICATE_SUBSCRIPTION` until the relay sent it `PUBLISH_DONE`, and
    /// never after.
    ///
    /// When the publisher is back within the time a track drains
    /// (`HubConfig::drain_timeout` of `xmoq-relay`), the track is the same
    /// one: the `SUBSCRIBE_OK` carries the Largest Object of what was
    /// published before. The relay leaves that as it is, and a publisher that
    /// publishes a track anew begins above every Group it used before (§2.4).
    /// WHY NOT have the relay take the new publisher's Largest Object: its
    /// cache and what it answers a FETCH with rest on one name being one
    /// content. Seen in a run: a second own-rust process that began at Group
    /// 0 again within the 2 seconds had its Groups 0 to 2 left out for the
    /// held subscriber; with `--first-group now` the subscriber gets the
    /// first Object (interop run `another-publisher-comes`). The `moqtail`
    /// relay 0.15.0 does not hold: it answered such a SUBSCRIBE at once with
    /// `0x10` (`docs/experiments/2026-10-13-a-viewer-that-comes-before-the-broadcast.md`).
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
        let filter = params::subscription_filter(&subscribe.parameters);
        let waiter = Waiter::new(
            session,
            stream,
            request,
            forward,
            filter,
            routing::Delivery::of(&subscribe.parameters),
        );
        // §10.2.6: "If the RENDEZVOUS_TIMEOUT is present, the relay SHOULD
        // hold the subscription and wait for a publisher to appear, up to
        // the specified duration. The relay does not send SUBSCRIBE_OK
        // until a publisher becomes available."
        // WHY the limit on such tracks is looked at here, for every way a
        // held SUBSCRIBE comes to its track: one that is asked of a session
        // which announced and then refuses is held as one that nobody was
        // asked for, and a limit that one of the ways goes round is none.
        let room = self.holds_a_subscribe(track_name) || self.held_tracks() < MAX_HELD_TRACKS;
        let hold = self
            .rendezvous_ms(session, &subscribe.parameters)
            .filter(|_| room);
        let waiter = match hold {
            Some(wait_ms) => waiter.held(wait_ms),
            None => waiter,
        };
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
                self.ask_the_publishers(track_name, None, None, actions);
                return;
            }
            // Every source said PUBLISH_DONE and the track still drains: it
            // has no Established upstream subscription (§9.4). §9.5 has the
            // publishers of the namespace asked then; a subscriber that
            // did not ask to be held is refused instead (known gap; §10.2.6
            // has it answered at once). The same goes while the relay still
            // waits for the answer of a publisher it asked for this track
            // earlier. WHY NOT hold one that did not ask to be held: §10.2.6
            // has the relay answer at once where the parameter is absent, and
            // the case needs a source that drains inside the time an answer
            // may take (`UPSTREAM_ANSWER_WAIT_MS`). WHY a PUBLISH_NAMESPACE
            // that arrives now does have its publisher asked: the track has
            // subscribers, who are there already and stay if that publisher
            // accepts.
            //
            // One that is held for a publisher to appear waits with the
            // track: its sources are all done, so it has no publisher
            // (§10.2.6: "This applies when a relay receives a SUBSCRIBE for
            // a Track that has no current publisher."), and one
            // may come back while the last streams are awaited.
            Some(TrackPhase::Draining) if hold.is_some() => {
                self.wait_while_it_drains(track_name, waiter)
            }
            Some(TrackPhase::Draining) => Err(request_error_code::DOES_NOT_EXIST),
            Some(TrackPhase::Pending) => {
                // A track that is held for a publisher to appear, and none
                // is asked for it: a subscriber waits there by its own
                // RENDEZVOUS_TIMEOUT or not at all (§10.2.6, as below).
                let nobody_yet = self.tracks.asks(track_name).is_empty();
                if nobody_yet && hold.is_none() && !self.waits_for_parent(track_name, actions) {
                    Err(request_error_code::DOES_NOT_EXIST)
                } else {
                    let waits = self.wait_with_the_others(track_name, waiter);
                    if waits.is_ok() {
                        self.ask_the_publishers(track_name, None, None, actions);
                    }
                    waits
                }
            }
            // §9.5: "If there are no Established upstream subscriptions for
            // the requested Track, the Relay MUST send a SUBSCRIBE request to
            // each publisher that has published the subscription's namespace
            // or prefix thereof."
            None => {
                let asked = self.ask_the_publishers(track_name, None, Some(&waiter), actions);
                // WHY not held where a publisher is there and could not be
                // asked: one has appeared, and the code says why it is not
                // asked.
                let holds = hold.is_some() && asked.nobody_because.is_none();
                // The parent is asked once its session is there.
                let for_parent =
                    asked.nobody_because.is_none() && self.waits_for_parent(track_name, actions);
                if asked.asks.is_empty() && (holds || for_parent) {
                    let pending = PendingTrack {
                        asks: Vec::new(),
                        waiters: vec![waiter],
                        declined: Declined::default(),
                    };
                    self.tracks.insert_pending(track_name.clone(), pending);
                    Ok(())
                } else if asked.asks.is_empty() {
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
                actions.push(Action::SubscribeHeld { session, stream });
                // Its time to be held runs whether or not somebody is asked
                // for the track now: the one that is asked may refuse.
                let waits_still =
                    self.tracks.pending(track_name).is_some() || self.holds_a_subscribe(track_name);
                if let (true, Some(wait_ms)) = (waits_still, hold) {
                    actions.push(Action::StartRendezvousTimer {
                        session,
                        stream,
                        wait_ms,
                    });
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

    /// How long a SUBSCRIBE of this session with these parameters is held
    /// for a publisher to appear, in milliseconds: its `RENDEZVOUS_TIMEOUT`,
    /// and no more than [`MAX_RENDEZVOUS_MS`]. `None` where it asks for
    /// none, and where the session has as many held as it may
    /// ([`MAX_HELD_SUBSCRIBES_PER_SESSION`]): such a SUBSCRIBE is answered as
    /// one that does not want to wait.
    fn rendezvous_ms(&self, session: SessionKey, parameters: &[Parameter]) -> Option<u64> {
        let asked = params::rendezvous_timeout(parameters)?;
        let held = self.sessions.get(&session).map_or(0, |state| {
            state
                .requests
                .iter()
                .filter(|(on, slot)| match slot {
                    RequestSlot::Waiting { track, .. } => self
                        .tracks
                        .waiters(track)
                        .iter()
                        .any(|waiter| waiter.is_on(session, **on) && waiter.is_held()),
                    _ => false,
                })
                .count()
        });
        (held < MAX_HELD_SUBSCRIBES_PER_SESSION).then_some(asked.min(MAX_RENDEZVOUS_MS))
    }

    /// The tracks that have a SUBSCRIBE held for a publisher to appear:
    /// those without a source, whether or not somebody is asked for them
    /// now, and those whose sources all drain.
    fn held_tracks(&self) -> usize {
        self.tracks.held()
    }

    /// Whether the track is one of those [`Self::held_tracks`] counts.
    fn holds_a_subscribe(&self, track: &FullTrackName) -> bool {
        self.tracks.waiters(track).iter().any(Waiter::is_held)
    }

    /// Holds a SUBSCRIBE that came while every source of the track drains.
    fn wait_while_it_drains(
        &mut self,
        track_name: &FullTrackName,
        waiter: Waiter,
    ) -> Result<(), u64> {
        // Never absent: the caller found the track draining.
        let Some(track) = self.tracks.live_mut(track_name) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        if track.held.len() >= MAX_WAITING_SUBSCRIBERS {
            return Err(request_error_code::EXCESSIVE_LOAD);
        }
        track.held.push(waiter);
        Ok(())
    }

    /// The time a SUBSCRIBE was held for a publisher to appear is over. It
    /// is refused where none has: §10.2.6, "If the timeout expires without
    /// a publisher, the relay SHOULD respond with `REQUEST_ERROR` with
    /// error code TIMEOUT."
    ///
    /// WHY not where a publisher is asked for the track and has not
    /// answered: "If a publisher becomes available within this time, the
    /// relay proceeds with the subscription normally". The SUBSCRIBE is no
    /// longer held then, and becomes of it what the publisher answers, or
    /// the time the relay gives it to.
    fn on_rendezvous_timeout(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let waiting = self
            .sessions
            .get(&session)
            .and_then(|state| state.requests.get(stream))
            .and_then(|slot| match slot {
                RequestSlot::Waiting { request, track, .. } => Some((*request, track.clone())),
                _ => None,
            });
        let Some((request, track)) = waiting else {
            return;
        };
        if !self.hold_is_over(&track, session, stream) {
            return;
        }
        self.waiters_left(&track, |waiter| waiter.is_on(session, stream), actions);
        self.refuse(
            session,
            stream,
            request,
            &Refusal::code(request_error_code::TIMEOUT),
            actions,
        );
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
        // WHY a stream to stop is replayed too: `on_data_stream_opened` reads
        // the same rule and stops it with its code.
        if early::settle(state.moqt.subscribed_alias(track_alias), true) == Settle::Keep {
            return;
        }
        let (ready, waiting): (VecDeque<_>, VecDeque<_>) = std::mem::take(&mut state.early)
            .into_iter()
            .partition(|early| early.header.track_alias == track_alias);
        state.early = waiting;
        for early in ready {
            actions.push(Action::StopEarlyTimer {
                session,
                stream: early.stream,
            });
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
                | RequestSlot::SubscribeNamespace { request }
                | RequestSlot::Fetch { request, .. }
                | RequestSlot::TrackStatus { request, .. }
                | RequestSlot::Finished(Some(request)),
            ) => (*request, None),
            Some(RequestSlot::Finished(None) | RequestSlot::Stopped) | None => return,
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
                        // The session has rejected the request; what the
                        // request held in the relay goes the way of a closed
                        // stream. The session keeps the request until the
                        // stream really closes, and so does the slot.
                        self.request_is_over(session, stream, false, false, actions);
                        self.finish_slot(session, stream, Some(request), actions);
                    }
                }
            }
            PeerRequestEvent::SubscriptionUpdated {
                response,
                subgroup_delivery_timeout,
                new_group_request,
            } => {
                // §10.9: "MUST respond with exactly one REQUEST_OK or
                // REQUEST_ERROR".
                actions.push(Action::SendRequest {
                    session,
                    stream,
                    message: response,
                });
                // §10.9: a parameter that is not in the update "remains
                // unchanged".
                if let (Some(milliseconds), Some(track)) = (subgroup_delivery_timeout, &track) {
                    self.set_delivery_timeout(track, session, milliseconds, actions);
                }
                if let (Some(group), Some(track)) = (new_group_request, &track) {
                    self.ask_for_a_new_group(track, group, actions);
                }
            }
            // For a FETCH that has its FETCH_OK.
            PeerRequestEvent::UpdateAccepted {
                response,
                subscriber_priority,
            } => self.on_fetch_updated((session, stream), response, subscriber_priority, actions),
            PeerRequestEvent::UpdateHeld {
                subscriber_priority,
            } => self.on_fetch_update_held(session, stream, subscriber_priority, actions),
            // The first two: the answer to a NEW_GROUP_REQUEST the relay
            // passed on. Nothing hangs on it: a publisher that refused ends
            // the subscription with PUBLISH_DONE next (§10.9.1), as it ends
            // any other.
            //
            // The last: a client that is to send its request again elsewhere
            // (§10.4); it ends this one itself, and the relay serves it until
            // then.
            PeerRequestEvent::OwnUpdateAccepted { .. }
            | PeerRequestEvent::OwnUpdateFailed(_)
            | PeerRequestEvent::OwnUpdateAnsweredAfterRefusal
            | PeerRequestEvent::UpdateAlreadyRefused
            | PeerRequestEvent::LateMessage
            | PeerRequestEvent::GoAway(_) => {}
        }
    }

    /// A subscriber sent a `NEW_GROUP_REQUEST` for a track with a source,
    /// in its SUBSCRIBE or in a `REQUEST_UPDATE` (§10.2.13): "A relay that
    /// receives a `NEW_GROUP_REQUEST` for an Established subscription with a
    /// value of 0 or a value larger than the Largest Group MUST send a
    /// `REQUEST_UPDATE` including the `NEW_GROUP_REQUEST` to the publisher
    /// unless: 1. The Track does not support dynamic Groups 2. There is
    /// already an outstanding `NEW_GROUP_REQUEST` from this Relay with a
    /// greater or equal value".
    ///
    /// WHY to every source that has `DYNAMIC_GROUPS`, where a track has
    /// several: the draft speaks of one publisher, and each of them is an
    /// Original Publisher's way to the relay. A source that came with a
    /// PUBLISH is asked on the stream of its PUBLISH, one the relay
    /// subscribed to on the stream of the relay's SUBSCRIBE.
    ///
    /// The subscriber's own update gets `REQUEST_OK` whatever is passed on.
    /// A request is outstanding until an Object of a larger Group comes, and
    /// also ends when a source is added ([`Tracks::add_source`]). The
    /// relay's `PUBLISH_OK` carries no `NEW_GROUP_REQUEST`.
    ///
    /// WHY the relay's upstream subscription is the "Established
    /// subscription" of a SUBSCRIBE the relay answers from a track it has:
    /// the draft does not say whose it is. A publisher's answer that comes
    /// after its `PUBLISH_DONE` closes its session: §10.11 has `PUBLISH_DONE`
    /// as "the final message".
    fn ask_for_a_new_group(
        &mut self,
        track_name: &FullTrackName,
        group: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(track) = self.tracks.live(track_name) else {
            return;
        };
        // "If a relay receives a NEW_GROUP_REQUEST with a non-zero value
        // less than or equal to the Largest Group, it does not send a
        // NEW_GROUP_REQUEST upstream."
        let known_already =
            group != 0 && track.largest.is_some_and(|largest| group <= largest.group);
        let asked_already = track.new_group_asked.is_some_and(|asked| asked >= group);
        if known_already || asked_already {
            return;
        }
        let sources: Vec<TrackSource> = track
            .sources()
            .filter(|source| source.dynamic_groups && !source.draining)
            .map(|source| source.from)
            .collect();
        let mut sent = false;
        for source in sources {
            let parameters = vec![Parameter::NewGroupRequest(group)];
            // An error: 16 updates of the relay wait for the publisher's
            // answer. One more would not be read either.
            let action = match source {
                TrackSource::Publisher {
                    session, stream, ..
                } => self.sessions.get_mut(&session).and_then(|state| {
                    let Some(RequestSlot::Publish { request, .. }) = state.requests.get(stream)
                    else {
                        return None;
                    };
                    let message = state.moqt.update_published(*request, parameters).ok()?;
                    Some(Action::SendRequest {
                        session,
                        stream,
                        message,
                    })
                }),
                TrackSource::Subscription {
                    session, request, ..
                } => self.sessions.get_mut(&session).and_then(|state| {
                    let id = state.own.get(&request)?.id;
                    let message = state.moqt.update_subscription(id, parameters).ok()?;
                    Some(Action::SendOwnRequest {
                        session,
                        request,
                        message,
                    })
                }),
            };
            if let Some(action) = action {
                actions.push(action);
                sent = true;
            }
        }
        if sent && let Some(track) = self.tracks.live_mut(track_name) {
            track.new_group_asked = Some(group);
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
        self.request_is_over(session, stream, finish, true, actions);
    }

    /// Lets go of what a request of the peer held in the relay. With
    /// `stream_gone` the session is told that the stream is gone; without,
    /// the request is over while its stream is still there, and the session
    /// goes on holding it as rejected.
    fn request_is_over(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        finish: bool,
        stream_gone: bool,
        actions: &mut Vec<Action>,
    ) {
        let slot = self.take_slot(session, stream, actions);
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        // WHY the kinds that are finished are named and not those that are
        // not: a kind added later is finished here only where whoever adds
        // it says so; the others end the relay's direction below, or have.
        if finish
            && matches!(
                slot,
                Some(
                    RequestSlot::Subscribe { .. }
                        | RequestSlot::Publish { .. }
                        | RequestSlot::PublishNamespace { .. }
                        | RequestSlot::SubscribeNamespace { .. }
                        | RequestSlot::Waiting { .. }
                        | RequestSlot::TrackStatus { .. }
                )
            )
        {
            actions.push(Action::FinishRequest { session, stream });
        }
        match slot {
            // §5.2: the subscriber cancelled its FETCH.
            Some(RequestSlot::Fetch { .. }) => self.on_fetch_cancelled(session, stream, actions),
            // The subscriber gave its TRACK_STATUS up before an answer came.
            Some(RequestSlot::TrackStatus { request, .. }) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                self.on_track_status_cancelled(session, stream, actions);
            }
            Some(RequestSlot::Subscribe { request, track, .. }) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                // The subscriber cancelled (§3.3.2, §5.1.1 "MUST reset any open
                // streams associated with the SUBSCRIBE"). No PUBLISH_DONE can
                // follow on a stream that is gone.
                self.remove_subscriber(&track, session, stream_reset_code::CANCELLED, actions);
            }
            Some(RequestSlot::Publish { request, track }) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                // After PUBLISH_DONE the stream may close, and the source
                // goes on draining.
                let cancelled =
                    |source: &Source| source.is_publish(session, stream) && !source.draining;
                self.source_gone(&track, cancelled, TrackEnd::SourceGone, actions);
            }
            Some(RequestSlot::Waiting { request, track, .. }) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                self.waiters_left(&track, |waiter| waiter.is_on(session, stream), actions);
            }
            Some(RequestSlot::PublishNamespace { request, .. }) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                // §9.5: cancelling the request withdraws the namespace.
                self.withdraw_namespace(session, stream);
                self.tell_the_namespace_subscriptions(actions);
                self.pass_announcements_to_parent(actions);
            }
            Some(RequestSlot::SubscribeNamespace { request }) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                self.end_namespace_subscription(session, stream);
            }
            Some(RequestSlot::Finished(Some(request))) => {
                if stream_gone {
                    state.moqt.on_peer_request_closed(request);
                }
                // A FETCH that was served: the subscriber has ended its
                // direction, which the relay waited for with its own.
                if self.fetch_ends.remove(&(session, stream)) {
                    actions.push(Action::FinishRequest { session, stream });
                }
            }
            Some(RequestSlot::Finished(None) | RequestSlot::Stopped) | None => {}
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

    /// Opens the downstream streams of a subgroup stream of a source once
    /// its Subgroup is known: at the header where it names the Subgroup ID or
    /// has none (Subgroup 0), at the first Object otherwise (§11.4.2: "0b01:
    /// The Subgroup ID field is absent and the Subgroup ID is the Object ID
    /// of the first Object transmitted in this Subgroup"). A header followed
    /// by a FIN and no Object is passed on and counts as delivered whole; it
    /// does not fit §11.4 ("followed by one or more sets of serialized Object
    /// fields"), and the draft does not say what a receiver does with it. A
    /// stream whose header leaves the Subgroup ID to its first Object and
    /// that starts inside a Subgroup is taken for a Subgroup of its own
    /// ([`Track::taking_of_whole`]).
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
        let alias = state.moqt.subscribed_alias(header.track_alias);
        // WHY an answer may always still come (`true`): the relay ends the
        // wait with its own timer ([`Action::StartEarlyTimer`]).
        let track_name = match early::arrival(alias, true, 0, 0) {
            Arrival::Deliver => match alias {
                SubscribedAlias::Established(track) | SubscribedAlias::Draining(track) => {
                    track.clone()
                }
                // Not reached: `arrival` delivers only these two.
                SubscribedAlias::Retired | SubscribedAlias::Unknown => return,
            },
            Arrival::Stop(code) => {
                actions.push(Action::StopDataStream {
                    session,
                    stream,
                    code,
                });
                return;
            }
            Arrival::Hold => {
                state.hold_early_stream(session, stream, header, actions);
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
        let taking = subgroup_id.map(|id| {
            let subgroup = (header.group_id, id);
            let of = (header.track_alias, priority);
            track.admit(
                subgroup,
                (session, stream),
                of,
                &mut self.out_streams,
                actions,
            )
        });
        let malformed = taking == Some(Taking::Malformed);
        let passage = taking.map_or(Passage::Undetermined, Taking::passage);
        let mut in_stream = InStream {
            header,
            subgroup_id,
            priority,
            passage,
            last_object: None,
            started: false,
            of_the_subscription_before: false,
            outs: HashMap::new(),
            stopped: HashSet::new(),
            receiving: None,
        };
        // Where the header gives the Subgroup ID, every subscriber gets a
        // stream now, also for a Subgroup that turns out to hold no Object: a
        // header that is followed by a reset says that there is a Subgroup
        // the source will not send (§11.4.3).
        if passage == Passage::Passed {
            // A subscription that starts inside this Group gets its stream
            // with the first Object that passes (`Self::on_object`): which
            // Objects the stream will carry is not known here. WHY NOT with
            // the header: a stream that is opened and carries nothing would
            // count in the Stream Count of `PUBLISH_DONE`, and be one the
            // subscriber has to end.
            let passing = |subscriber: &&mut Subscriber| subscriber.passes_all_of(header.group_id);
            for subscriber in track.subscribers.iter_mut().filter(passing) {
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
        let Some(taking) = track.taking_of_whole(key, &object, &mut self.out_streams, actions)
        else {
            return;
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
            Taking::Checked => {
                in_stream.passage = Passage::Checked;
                self.on_checked_object(session, stream, object.object_id, actions);
                return;
            }
            Taking::Malformed => {
                self.on_track_malformed(session, stream, actions);
                return;
            }
        }
        in_stream.passage = Passage::Passed;
        in_stream.last_object = Some(object.object_id);
        let location = Location {
            group: in_stream.header.group_id,
            object: object.object_id,
        };
        // §11.4.2, mode 0b01: the Subgroup ID is the Object ID of the first
        // Object of the Subgroup.
        let subgroup_id = *in_stream.subgroup_id.get_or_insert(object.object_id);
        let object = Arc::new(object);
        // WHY before anything is done with the Object: one that shows the
        // track to be malformed is not counted as its largest and not sent
        // on, as an Object whose stream does (above).
        let kept_before = track.cache.kept_group();
        let verdict = keep(
            &mut track.cache,
            track.never_cached,
            in_stream,
            subgroup_id,
            &object,
            &mut self.cached_bytes,
        );
        self.cache_let_go |= kept_before.is_some() && track.cache.kept_group() != kept_before;
        if verdict == cache::Verdict::Malformed {
            self.on_track_malformed(session, stream, actions);
            return;
        }
        // An Object that was whole when its fields were read is received
        // here; one that was not, at its first byte (`Self::on_object_begun`).
        let receiving = in_stream
            .receiving
            .take()
            .filter(|receiving| receiving.object_id == object.object_id);
        raise_largest(&mut track.largest, &mut track.new_group_asked, location);

        let header = in_stream.header_downstream(subgroup_id);
        let accepted_during = |subscriber: &Subscriber| {
            receiving.as_ref().is_some_and(|receiving| {
                receiving
                    .accepted_during
                    .contains(&(subscriber.session, subscriber.track_alias))
            })
        };
        for subscriber in &mut track.subscribers {
            if in_stream.lacks_a_stream_for(subscriber)
                && subscriber.passes(location)
                && !accepted_during(subscriber)
            {
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
        for subscriber in &track.subscribers {
            if let Some(out_stream) = in_stream.outs.get(&subscriber.session) {
                actions.push(Action::SendObject {
                    out_stream: *out_stream,
                    object: Arc::clone(&object),
                });
            }
        }
        // What was held for it can be answered from the cache now.
        if receiving.is_some() {
            let track_name = track_name.clone();
            self.receipt_ended(&track_name, location, true, actions);
        }
    }

    /// The first byte of an Object arrived, and its payload is still on the
    /// way ([`Event::ObjectBegun`]): the Object is received from here on
    /// (§5.1.2), and raises the largest Location of the track. A FETCH that
    /// asks for it waits for it to be whole (`fetch::ReceiptWait`).
    ///
    /// WHY at the first byte and not when the Object is whole (decided on
    /// 2026-10-25): §5.1.2 has the Largest Object update "when the first
    /// byte of an Object ... is ... received", and §10.2.11 has a relay
    /// "MUST set `LARGEST_OBJECT` to the largest of" what its upstream said
    /// and "The largest Location of an Object received on an upstream
    /// subscription"; draft 18 changed that so that relays do not lie about
    /// `LARGEST_OBJECT` (#1621). WHY NOT raise it and ask a publisher for a
    /// FETCH that reaches the Object: a viewer that comes while a key frame
    /// is received gets its FETCH refused where the publisher serves none
    /// (the browser's), its subscription starts after the key frame, and it
    /// has no picture until the next Group.
    ///
    /// Nothing that makes a track malformed by the Location alone is decided
    /// here; an Object that would is not counted until it is whole, where it
    /// is decided (`Track::malformed_at`). WHY: two places that decide would
    /// make what happens depend on which of the two did. A stream that
    /// leaves its Subgroup ID to its first Object is admitted here: §11.4.2
    /// has it from "the first Object transmitted", which is known once its
    /// Object ID is read, and the largest Location is not to be raised for a
    /// stream that turns out to be left.
    fn on_object_begun(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        object_id: u64,
        actions: &mut Vec<Action>,
    ) {
        // A stream of a Track Alias that is not known yet is not of a
        // subscription yet: its Objects count when they are let go of
        // (`hold_early_object`), whole.
        let Some(track_name) = self
            .sessions
            .get(&session)
            .and_then(|state| state.in_streams.get(&stream))
        else {
            return;
        };
        let Some(track) = self.tracks.live_mut(track_name) else {
            return;
        };
        let key = (session, stream);
        let (group, of, known) = match track.in_streams.get(&key) {
            // A stream that was stopped carries nothing that counts
            // (§11.4.3, and `Passage::Discarded`); one read for the check
            // alone (`Passage::Checked`) carries a Subgroup taken from
            // another stream, and what begins on it is compared when it is
            // whole (`Self::on_checked_object`).
            Some(in_stream)
                if in_stream.passage != Passage::Discarded
                    && in_stream.passage != Passage::Checked =>
            {
                (
                    in_stream.header.group_id,
                    (in_stream.header.track_alias, in_stream.priority),
                    in_stream.subgroup_id,
                )
            }
            _ => return,
        };
        let location = Location {
            group,
            object: object_id,
        };
        if track.malformed_at(location) {
            return;
        }
        let taking = if known.is_some() {
            Taking::Taken
        } else {
            track.admit((group, object_id), key, of, &mut self.out_streams, actions)
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
            // Read for the check alone: the Subgroup is another stream's,
            // and this Object raises nothing.
            Taking::Checked => {
                in_stream.passage = Passage::Checked;
                return;
            }
            Taking::Malformed => {
                self.on_track_malformed(session, stream, actions);
                return;
            }
        }
        in_stream.passage = Passage::Passed;
        in_stream.subgroup_id.get_or_insert(object_id);
        in_stream.receiving = Some(Receiving {
            object_id,
            accepted_during: HashSet::new(),
        });
        raise_largest(&mut track.largest, &mut track.new_group_asked, location);
    }

    /// An Object on a stream read for the check alone ([`Passage::Checked`]):
    /// the track is malformed where it is larger than the final Object of its
    /// Subgroup (§2.4.2, the second example). It goes nowhere else: not
    /// downstream, not into the cache, and it is not counted as the track's
    /// largest ("Object(s) triggering Malformed Track status MUST NOT be
    /// cached").
    fn on_checked_object(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        object_id: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(track_name) = self
            .sessions
            .get(&session)
            .and_then(|state| state.in_streams.get(&stream))
        else {
            return;
        };
        let Some(track) = self.tracks.live_mut(track_name) else {
            return;
        };
        let Some(in_stream) = track.in_streams.get_mut(&(session, stream)) else {
            return;
        };
        in_stream.last_object = Some(object_id);
        // §11.4.2, mode 0b01, as for a stream that passes.
        in_stream.subgroup_id.get_or_insert(object_id);
        let past_the_end = in_stream.subgroup_id.is_some_and(|subgroup| {
            track
                .subgroups
                .final_object(in_stream.header.group_id, subgroup)
                .is_some_and(|final_object| object_id > final_object)
        });
        if past_the_end {
            // WHY stopped here: what ends the track stops the streams it
            // took Objects from, and not this one.
            actions.push(Action::StopDataStream {
                session,
                stream,
                code: stream_reset_code::MALFORMED_TRACK,
            });
            self.on_track_malformed(session, stream, actions);
        }
    }

    /// The time a stream read for the check alone had is over: it is stopped,
    /// and what it would have shown is not found.
    fn on_check_timeout(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(track_name) = self
            .sessions
            .get(&session)
            .and_then(|state| state.in_streams.get(&stream))
        else {
            return;
        };
        let Some(in_stream) = self
            .tracks
            .live_mut(track_name)
            .and_then(|track| track.in_streams.get_mut(&(session, stream)))
            .filter(|in_stream| in_stream.passage == Passage::Checked)
        else {
            return;
        };
        in_stream.passage = Passage::Discarded;
        actions.push(Action::StopDataStream {
            session,
            stream,
            code: stream_reset_code::CANCELLED,
        });
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
        if track.ends_apart((session, stream), end) {
            self.track_malformed_by(&track_name, (session, stream), actions);
            return;
        }
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
        // An Object it ended inside of: nothing of it is cached (§9.1, "A
        // cache MUST store all fields of an Object"), and the largest
        // Location stays (`raise_largest`).
        let cut_inside = track
            .in_streams
            .get(&(session, stream))
            .and_then(|in_stream| {
                let receiving = in_stream.receiving.as_ref()?;
                (in_stream.passage == Passage::Passed).then_some(Location {
                    group: in_stream.header.group_id,
                    object: receiving.object_id,
                })
            });
        track.in_stream_ended((session, stream), end, &mut self.out_streams, actions);
        self.retire_drained_sources(&track_name, actions);
        if let Some(location) = cut_inside {
            self.receipt_ended(&track_name, location, false, actions);
        }
    }

    /// The I/O side is done with a stream: nothing more is to be said about
    /// it.
    /// Marks the subscription of a stream its subscriber stopped, on the
    /// source's stream, so that it is not opened again ([`InStream::stopped`]).
    fn out_stream_stopped(&mut self, out_stream: OutStreamKey) {
        let Some(out) = self.out_streams.get(&out_stream) else {
            return;
        };
        let Some(track) = self.tracks.live_mut(&out.track) else {
            return;
        };
        let Some(track_alias) = track
            .subscribers
            .iter()
            .find(|subscriber| subscriber.outs.contains(&out_stream))
            .map(|subscriber| subscriber.track_alias)
        else {
            return;
        };
        let subscriptions: HashSet<(SessionKey, u64)> = track
            .subscribers
            .iter()
            .map(|subscriber| (subscriber.session, subscriber.track_alias))
            .collect();
        if let Some(in_stream) = track
            .in_streams
            .values_mut()
            .find(|in_stream| in_stream.outs.get(&out.session) == Some(&out_stream))
        {
            // Bounded by the subscriptions there are: those that went are
            // let go of here.
            in_stream
                .stopped
                .retain(|subscription| subscriptions.contains(subscription));
            in_stream.stopped.insert((out.session, track_alias));
        }
    }

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
    /// §9.4's "the upstream publisher terminates the subscription" speaks of
    /// one publisher; with several the relay takes it to mean the last.
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
        // WHY not for a wait that ran out: its timer is the one that said
        // so.
        if end != TrackEnd::TimedOut {
            actions.extend(gone.iter().filter_map(|source| {
                let (session, _) = source.session_and_alias();
                source
                    .wait
                    .map(|wait| Action::StopDrainTimer { session, wait })
            }));
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
        self.cache_let_go |= track.cache.kept_group().is_some();
        track.cache.clear(&mut self.cached_bytes);
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
        // A FETCH held for an Object of the track has nothing to wait for.
        self.receipts_of_a_gone_track(track_name, actions);
        // The SUBSCRIBEs that were held while the track drained wait on
        // under its name, for what is left of their time: the track has no
        // source, and the publishers of its namespace are asked as for any
        // SUBSCRIBE that comes to a name without one.
        let held = std::mem::take(&mut track.held);
        if !held.is_empty() {
            let pending = PendingTrack {
                asks: Vec::new(),
                waiters: held,
                declined: Declined::default(),
            };
            self.tracks.insert_pending(track_name.clone(), pending);
            self.ask_the_publishers(track_name, None, None, actions);
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
        let end = match (end, own) {
            (TrackEnd::Done, _) => OwnEnd::Answered,
            // §3.3.2: the relay cancels its own request. For a malformed
            // track §2.4.2 asks for exactly that.
            (
                TrackEnd::TimedOut
                | TrackEnd::Malformed { .. }
                | TrackEnd::SourceGone
                | TrackEnd::Unwanted,
                Some(own),
            ) => OwnEnd::GivenUp {
                id: own.id,
                code: reset_code,
            },
            (_, None) => OwnEnd::Reset { code: reset_code },
        };
        self.close_own_request(session, request, end, actions);
    }

    /// A `REQUEST_UPDATE` set the `SUBGROUP_DELIVERY_TIMEOUT` of a
    /// subscription, in milliseconds; 0 is no timeout (§8). An update
    /// without the parameter leaves it as it is, and the smaller of it and
    /// `--delivery-patience-ms` of `xmoq-relay` holds
    /// (`SendLimits::delivery_patience` of `xmoq-moqt-io`).
    ///
    /// WHY not sent on to the publishers: §10.2.3 has "SHOULD NOT be
    /// forwarded upstream by a relay that intends to serve multiple
    /// subscriptions for the same track".
    fn set_delivery_timeout(
        &mut self,
        track_name: &FullTrackName,
        session: SessionKey,
        milliseconds: u64,
        actions: &mut Vec<Action>,
    ) {
        // Neither `return` is reached: the session lets an update by for a
        // subscription it has accepted and not ended, and the relay accepts
        // one only on a track that has a source (`Tracks::live`) and ends
        // every subscription of a track before the track goes. A track that
        // waits for a publisher has waiters, whose SUBSCRIBE has no answer
        // and whose update the session refuses.
        let Some(track) = self.tracks.live_mut(track_name) else {
            return;
        };
        let Some(subscriber) = track
            .subscribers
            .iter_mut()
            .find(|subscriber| subscriber.session == session)
        else {
            return;
        };
        let subgroup_delivery_timeout = Some(milliseconds).filter(|asked| *asked != 0);
        subscriber.subgroup_delivery_timeout = subgroup_delivery_timeout;
        actions.extend(
            subscriber
                .outs
                .iter()
                .map(|out_stream| Action::SetDeliveryTimeout {
                    out_stream: *out_stream,
                    subgroup_delivery_timeout,
                }),
        );
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
        let done = state.moqt.publish_done_on_subscribe(
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
        self.parent_gone(session);
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
        self.pass_announcements_to_parent(actions);
        self.drop_fetches_of(session, actions);
        self.drop_track_statuses_of(session, actions);
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
            Target::Closing | Target::Relay => true,
        });
        self.out_streams.retain(|_, out| out.session != session);
    }

    /// Whether the relay keeps an entry for a request stream of a peer: it
    /// does until the peer's direction of the stream is reported gone, and
    /// lets go at once of one it has said its last on when the peer had
    /// finished its direction before.
    ///
    /// For the I/O side, which gives a peer a time to end its direction
    /// after [`Action::FinishRequest`] and has nothing to wait for where
    /// the peer already did.
    #[must_use]
    pub fn has_request_stream(&self, session: SessionKey, stream: StreamKey) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|state| state.requests.get(stream).is_some())
    }

    /// Whether the session holds nothing: no request of the peer that is
    /// open or held (a SUBSCRIBE, also one that waits for a publisher, a
    /// PUBLISH, a namespace it announced, a FETCH), and no request the
    /// relay has open on it. `false` for a session the relay does not have.
    ///
    /// For the I/O side, which lets go of a session that stays so
    /// (§13.6.1 has a session that is idle be one that can close).
    #[must_use]
    pub fn holds_nothing(&self, session: SessionKey) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|state| state.requests.len() == 0 && state.own_requests() == 0)
            && !self.announces_to_parent_on(session)
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
            cached_bytes: self.cached_bytes,
            held_beyond_cache_bytes: self.held_beyond_the_cache().total,
            remembered_subgroups: self
                .tracks
                .all_live()
                .map(|(_, track)| track.subgroups.taken.len())
                .sum(),
            namespaces: self.namespaces.len(),
            pending_tracks: self.tracks.all_pending().count(),
            own_requests: self.sessions.values().map(SessionState::own_requests).sum(),
            fetches: self.fetch_count(),
            track_statuses: self.track_status_count(),
            fetch_ends: self.fetch_ends.len(),
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
            subgroup_delivery_timeout: subscriber.subgroup_delivery_timeout,
            send_order: send_order(
                subscriber.priority,
                // The header's, or the default of the stream's source
                // (§11.4.2). §12.4: "If omitted, the Default Publisher
                // Priority is 128", which also stands in where the relay
                // cannot tell the source (`InStream::priority`).
                in_stream
                    .priority
                    .unwrap_or(track_property::DEFAULT_PUBLISHER_PRIORITY_IF_OMITTED),
            ),
        });
    }
}

/// Where a subgroup stream towards a subscriber stands among the streams of
/// its session: **the larger number is sent first**, and streams with the
/// same number take turns. §7.2, its first two steps: the higher
/// Subscriber Priority (the lower number), then the higher Publisher
/// Priority.
///
/// - **Every number is below 0**, which is what a stream has that nobody
///   gave a number: the control stream and the request streams. §7.2: "the
///   control streams SHOULD be prioritized highest, followed by the bidi
///   request streams and then all subscribed Objects". WHY NOT the layout
///   of `moqtail`'s relay (`compute_stream_priority`, tag `relay@0.15.0`),
///   which spreads the pairs of priorities over all of an `i32`: a
///   subscriber that asks for a priority above the middle would have its
///   Objects sent before the answers to its requests.
/// - **The Group does not count** (§7.2's third step, by the Group Order of
///   the subscription, and its fourth, by the Subgroup ID): streams of one
///   subscription take turns, the older Groups with the newer. WHY NOT a
///   part of the number for the Group, as `moqtail`'s relay has: the
///   transport knows one number for a stream and not which subscription
///   it is of, so the Group would order the streams of different tracks
///   as well, where §7.2 has "the expectation is that all subscriptions
///   will be able to send some data". The demo's sound has five Groups a
///   second and its video one, both counted from the time of day: with
///   the older Group first the video's would be older for good, and on a
///   link with less room than both the sound would not be sent at all.
/// - **Not to be built**: a relay that sends from its writers of the
///   highest priority first, and the sound above the video by default.
///   Behind a narrow link the sound waited behind the send window of the
///   connection, with a priority above the video as without
///   (`docs/experiments/2026-10-15-what-the-sound-waits-behind-on-a-narrow-link.md`);
///   the demo's viewer asks for 150 ms on its video, and with that the send
///   window does not fill.
#[must_use]
pub fn send_order(subscriber_priority: u8, publisher_priority: u8) -> i32 {
    -1 - ((i32::from(subscriber_priority) << 8) | i32::from(publisher_priority))
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

/// What an Object counts for in bytes against every limit of the relay that
/// counts bytes (the cache's and `MAX_BYTES_HELD_BEYOND_THE_CACHE`), and in
/// the time a subscriber has for a fetch stream of the relay's cache
/// (`HubConfig` of `xmoq-relay`): its payload and its properties.
///
/// WHY the properties too: an Object can carry them without a payload, 64 of
/// them with values of up to 64 KiB each. WHY one function: limits that
/// count the same Object apart let what one of them keeps out through
/// another. WHY NOT for the Objects held for an alias not known yet: they
/// are counted with [`early::held_bytes`], as a client counts them, which is
/// 64 bytes an Object more than this; what that limit keeps out, the
/// others never let through.
#[must_use]
pub fn object_size(object: &SubgroupObject) -> usize {
    // What a pair takes apart from the bytes of its value: the type and a
    // varint value, 8 bytes each. The relay's reckoning of what it keeps, not
    // the length of the pair on the wire, which is shorter.
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
        (objects + 1, bytes.saturating_add(early::held_bytes(object)))
    });
    let stop = if objects >= MAX_EARLY_OBJECTS {
        Some(stream_reset_code::INTERNAL_ERROR)
    } else {
        match early::arrival(
            SubscribedAlias::Unknown,
            true,
            bytes,
            early::held_bytes(&object),
        ) {
            Arrival::Stop(code) => Some(code),
            // `Deliver` is not reached for an alias that is not known.
            Arrival::Hold | Arrival::Deliver => None,
        }
    };
    if let Some(code) = stop {
        state.early.remove(position);
        actions.push(Action::StopEarlyTimer { session, stream });
        actions.push(Action::StopDataStream {
            session,
            stream,
            code,
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
    ///
    /// The draft has no rule for the status a relay sends when its upstream
    /// ends; `TRACK_ENDED` for a track whose last source is over is the
    /// relay's choice.
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

/// The Publisher Priority of the Subgroups of a subscription that carry
/// none, from the Track Properties of the PUBLISH or `SUBSCRIBE_OK` that
/// established it, outside Immutable Properties or inside (§12.7: "When
/// looking for the value of a property, processors MUST search both").
/// A value above 255 never gets here: the decoder of `xmoq-moqt` closes the
/// session. Of two such properties the first searched counts, the ones
/// outside before the ones inside; nothing refuses a second.
fn default_publisher_priority(properties: &[KeyValuePair]) -> u8 {
    searched_properties(properties, Of::Track)
        .iter()
        .find(|property| property.ty() == track_property::DEFAULT_PUBLISHER_PRIORITY)
        .and_then(|property| match property.value() {
            Value::Varint(priority) => u8::try_from(*priority).ok(),
            Value::Bytes(_) => None,
        })
        .unwrap_or(track_property::DEFAULT_PUBLISHER_PRIORITY_IF_OMITTED)
}

/// Whether the Track Properties of a PUBLISH or a `SUBSCRIBE_OK` say that
/// the Original Publisher starts a new Group on request (§12.6: "The
/// allowed values are 0 or 1 ... If omitted, the value is 0"). A larger
/// value never gets here: the decoder of `xmoq-moqt` closes the session.
/// Inside Immutable Properties as well (§12.7).
fn dynamic_groups(properties: &[KeyValuePair]) -> bool {
    searched_properties(properties, Of::Track)
        .iter()
        .any(|property| {
            property.ty() == track_property::DYNAMIC_GROUPS && *property.value() == Value::Varint(1)
        })
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
    /// The action is of no session.
    Relay,
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
            | Self::StopDrainTimer { session, .. }
            | Self::StopEarlyTimer { session, .. }
            | Self::StopJoiningFetchTimer { session, .. }
            | Self::StartEarlyTimer { session, .. }
            | Self::StartCheckTimer { session, .. }
            | Self::StartJoiningFetchTimer { session, .. }
            | Self::StartReceiptTimer { session, .. }
            | Self::StopReceiptTimer { session, .. }
            | Self::StartRendezvousTimer { session, .. }
            | Self::StartAnnouncementTimer { session, .. }
            | Self::StartParentAnnouncementTimer { session, .. }
            | Self::SubscribeHeld { session, .. }
            | Self::StartFetchEndTimer { session, .. }
            | Self::StopRequest { session, .. }
            | Self::OpenRequest { session, .. }
            | Self::SendOwnRequest { session, .. }
            | Self::FinishOwnRequest { session, .. }
            | Self::CancelOwnRequest { session, .. }
            | Self::StartUpstreamTimer { session, .. }
            | Self::StartFetchStreamTimer { session, .. }
            | Self::StopUpstreamTimer { session, .. }
            | Self::PipeFetch { session, .. }
            | Self::AbortFetchPipe { session, .. }
            | Self::WriteFetchStream { session, .. }
            | Self::AbortFetchWrite { session, .. }
            | Self::SetFetchSendOrder { session, .. }
            | Self::NoteHeldBeyondCache { session, .. } => Target::Session(*session),
            Self::SendObject { out_stream, .. }
            | Self::SetDeliveryTimeout { out_stream, .. }
            | Self::FinishDataStream { out_stream }
            | Self::ResetDataStream { out_stream, .. } => Target::OutStream(*out_stream),
            Self::CloseSession { .. } => Target::Closing,
            Self::ConnectParent { .. } => Target::Relay,
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
