//! FETCH through the relay (§9.4, §10.12): a Standalone FETCH of a
//! subscriber is asked of one publisher at a time, and the fetch stream of
//! the one that answers is passed on as it is.
//!
//! A FETCH whose range lies wholly in what the relay has cached of the
//! track, its latest Group, is answered from there (`crate::cache`), and
//! the relay writes the fetch stream itself ([`Action::WriteFetchStream`]).
//! Every other FETCH is a publisher's to answer: the relay does not look
//! into a fetch stream it passes on, and caches nothing of it. What it
//! decides then is whom to ask, what the subscriber is told on its request
//! stream, and when the whole thing is cut short; moving the bytes is the
//! I/O side's ([`Action::PipeFetch`]).

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use xmoq_moqt::fetch::RangeEnd;
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{ControlMessage, Fetch, FetchRange, RequestOk, request_error_code};
use xmoq_moqt::name::{FullTrackName, Location};
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::{FetchStreamOpened, PeerRequestId, RequestEvent, RequestId};
use xmoq_moqt::{RequestFault, stream_reset_code};

use crate::cache;
use crate::events::{Event, FetchPiece, FetchPipeEnd};
use crate::requests::{Joinable, Subscribed};
use crate::routing::{MAX_OWN_REQUESTS_PER_SESSION, OwnRequestKind, Refusal};
use crate::{
    Action, OwnRequestEnd, OwnRequestKey, Relay, RequestSlot, SessionKey, StreamKey, internal_error,
};

/// FETCH requests one session may have open at once. One more is answered
/// with `EXCESSIVE_LOAD`.
///
/// Provisional: the draft sets no limit, and `XmoQ` has not decided one.
pub(crate) const MAX_FETCHES_PER_SESSION: usize = 16;

/// FETCH requests the relay may have open on one publisher's session at once.
/// One more is answered with `EXCESSIVE_LOAD`.
///
/// WHY a number of its own below [`MAX_OWN_REQUESTS_PER_SESSION`]: the
/// publisher's fetch stream stays open for as long as the subscriber takes
/// to read it, and each such stream is one of the unidirectional streams the
/// publisher may have open towards the relay (64). Slow subscribers with
/// many FETCH requests would otherwise use up all of them, and the publisher could
/// open no subgroup stream for anybody. Provisional.
pub(crate) const MAX_OWN_FETCHES_PER_SESSION: usize = 16;

/// Of those, how many the relay has open on one publisher for the FETCH
/// requests of one subscriber's session. One more is answered with `EXCESSIVE_LOAD`.
/// Decided on 2026-10-21.
///
/// WHY: a session may have [`MAX_FETCHES_PER_SESSION`] FETCH requests open, which
/// is all a publisher has, and one that sent 10 a second to a publisher
/// that answered late had 12 of another viewer's 14 FETCH requests refused
/// (measured 2026-10-13). §13.1 has a publisher "enforce limits to prevent
/// resource exhaustion from a single subscriber or session".
pub const MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER: usize = 4;

/// Bytes of Objects that the fetch streams the relay writes keep alive
/// after the cache has let go of their Group, over the whole relay: as much
/// as the caches themselves may take. **Provisional.**
///
/// A fetch stream the relay writes from its cache shares the Objects with
/// the cache, and costs nothing more while the cache has their Group. When
/// the cache moves on to a later Group, the stream is what keeps them. They
/// are counted then, once for a Group however many streams carry it, by the
/// bytes of the stream that carries the most of it.
///
/// Over the limit, the relay refuses a FETCH it would answer from its
/// cache with `EXCESSIVE_LOAD`, and a Group the cache lets go of is not
/// kept for the streams that carry it: they are reset with
/// `EXCESSIVE_LOAD`.
///
/// WHY: a subscriber that takes a stream slowly keeps a Group of up to
/// 8 MiB for as long as the stream has (134 seconds for that much), a
/// session has 16 FETCH requests, and the cache's own limit does not count
/// what it has dropped.
pub const MAX_BYTES_HELD_BEYOND_THE_CACHE: usize = 256 * 1024 * 1024;

/// The part of [`MAX_BYTES_HELD_BEYOND_THE_CACHE`] one subscriber's session
/// may have kept for it: an eighth. The session is refused, and its streams
/// reset, as the relay does for all over the whole. **Provisional.**
///
/// WHY: with 16 FETCH requests of 8 MiB a session has half of the whole,
/// and two sessions would have every other viewer's FETCH refused.
pub(crate) const SHARE_OF_BYTES_HELD_BEYOND_THE_CACHE: usize = 8;

/// How long a publisher may send nothing on a fetch stream while the
/// subscriber can take more, when the FETCH has no `FILL_TIMEOUT`
/// (§10.2.5: "the relay waits for an implementation specific duration").
/// Provisional.
pub(crate) const FETCH_UPSTREAM_WAIT_MS: u64 = 5_000;

/// The longest the relay waits that way, whatever `FILL_TIMEOUT` says
/// (§10.2.5: "the relay MAY use a shorter timeout without informing the
/// subscriber"). Provisional.
pub(crate) const MAX_FETCH_UPSTREAM_WAIT_MS: u64 = 30_000;

/// The least a publisher gets to open its fetch stream after its `FETCH_OK`,
/// in milliseconds, however short the `FILL_TIMEOUT`. Provisional.
pub(crate) const MIN_FETCH_STREAM_WAIT_MS: u64 = 1_000;

/// How long a subscriber may take nothing of a fetch stream before the relay
/// stops passing it on. Provisional.
pub(crate) const FETCH_DOWNSTREAM_WAIT_MS: u64 = 10_000;

/// How long the relay's direction of the request stream of a FETCH stays
/// open after its fetch stream was passed on or written to its FIN, for
/// the subscriber to end its own direction first. Provisional; the Rust
/// client and `@xmoq/moqt` wait as long where they answer a FETCH.
///
/// WHY wait at all: §5.2 has a subscriber's FETCH end with the FIN or the
/// reset of the fetch stream, with a `REQUEST_ERROR`, or with its own
/// cancelling, and a publisher "destroy all FETCH state after closing the
/// data stream with a FIN"; neither is told to finish the request stream.
/// The two streams arrive apart, and `moqtransport` v0.14.0 fails a FETCH
/// whose request stream is finished before it has read the fetch stream to
/// its end. A subscriber that has read it ends the request stream itself.
/// WHY NOT leave the direction open for good: one that never does would
/// keep the stream, and the request in the session.
pub(crate) const FETCH_PEER_END_WAIT_MS: u64 = 3_000;

/// `REQUEST_UPDATE`s for a FETCH that may wait for the answer to the FETCH,
/// each of which is owed a `REQUEST_OK` after it. One more and the FETCH is
/// refused with `EXCESSIVE_LOAD`, after which none of them gets an answer.
///
/// WHY a limit: the answers are sent in one go when the publisher's comes,
/// and how many there are is the subscriber's to choose. Provisional.
pub(crate) const MAX_HELD_FETCH_UPDATES: u32 = 64;

/// How far the passing on of the publisher's fetch stream has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pipe {
    /// The publisher has opened no fetch stream yet.
    NotOpen,
    Running,
    /// Passed on to its FIN.
    Done,
}

/// A FETCH of a subscriber that the relay asked a publisher for.
#[derive(Debug)]
pub(crate) struct RelayedFetch {
    request: PeerRequestId,
    track: FullTrackName,
    publisher: SessionKey,
    own: OwnRequestKey,
    /// Whether the subscriber has its `FETCH_OK`.
    answered: bool,
    pipe: Pipe,
    upstream_wait_ms: u64,
    /// `REQUEST_UPDATE`s that wait for the answer.
    held_updates: u32,
    /// For a Joining FETCH: the request stream of the SUBSCRIBE it names.
    joins: Option<StreamKey>,
    /// [`AskedRange::send_order`].
    send_order: i32,
    /// The range and the parameters the publisher was asked with, for the
    /// next one ([`Relay::ask_the_next_or_fail`]).
    start: Location,
    end: Location,
    parameters: Vec<Parameter>,
    /// The publishers asked so far, the one asked last among them.
    asked: Vec<SessionKey>,
}

/// What a Joining FETCH that is held waits for. WHY two kinds and not one
/// key: the SUBSCRIBE a FETCH names is known by its Request ID until it has
/// come, and by its request stream from then on, and neither stands for the
/// other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Awaited {
    /// The SUBSCRIBE on this request stream, which has come and has no
    /// answer yet.
    Answer(StreamKey),
    /// The SUBSCRIBE with this Request ID, which has not come yet: the two
    /// travel on streams of their own, and this FETCH was read first.
    Subscribe(u64),
}

/// A Joining FETCH that waits for its SUBSCRIBE: to come, or to be answered.
#[derive(Debug)]
pub(crate) struct HeldFetch {
    request: PeerRequestId,
    joins: Awaited,
    fetch: Fetch,
    /// `REQUEST_UPDATE`s that wait for the answer.
    held_updates: u32,
}

/// Where a FETCH of a subscriber stands.
#[derive(Debug)]
enum Standing {
    /// A Joining FETCH that waits for its SUBSCRIBE to come, or to be
    /// answered.
    Held(HeldFetch),
    /// Asked of a publisher.
    Relayed(RelayedFetch),
    /// Answered by the relay itself, for this track; the I/O side has not
    /// reported its fetch stream written yet.
    Written {
        track: FullTrackName,
        /// The Group the stream carries Objects of, and how many bytes of
        /// payload: what the write keeps alive when the cache lets go of
        /// that Group ([`MAX_BYTES_HELD_BEYOND_THE_CACHE`]). `None` for a
        /// stream without an Object.
        holds: Option<(KeptGroup, usize)>,
        /// The I/O side was told to give the stream up
        /// ([`Action::AbortFetchWrite`]).
        told_to_stop: bool,
    },
}

/// The FETCH requests of subscribers that the relay has not let go of, by
/// the subscriber's session and request stream.
///
/// WHY one map for the three ways a FETCH can stand: a FETCH is in one of
/// them at a time, and what is said of all of them (how many a session has
/// open, what a session that is gone leaves) is then said in one place.
///
/// WHY in the order of the sessions: what is asked on every request and
/// message of a peer ([`Relay::settle_early_fetches`]) then looks at the
/// FETCH requests of that peer's session, 16 at most, and not at those of
/// every session of the relay.
#[derive(Debug, Default)]
pub(crate) struct Fetches(BTreeMap<(SessionKey, StreamKey), Standing>);

impl Fetches {
    fn len(&self) -> usize {
        self.0.len()
    }

    /// How many FETCH requests of `session` are open.
    fn open_of(&self, session: SessionKey) -> usize {
        self.of(session).count()
    }

    /// Forgets what `session` has that nothing was asked of a publisher
    /// for.
    fn forget_idle_of(&mut self, session: SessionKey) {
        self.0.retain(|(subscriber, _), standing| {
            *subscriber != session || matches!(standing, Standing::Relayed(_))
        });
    }

    fn hold(&mut self, subscriber: (SessionKey, StreamKey), held: HeldFetch) {
        self.0.insert(subscriber, Standing::Held(held));
    }

    fn relay(&mut self, subscriber: (SessionKey, StreamKey), fetch: RelayedFetch) {
        self.0.insert(subscriber, Standing::Relayed(fetch));
    }

    fn write(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        track: FullTrackName,
        holds: Option<(KeptGroup, usize)>,
    ) {
        self.0.insert(
            subscriber,
            Standing::Written {
                track,
                holds,
                told_to_stop: false,
            },
        );
    }

    /// The fetch streams the relay writes and was not told to stop, that
    /// carry Objects: who reads it, the track, the Group and the bytes.
    fn writes(
        &self,
    ) -> impl Iterator<Item = (&(SessionKey, StreamKey), &FullTrackName, KeptGroup, usize)> {
        self.0
            .iter()
            .filter_map(|(subscriber, standing)| match standing {
                Standing::Written {
                    track,
                    holds: Some((group, bytes)),
                    told_to_stop: false,
                } => Some((subscriber, track, *group, *bytes)),
                _ => None,
            })
    }

    /// Whether the relay is writing the answer to a FETCH itself.
    fn is_written(&self, subscriber: &(SessionKey, StreamKey)) -> bool {
        matches!(self.0.get(subscriber), Some(Standing::Written { .. }))
    }

    /// Notes that the I/O side is told to give up the fetch stream of a
    /// FETCH the relay is writing the answer of. Whether it is to be told:
    /// not when there is no such FETCH, and not a second time.
    ///
    /// WHY once: a subscriber that cancels resets its request stream and
    /// stops it, which the I/O side may report as two ends, and a track can
    /// be found malformed after either. The first code is the one the
    /// stream is reset with.
    fn tell_to_stop(&mut self, subscriber: &(SessionKey, StreamKey)) -> bool {
        match self.0.get_mut(subscriber) {
            Some(Standing::Written { told_to_stop, .. }) => !std::mem::replace(told_to_stop, true),
            _ => false,
        }
    }

    /// The FETCH requests for `track` the relay is writing the answer of,
    /// in the order of their keys.
    fn written_for(&self, track: &FullTrackName) -> Vec<(SessionKey, StreamKey)> {
        self.0
            .iter()
            .filter(|(_, standing)| matches!(standing, Standing::Written { track: of, .. } if of == track))
            .map(|(subscriber, _)| *subscriber)
            .collect()
    }

    fn relayed(&self, subscriber: &(SessionKey, StreamKey)) -> Option<&RelayedFetch> {
        match self.0.get(subscriber)? {
            Standing::Relayed(fetch) => Some(fetch),
            Standing::Held(_) | Standing::Written { .. } => None,
        }
    }

    fn relayed_mut(&mut self, subscriber: &(SessionKey, StreamKey)) -> Option<&mut RelayedFetch> {
        match self.0.get_mut(subscriber)? {
            Standing::Relayed(fetch) => Some(fetch),
            Standing::Held(_) | Standing::Written { .. } => None,
        }
    }

    fn held_mut(&mut self, subscriber: &(SessionKey, StreamKey)) -> Option<&mut HeldFetch> {
        match self.0.get_mut(subscriber)? {
            Standing::Held(held) => Some(held),
            Standing::Relayed(_) | Standing::Written { .. } => None,
        }
    }

    /// Lets go of a FETCH that a publisher was asked for, and of no other.
    fn take_relayed(&mut self, subscriber: &(SessionKey, StreamKey)) -> Option<RelayedFetch> {
        match self.0.entry(*subscriber) {
            Entry::Occupied(entry) if matches!(entry.get(), Standing::Relayed(_)) => {
                match entry.remove() {
                    Standing::Relayed(fetch) => Some(fetch),
                    Standing::Held(_) | Standing::Written { .. } => None,
                }
            }
            Entry::Occupied(_) | Entry::Vacant(_) => None,
        }
    }

    /// Lets go of a FETCH that waits for its SUBSCRIBE, and of no other.
    fn take_held(&mut self, subscriber: &(SessionKey, StreamKey)) -> Option<HeldFetch> {
        match self.0.entry(*subscriber) {
            Entry::Occupied(entry) if matches!(entry.get(), Standing::Held(_)) => {
                match entry.remove() {
                    Standing::Held(held) => Some(held),
                    Standing::Relayed(_) | Standing::Written { .. } => None,
                }
            }
            Entry::Occupied(_) | Entry::Vacant(_) => None,
        }
    }

    /// Lets go of a FETCH whose stream the relay wrote itself. Whether
    /// there was one.
    fn finish_written(&mut self, subscriber: &(SessionKey, StreamKey)) -> bool {
        if !self.is_written(subscriber) {
            return false;
        }
        self.0.remove(subscriber).is_some()
    }

    fn all_relayed(&self) -> impl Iterator<Item = (&(SessionKey, StreamKey), &RelayedFetch)> {
        self.0
            .iter()
            .filter_map(|(subscriber, standing)| match standing {
                Standing::Relayed(fetch) => Some((subscriber, fetch)),
                Standing::Held(_) | Standing::Written { .. } => None,
            })
    }

    /// The FETCH requests of `session`, in the order of their request
    /// streams.
    fn of(&self, session: SessionKey) -> impl Iterator<Item = (StreamKey, &Standing)> {
        self.0
            .range((session, StreamKey(0))..=(session, StreamKey(u64::MAX)))
            .map(|((_, stream), standing)| (*stream, standing))
    }

    /// Those of `session` that wait for their SUBSCRIBE, in the order of
    /// their request streams.
    fn held_of(&self, session: SessionKey) -> impl Iterator<Item = (StreamKey, &HeldFetch)> {
        self.of(session)
            .filter_map(|(stream, standing)| match standing {
                Standing::Held(held) => Some((stream, held)),
                Standing::Relayed(_) | Standing::Written { .. } => None,
            })
    }
}

/// What a FETCH of a subscriber asks for, once the relay knows it.
struct AskedRange {
    track: FullTrackName,
    start: Location,
    end: Location,
    /// For a Joining FETCH: the request stream of the SUBSCRIBE it names.
    joins: Option<StreamKey>,
    /// Where the fetch stream stands among the streams to the subscriber
    /// ([`fetch_send_order`]).
    send_order: i32,
}

/// Where the stream that answers a FETCH stands among the streams to its
/// subscriber ([`crate::send_order`]): by the `SUBSCRIBER_PRIORITY` of the
/// FETCH, 128 without one (§10.2.7: "relative to other subscriptions and
/// fetch responses in the same session"), and a Publisher Priority of 128.
///
/// WHY 128 for the publisher's, where §7.1 has "FETCH responses however
/// can contain objects with different publisher priorities": a fetch
/// stream is one stream, the transport takes one number for a stream, and
/// the relay passes its bytes on without reading the Objects' priorities.
/// WHY a Joining FETCH is not put first: §7.2 has only the control stream
/// and the request streams before "all subscribed Objects".
fn fetch_send_order(parameters: &[Parameter]) -> i32 {
    let subscriber_priority = parameters
        .iter()
        .find_map(|parameter| match parameter {
            Parameter::SubscriberPriority(priority) => Some(*priority),
            _ => None,
        })
        .unwrap_or(crate::DEFAULT_PRIORITY);
    crate::send_order(subscriber_priority, crate::DEFAULT_PRIORITY)
}

/// How the answer to a FETCH that has its `FETCH_OK` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnswerEnd {
    /// The fetch stream was passed on or written to its FIN.
    Served,
    /// The fetch stream was reset, or none came: the reset, or the lack of
    /// a stream, has said what happened, and the request stream is finished
    /// at once.
    Cut,
    /// The relay gave up a fetch stream it wrote itself: the request stream
    /// is reset with this code.
    GivenUp(u32),
}

/// The relay's FETCH to a publisher, as the publisher's session holds it.
#[derive(Debug)]
pub(crate) struct OwnFetch {
    id: RequestId,
    /// The session and request stream of the subscriber's FETCH.
    subscriber: (SessionKey, StreamKey),
}

/// A Group as a cache had it: which track with the name (`Track::born`),
/// and the Group ID.
///
/// WHY not the Group ID alone: a track that comes again under the name may
/// count its Groups as the one before did, and the cache of the new track
/// having a Group of that number says nothing of the Objects a stream of
/// the old one carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct KeptGroup {
    track: u32,
    group: u64,
}

/// What the fetch streams the relay writes keep alive beyond the cache.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    /// For the relay: each Group once.
    pub(crate) total: usize,
    /// For each session that has any: each Group it has a stream of.
    sessions: BTreeMap<SessionKey, usize>,
}

impl Held {
    fn of(&self, session: SessionKey) -> usize {
        self.sessions.get(&session).copied().unwrap_or(0)
    }
}

/// The `FILL_TIMEOUT` of a FETCH, in milliseconds (§10.2.5).
fn fill_timeout(parameters: &[Parameter]) -> Option<u64> {
    parameters.iter().find_map(|parameter| match parameter {
        Parameter::FillTimeout(millis) => Some(*millis),
        _ => None,
    })
}

/// What of a subscriber's FETCH goes into the relay's own.
///
/// WHY `GROUP_ORDER`, where §7.3 has "Relays SHOULD NOT directly use
/// Subscriber Priority or Group Order from incoming subscriptions for
/// upstream subscriptions": that is said of subscriptions, whose Objects the
/// relay sends in an order of its own choosing. A fetch stream is passed on
/// as it comes, and "A publisher MUST send fetched groups in the requested
/// group order" (§10.12.3) is kept only if the publisher was asked for that
/// order.
///
/// WHY `FILL_TIMEOUT`: it says how long the subscriber waits, which a relay
/// further upstream can use and an original publisher ignores (§10.2.1:
/// "relays can consider received parameter values"). It is passed on as it
/// came; that relay shortens it as it sees fit.
///
/// WHY NOT `SUBSCRIBER_PRIORITY`: §7.3, and nothing here sends by priority.
/// WHY NOT `AUTHORIZATION_TOKEN`: what a subscriber's token means to a
/// publisher is undecided, as for SUBSCRIBE.
fn upstream_parameters(parameters: &[Parameter]) -> Vec<Parameter> {
    parameters
        .iter()
        .filter(|parameter| {
            matches!(
                parameter,
                Parameter::GroupOrder(_) | Parameter::FillTimeout(_)
            )
        })
        .cloned()
        .collect()
}

/// The range of a Joining FETCH for a subscription with the Joining Location
/// `joining` (§10.12.2.1): Start Location and End Location. `None` for an
/// Absolute Joining FETCH that starts after the Joining Location, which is a
/// range that ends before it starts (§10.12.3).
///
/// WHY Group 0 where a Relative Joining FETCH asks for more Groups back than
/// there are: the draft has "{Joining Location.Group - Joining Start, 0}"
/// and does not say what it is when that cannot be subtracted. The request
/// is for so many Groups before the live edge, and the Groups that exist are
/// what can be had of that. Refusing it with `INVALID_RANGE` would be the
/// other reading.
fn joining_range(range: &FetchRange, joining: Location) -> Option<(Location, Location)> {
    let start_group = match range {
        FetchRange::RelativeJoining { joining_start, .. } => {
            joining.group.saturating_sub(*joining_start)
        }
        FetchRange::AbsoluteJoining { joining_start, .. } => *joining_start,
        FetchRange::Standalone { .. } => return None,
    };
    if start_group > joining.group {
        return None;
    }
    let start = Location {
        group: start_group,
        object: 0,
    };
    // "{Joining Location.Group, Joining Location.Object + 1}": the last
    // Object of the response is the one at the Joining Location. Where that
    // is the last Object ID there is, the number after it cannot be written,
    // and Object 0 says the same: "A Location.Object value of 0 means the
    // entire group is requested" (§10.12.1).
    let end = Location {
        group: joining.group,
        object: joining.object.checked_add(1).unwrap_or(0),
    };
    Some((start, end))
}

/// The End Location of `FETCH_OK` for a FETCH that asked for `end`, where
/// the Largest Object is `largest` (§10.13: "If the requested FETCH End
/// Location was beyond the Largest known (possibly final) Object, End
/// Location is {Largest.Group, Largest.Object + 1}").
fn end_location(end: Location, largest: Location) -> Location {
    if last_location(end) > largest {
        Location {
            group: largest.group,
            // The last Object ID there is: "or 0 to indicate the entire
            // Group" (§10.13).
            object: largest.object.checked_add(1).unwrap_or(0),
        }
    } else {
        end
    }
}

/// The last Location an End Location covers: an End Location names the last
/// Object plus 1, and with Object 0 the whole Group (§10.12.1).
fn last_location(end: Location) -> Location {
    Location {
        group: end.group,
        object: end.object.checked_sub(1).unwrap_or(u64::MAX),
    }
}

impl Relay {
    /// Takes the events that are about a FETCH the relay passes on: the ones
    /// of a fetch stream, and what comes back on the relay's own FETCH.
    /// Returns every other event.
    pub(crate) fn on_fetch_event(
        &mut self,
        event: Event,
        actions: &mut Vec<Action>,
    ) -> Option<Event> {
        match event {
            Event::JoiningFetchTimeout { session, stream } => {
                self.settle_early_fetches(session, Some(stream), actions);
            }
            Event::FetchEndTimeout { session, stream } => {
                self.on_fetch_end_timeout(session, stream, actions);
            }
            Event::FetchStreamOpened {
                session,
                stream,
                request_id,
            } => self.on_fetch_stream_opened(session, stream, request_id, actions),
            Event::FetchPipeEnded {
                session,
                stream,
                how,
            } => self.on_fetch_pipe_ended(session, stream, how, actions),
            Event::FetchStreamWritten {
                session,
                stream,
                given_up,
            } => self.on_fetch_stream_written(session, stream, given_up, actions),
            Event::OwnRequestMessage {
                session,
                request,
                message,
            } if self.is_own_fetch(session, request) => {
                self.on_own_fetch_message(session, request, message, actions);
            }
            Event::OwnRequestClosed {
                session,
                request,
                how,
            } if self.is_own_fetch(session, request) => {
                self.on_own_fetch_closed(session, request, how, actions);
            }
            Event::UpstreamTimeout { session, request } if self.is_own_fetch(session, request) => {
                self.on_fetch_timeout(session, request, actions);
            }
            other => return Some(other),
        }
        None
    }

    /// Tells the Joining FETCH requests that name a SUBSCRIBE what became
    /// of it. Every change of a request slot leads here (module `requests`).
    pub(crate) fn joined(
        &mut self,
        session: SessionKey,
        subscribe: StreamKey,
        change: Joinable,
        actions: &mut Vec<Action>,
    ) {
        match change {
            Joinable::Unchanged => {}
            Joinable::Arrived => self.subscribe_has_come(session, subscribe, actions),
            Joinable::Established => {
                self.subscribe_has_come(session, subscribe, actions);
                self.release_held_fetches(session, subscribe, actions);
            }
            Joinable::Over => self.subscribe_is_over(session, subscribe, actions),
        }
    }

    /// A subscriber sent FETCH.
    pub(crate) fn on_fetch(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        fetch: &Fetch,
        actions: &mut Vec<Action>,
    ) {
        if self.fetches.open_of(session) >= MAX_FETCHES_PER_SESSION {
            let too_many = Refusal::code(request_error_code::EXCESSIVE_LOAD);
            return self.refuse(session, stream, request, &too_many, actions);
        }
        let slot = RequestSlot::Fetch {
            request,
            peer_finished: false,
        };
        self.put_slot(session, stream, slot, actions);
        match &fetch.range {
            FetchRange::Standalone {
                full_track_name,
                start,
                end,
            } => {
                let asked = AskedRange {
                    track: full_track_name.clone(),
                    start: *start,
                    end: *end,
                    joins: None,
                    send_order: fetch_send_order(&fetch.parameters),
                };
                self.ask_for(session, stream, request, &asked, &fetch.parameters, actions);
            }
            FetchRange::RelativeJoining {
                joining_request_id, ..
            }
            | FetchRange::AbsoluteJoining {
                joining_request_id, ..
            } => self.on_joining_fetch(
                session,
                stream,
                request,
                *joining_request_id,
                fetch,
                actions,
            ),
        }
    }

    /// A Joining FETCH (§10.12.2). The session has seen to it that it names
    /// a SUBSCRIBE of the same session that is Pending or Established, or a
    /// Request ID whose request is yet to come.
    fn on_joining_fetch(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        joining_request_id: u64,
        fetch: &Fetch,
        actions: &mut Vec<Action>,
    ) {
        let joined = self.sessions.get(&session).and_then(|state| {
            state.requests.iter().find_map(|(subscribe, slot)| {
                let (request, subscribed) = slot.subscription()?;
                (request.value() == joining_request_id).then_some((*subscribe, subscribed))
            })
        });
        match joined {
            Some((subscribe, Subscribed::Accepted)) => {
                self.join(session, stream, request, subscribe, fetch, actions);
            }
            // §10.12.2: "if that subscription has not yet been established,
            // the Publisher receiving the Joining Fetch buffers the pending
            // Joining Fetch until either the Subscription is established or
            // the request times out". The time is the subscription's: the
            // FETCH is answered when the SUBSCRIBE is. A SUBSCRIBE waits
            // only on a track that publishers are asked for, and each of
            // those requests has the upstream timer running, from when it
            // was sent and anew from a `DUPLICATE_SUBSCRIPTION` that
            // promises a PUBLISH; when the last is over the SUBSCRIBE is
            // refused, and this FETCH with it. What the wait has no limit
            // for is publishers that keep coming: each one that announces
            // the namespace meanwhile is asked, with a timer of its own.
            Some((subscribe, Subscribed::Waiting)) => {
                self.fetches.hold(
                    (session, stream),
                    HeldFetch {
                        request,
                        joins: Awaited::Answer(subscribe),
                        fetch: fetch.clone(),
                        held_updates: 0,
                    },
                );
            }
            // No SUBSCRIBE with that Request ID is here. One that is yet to
            // come is waited for: the two travel on streams of their own,
            // and a subscriber that sends them together cannot tell which
            // the relay reads first (§10.12.2 says nothing of it).
            //
            // WHY not put the request streams in the order of their Request
            // IDs before the relay sees them: every request would wait for
            // one that is late, also the many that name no other.
            None if self.sessions.get(&session).is_some_and(|state| {
                state
                    .moqt
                    .peer_request_yet_to_come(joining_request_id, request)
            }) =>
            {
                self.fetches.hold(
                    (session, stream),
                    HeldFetch {
                        request,
                        joins: Awaited::Subscribe(joining_request_id),
                        fetch: fetch.clone(),
                        held_updates: 0,
                    },
                );
                actions.push(Action::StartJoiningFetchTimer { session, stream });
            }
            // The ID was used by something that is no subscription now: a
            // SUBSCRIBE that is over, or another kind of request.
            None => {
                let gone = Refusal::code(request_error_code::INVALID_JOINING_REQUEST_ID);
                self.refuse(session, stream, request, &gone, actions);
            }
        }
    }

    /// Turns a Joining FETCH for an Established subscription into a range of
    /// the track and asks a publisher for it.
    ///
    /// WHY a Standalone FETCH upstream and not a Joining one: the range is
    /// counted from the Joining Location, which is each subscription's own,
    /// "the Largest Location communicated in `SUBSCRIBE_OK`" (§5.1). The
    /// relay's subscription to the publisher has another than the
    /// subscriber's to the relay, and a publisher asked to count from that
    /// would send another range.
    fn join(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        subscribe: StreamKey,
        fetch: &Fetch,
        actions: &mut Vec<Action>,
    ) {
        let subscription = self.sessions.get(&session).and_then(|state| {
            let RequestSlot::Subscribe { track, .. } = state.requests.get(subscribe)? else {
                return None;
            };
            let subscriber = self
                .tracks
                .live(track)?
                .subscribers
                .iter()
                .find(|subscriber| {
                    subscriber.session == session && subscriber.stream == subscribe
                })?;
            Some((track.clone(), subscriber.forward, subscriber.joining))
        });
        let Some((track, forward, joining)) = subscription else {
            let gone = Refusal::code(request_error_code::INVALID_JOINING_REQUEST_ID);
            return self.refuse(session, stream, request, &gone, actions);
        };
        // §10.12.2: "A Joining Fetch is only permitted when the associated
        // subscription has Forward State 1; otherwise the publisher MUST
        // respond with a REQUEST_ERROR with error code INVALID_RANGE", and
        // "If no Objects have been published for the track the publisher
        // MUST respond with a REQUEST_ERROR with error code INVALID_RANGE".
        // What is looked at for the second is the subscription: its
        // `SUBSCRIBE_OK` had no `LARGEST_OBJECT`, so it has no Joining
        // Location (§5.1), whatever was published since.
        let range = joining
            .filter(|_| forward)
            .and_then(|joining| joining_range(&fetch.range, joining));
        let Some((start, end)) = range else {
            let invalid = Refusal::code(request_error_code::INVALID_RANGE);
            return self.refuse(session, stream, request, &invalid, actions);
        };
        let asked = AskedRange {
            track,
            start,
            end,
            joins: Some(subscribe),
            send_order: fetch_send_order(&fetch.parameters),
        };
        self.ask_for(session, stream, request, &asked, &fetch.parameters, actions);
    }

    /// Asks a publisher for a range of a track, for a FETCH of a subscriber.
    fn ask_for(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        asked: &AskedRange,
        parameters: &[Parameter],
        actions: &mut Vec<Action>,
    ) {
        let refuse = |relay: &mut Self, error_code, actions: &mut Vec<Action>| {
            relay.refuse(
                session,
                stream,
                request,
                &Refusal::code(error_code),
                actions,
            );
        };
        let fill_timeout = fill_timeout(parameters);
        if fill_timeout == Some(0) {
            let range = (asked.start, asked.end, asked.send_order);
            return self.answer_without_asking(
                session,
                stream,
                request,
                &asked.track,
                range,
                actions,
            );
        }
        if let Some((end, objects)) = self.cached(asked) {
            // §10.6.2: "EXCESSIVE_LOAD: The responder is overloaded and
            // cannot process the request at this time."
            let held = self.held_beyond_the_cache();
            if held.total >= self.held_beyond_cache_limit
                || held.of(session) >= self.held_beyond_cache_share
            {
                actions.push(Action::NoteHeldBeyondCache {
                    session,
                    stream,
                    refused: true,
                    relay_bytes: held.total,
                    session_bytes: held.of(session),
                });
                return refuse(self, request_error_code::EXCESSIVE_LOAD, actions);
            }
            let pieces = objects.into_iter().map(FetchPiece::Object).collect();
            let answer = (end, pieces, asked.send_order);
            return self.write_answer(session, stream, request, &asked.track, answer, actions);
        }
        let parameters = upstream_parameters(parameters);
        let mut asked_of = Vec::new();
        let range = (asked.start, asked.end);
        let found = self.ask_in_turn(
            (session, stream),
            &asked.track,
            range,
            &parameters,
            &mut asked_of,
            actions,
        );
        let (publisher, own) = match found {
            Ok(found) => found,
            Err(code) => {
                let code = code.unwrap_or(request_error_code::DOES_NOT_EXIST);
                return refuse(self, code, actions);
            }
        };
        self.fetches.relay(
            (session, stream),
            RelayedFetch {
                request,
                track: asked.track.clone(),
                publisher,
                own,
                answered: false,
                pipe: Pipe::NotOpen,
                upstream_wait_ms: fill_timeout
                    .unwrap_or(FETCH_UPSTREAM_WAIT_MS)
                    .min(MAX_FETCH_UPSTREAM_WAIT_MS),
                held_updates: 0,
                joins: asked.joins,
                send_order: asked.send_order,
                start: asked.start,
                end: asked.end,
                parameters,
                asked: asked_of,
            },
        );
    }

    /// Asks the publishers of the track one after another, those in `asked`
    /// left out, until one can be sent the FETCH. Returns that publisher and
    /// the relay's request; or, where none could, the code of the last
    /// refusal the relay made itself, and `None` where there was nobody to
    /// ask. Each publisher tried is added to `asked`.
    fn ask_in_turn(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        track: &FullTrackName,
        range: (Location, Location),
        parameters: &[Parameter],
        asked: &mut Vec<SessionKey>,
        actions: &mut Vec<Action>,
    ) -> Result<(SessionKey, OwnRequestKey), Option<u64>> {
        let mut refused = None;
        while let Some(publisher) = self.whom_to_ask(track, subscriber.0, asked) {
            asked.push(publisher);
            match self.ask_one(subscriber, publisher, track, range, parameters, actions) {
                Ok(own) => return Ok((publisher, own)),
                Err(code) => refused = Some(code),
            }
        }
        Err(refused)
    }

    /// Sends one publisher the relay's FETCH for a subscriber's, or says
    /// with which code the relay refuses to.
    fn ask_one(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        publisher: SessionKey,
        track: &FullTrackName,
        (start, end): (Location, Location),
        parameters: &[Parameter],
        actions: &mut Vec<Action>,
    ) -> Result<OwnRequestKey, u64> {
        let fetches_for_one = self.fetches_for_one;
        let Some(state) = self.sessions.get_mut(&publisher) else {
            return Err(request_error_code::DOES_NOT_EXIST);
        };
        let for_this_session = state
            .own_fetches
            .values()
            .filter(|own| own.subscriber.0 == subscriber.0)
            .count();
        // §10.4: no new request to a peer that sent GOAWAY, as for a
        // SUBSCRIBE of the relay (`Relay::ask`).
        if state.moqt.peer_going_away() {
            return Err(request_error_code::DOES_NOT_EXIST);
        }
        // The relay's FETCH and SUBSCRIBE requests share the publisher's
        // session, and its patience.
        if state.own_requests() >= MAX_OWN_REQUESTS_PER_SESSION
            || state.own_fetches.len() >= MAX_OWN_FETCHES_PER_SESSION
            || for_this_session >= fetches_for_one
        {
            return Err(request_error_code::EXCESSIVE_LOAD);
        }
        let range = FetchRange::Standalone {
            full_track_name: track.clone(),
            start,
            end,
        };
        let Ok((id, message)) = state.moqt.fetch(range, parameters.to_vec()) else {
            // The range and the parameters passed the session's checks when
            // they came in, or were made here.
            return Err(request_error_code::INTERNAL_ERROR);
        };
        let own = OwnRequestKey(self.next_own_request);
        self.next_own_request = self.next_own_request.saturating_add(1);
        state.own_fetches.insert(own, OwnFetch { id, subscriber });
        actions.push(Action::OpenRequest {
            session: publisher,
            request: own,
            message,
        });
        actions.push(Action::StartUpstreamTimer {
            session: publisher,
            request: own,
        });
        Ok(own)
    }

    /// The publisher a FETCH was asked of refused it, or went, before it
    /// answered: the next publisher of the track is asked
    /// ([`Self::whom_to_ask`]). Where there is none, the subscriber is
    /// refused with `refusal`. A FETCH that was answered, or whose fetch
    /// stream has come, fails as [`Self::fail_fetch`] has it.
    ///
    /// WHY NOT also after a publisher that did not answer in time: each
    /// publisher gets the whole wait, and a subscriber would wait as many
    /// times as there are publishers. A refusal or a session that ends comes
    /// at once.
    fn ask_the_next_or_fail(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        refusal: &Refusal,
        pipe_code: u32,
        actions: &mut Vec<Action>,
    ) {
        let unanswered = self
            .fetches
            .relayed(&subscriber)
            .is_some_and(|fetch| !fetch.answered && fetch.pipe == Pipe::NotOpen);
        if !unanswered {
            return self.fail_fetch(subscriber, refusal, pipe_code, actions);
        }
        let Some(mut fetch) = self.fetches.take_relayed(&subscriber) else {
            return;
        };
        self.cancel_upstream(&fetch, actions);
        let track = fetch.track.clone();
        let parameters = std::mem::take(&mut fetch.parameters);
        let range = (fetch.start, fetch.end);
        let found = self.ask_in_turn(
            subscriber,
            &track,
            range,
            &parameters,
            &mut fetch.asked,
            actions,
        );
        fetch.parameters = parameters;
        match found {
            Ok((publisher, own)) => {
                fetch.publisher = publisher;
                fetch.own = own;
                self.fetches.relay(subscriber, fetch);
            }
            Err(Some(code)) => {
                let (session, stream) = subscriber;
                self.refuse(
                    session,
                    stream,
                    fetch.request,
                    &Refusal::code(code),
                    actions,
                );
            }
            Err(None) => {
                let (session, stream) = subscriber;
                self.refuse(session, stream, fetch.request, refusal, actions);
            }
        }
    }

    /// The SUBSCRIBE on `subscribe` has come: the Joining FETCH requests that
    /// were read before it wait for its answer from here on.
    fn subscribe_has_come(
        &mut self,
        session: SessionKey,
        subscribe: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let request_id = self
            .sessions
            .get(&session)
            .and_then(|state| state.requests.get(subscribe))
            .and_then(RequestSlot::subscription)
            .map(|(request, _)| request.value());
        let Some(request_id) = request_id else {
            return;
        };
        let early: Vec<StreamKey> = self
            .fetches
            .held_of(session)
            .filter(|(_, held)| held.joins == Awaited::Subscribe(request_id))
            .map(|(stream, _)| stream)
            .collect();
        for stream in early {
            if let Some(held) = self.fetches.held_mut(&(session, stream)) {
                held.joins = Awaited::Answer(subscribe);
                actions.push(Action::StopJoiningFetchTimer { session, stream });
            }
        }
    }

    /// The peer used a Request ID, or the time for a SUBSCRIBE to come ran
    /// out (`timed_out` names the FETCH): the Joining FETCH requests of
    /// `session` that still wait for a SUBSCRIBE to come, and whose
    /// SUBSCRIBE cannot come any more, are refused with
    /// `INVALID_JOINING_REQUEST_ID`.
    ///
    /// Called after every request and message of the peer. A SUBSCRIBE that
    /// came has taken its FETCH requests along before this is reached
    /// ([`Self::subscribe_has_come`]), so what is left with a Request ID
    /// that was used names something that is no subscription.
    pub(crate) fn settle_early_fetches(
        &mut self,
        session: SessionKey,
        timed_out: Option<StreamKey>,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get(&session) else {
            return;
        };
        let missed: Vec<StreamKey> = self
            .fetches
            .held_of(session)
            .filter(|(stream, held)| {
                let Awaited::Subscribe(request_id) = held.joins else {
                    return false;
                };
                timed_out == Some(*stream)
                    || !state
                        .moqt
                        .peer_request_yet_to_come(request_id, held.request)
            })
            .map(|(stream, _)| stream)
            .collect();
        let gone = Refusal::code(request_error_code::INVALID_JOINING_REQUEST_ID);
        for stream in missed {
            // The one whose time ran out has no timer left to drop.
            let held = if timed_out == Some(stream) {
                self.fetches.take_held(&(session, stream))
            } else {
                self.let_go_of_held(&(session, stream), actions)
            };
            if let Some(held) = held {
                self.refuse(session, stream, held.request, &gone, actions);
            }
        }
    }

    /// Takes a held FETCH out of the relay's hands; the timer of one that
    /// still waited for its SUBSCRIBE to come is dropped with it.
    fn let_go_of_held(
        &mut self,
        subscriber: &(SessionKey, StreamKey),
        actions: &mut Vec<Action>,
    ) -> Option<HeldFetch> {
        let held = self.fetches.take_held(subscriber)?;
        if matches!(held.joins, Awaited::Subscribe(_)) {
            let (session, stream) = *subscriber;
            actions.push(Action::StopJoiningFetchTimer { session, stream });
        }
        Some(held)
    }

    /// The SUBSCRIBE on `subscribe` was accepted: the Joining FETCH requests that
    /// waited for it are asked of a publisher.
    fn release_held_fetches(
        &mut self,
        session: SessionKey,
        subscribe: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        for (stream, held) in self.take_held(session, subscribe) {
            self.join(
                session,
                stream,
                held.request,
                subscribe,
                &held.fetch,
                actions,
            );
            // The updates that came while it waited go on waiting, for the
            // publisher's answer.
            if let Some(fetch) = self.fetches.relayed_mut(&(session, stream)) {
                fetch.held_updates = held.held_updates;
            }
        }
    }

    /// The subscription on `subscribe` is over: refused, ended with
    /// `PUBLISH_DONE`, or cancelled.
    ///
    /// Its Joining FETCH requests that have no `FETCH_OK` yet go with it:
    /// the ones that waited for it, and the ones a publisher is asked for.
    /// The subscription they name is not there (§10.12.2,
    /// `INVALID_JOINING_REQUEST_ID`), and the session makes no `FETCH_OK`
    /// for a Joining FETCH whose SUBSCRIBE is over.
    ///
    /// WHY one that has its `FETCH_OK` goes on: §10.12.2 has "Changing the
    /// Forward State of the associated subscription to 0 after the Joining
    /// Fetch has been accepted has no effect on the Joining Fetch", and
    /// says nothing of a subscription that ends. What was accepted is a
    /// range of the track, which the end of the subscription does not
    /// change; and a subscriber the relay drops as too far behind while it
    /// reads a large FETCH would lose the past along with the present.
    fn subscribe_is_over(
        &mut self,
        session: SessionKey,
        subscribe: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let gone = Refusal::code(request_error_code::INVALID_JOINING_REQUEST_ID);
        for (stream, held) in self.take_held(session, subscribe) {
            self.refuse(session, stream, held.request, &gone, actions);
        }
        let mut joined: Vec<StreamKey> = self
            .fetches
            .all_relayed()
            .filter(|((subscriber, _), fetch)| {
                *subscriber == session && fetch.joins == Some(subscribe) && !fetch.answered
            })
            .map(|((_, stream), _)| *stream)
            .collect();
        joined.sort_unstable_by_key(|stream| stream.0);
        for stream in joined {
            self.fail_fetch(
                (session, stream),
                &gone,
                stream_reset_code::CANCELLED,
                actions,
            );
        }
    }

    /// Takes the Joining FETCH requests of `session` that wait for the SUBSCRIBE on
    /// `subscribe`, in the order of their request streams.
    fn take_held(
        &mut self,
        session: SessionKey,
        subscribe: StreamKey,
    ) -> Vec<(StreamKey, HeldFetch)> {
        let streams: Vec<StreamKey> = self
            .fetches
            .held_of(session)
            .filter(|(_, held)| held.joins == Awaited::Answer(subscribe))
            .map(|(stream, _)| stream)
            .collect();
        streams
            .into_iter()
            .filter_map(|stream| Some((stream, self.fetches.take_held(&(session, stream))?)))
            .collect()
    }

    /// The publisher a FETCH for the track is sent to next (§9.5: "When
    /// more than one publisher is available, the Relay MAY send the FETCH to
    /// any of them"): the first source of the track that is not on its way
    /// out, then the first publisher that announced a matching namespace;
    /// neither the subscriber nor one in `asked`.
    ///
    /// WHY the next one after a refusal, where the draft has only the first
    /// ask: the sources of one track can differ in what they keep and in
    /// whether they serve a FETCH at all (the own Rust client serves none),
    /// and a subscriber that is refused while another publisher has the
    /// Objects cannot reach that one by itself.
    fn whom_to_ask(
        &self,
        track: &FullTrackName,
        subscriber: SessionKey,
        asked: &[SessionKey],
    ) -> Option<SessionKey> {
        let left = |session: &SessionKey| *session != subscriber && !asked.contains(session);
        let source = self.tracks.live(track).and_then(|live| {
            live.sources()
                .filter(|source| !source.draining)
                .map(|source| source.session_and_alias().0)
                .find(left)
        });
        source.or_else(|| self.matching_publishers(track).find(left))
    }

    /// A FETCH with `FILL_TIMEOUT` 0 (§10.2.5): "the relay MUST NOT wait for
    /// upstream delivery and MUST report any unavailable Objects as Unknown
    /// gaps". The answer is a fetch stream with one End of Unknown Range
    /// indicator for the whole range.
    ///
    /// FIXME: give the Objects the cache holds of the range, and the
    /// indicator for the rest.
    ///
    /// The `FETCH_OK` needs an End Location, which the relay knows only for
    /// a track it has a source of, from the largest Location it has seen.
    /// WHY `TIMEOUT` where it does not: "the `FETCH_OK` MUST NOT be sent
    /// until the End Location is known" (§10.13), learning it means asking
    /// the publisher and waiting for the answer, and the subscriber said it
    /// waits for nothing. WHY NOT ask anyway, reading "upstream delivery" as
    /// the delivery of Objects only: waiting for the answer is waiting.
    fn answer_without_asking(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        track: &FullTrackName,
        (start, end, send_order): (Location, Location, i32),
        actions: &mut Vec<Action>,
    ) {
        let largest = self.tracks.live(track).and_then(|live| live.largest);
        let refusal = match largest {
            // §10.12.3: "If ... Start Location is greater than the Largest
            // Object ... the publisher MUST return REQUEST_ERROR with error
            // code INVALID_RANGE."
            Some(largest) if start > largest => Some(request_error_code::INVALID_RANGE),
            Some(_) => None,
            // A source that has said nothing of a largest Location and sent
            // no Object has published none, as far as the relay knows
            // (§10.2.11: "If omitted from a message, the sending endpoint has
            // not published or received any Objects in the Track"), and
            // §10.12.3 has `INVALID_RANGE` for that.
            None if self.tracks.live(track).is_some() => Some(request_error_code::INVALID_RANGE),
            None if self.whom_to_ask(track, session, &[]).is_none() => {
                Some(request_error_code::DOES_NOT_EXIST)
            }
            // A publisher of the namespace, of which the relay knows nothing
            // about the track.
            None => Some(request_error_code::TIMEOUT),
        };
        let (Some(largest), None) = (largest, refusal) else {
            let error_code = refusal.unwrap_or(request_error_code::INTERNAL_ERROR);
            self.refuse(
                session,
                stream,
                request,
                &Refusal::code(error_code),
                actions,
            );
            return;
        };
        let end = end_location(end, largest);
        // An indicator is a Location, written the same in either Group
        // Order. WHY the last Location of the range also for a Descending
        // FETCH, which begins there: §11.4.4.2 has an indicator cover what
        // lies "between the last serialized Object, if any, and this
        // Location", and does not say from where when no Object came before
        // and the Groups run downwards. A subscriber may read this one as
        // covering nothing, or everything.
        let unknown = FetchPiece::EndOfRange(RangeEnd::Unknown, last_location(end));
        self.write_answer(
            session,
            stream,
            request,
            track,
            (end, vec![unknown], send_order),
            actions,
        );
    }

    /// What the cache of the track has for the range of a FETCH: the End
    /// Location of `FETCH_OK` and the Objects. `None` where the FETCH is a
    /// publisher's to answer ([`cache::Cache::answer`]).
    ///
    /// The End Location is the one §10.13 has, counted from the Largest
    /// Object the relay tells subscribers. WHY that matters to the cache:
    /// what lies "between the last object in the stream and the Largest
    /// Group/Object indicated in `FETCH_OK`" reads as not existing (§10.12.3),
    /// so the cache has to answer for every Location up to there, also where
    /// a publisher said that the track is further than the Objects the relay
    /// received.
    fn cached(&self, asked: &AskedRange) -> Option<(Location, Vec<cache::CachedObject>)> {
        let live = self.tracks.live(&asked.track)?;
        let largest = live.largest?;
        if live.never_cached {
            return None;
        }
        let end = end_location(asked.end, largest);
        let objects = live.cache.answer(asked.start, last_location(end))?;
        Some((end, objects))
    }

    /// Answers a FETCH with `FETCH_OK` and a fetch stream the relay writes
    /// itself: the End Location and what the stream carries.
    ///
    /// End Of Track is 1 where the range reaches the Object with the status
    /// `END_OF_TRACK` that the relay received (§10.13: "1 if all Objects have
    /// been published on this Track, and the End Location is the final Object
    /// in the Track"; §2.4.2: "The final Object in a Track is the Object with
    /// Status `END_OF_TRACK`"). The End Location is never beyond it: the
    /// largest Location of the track is that Object from then on.
    fn write_answer(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        track: &FullTrackName,
        (end, pieces, send_order): (Location, Vec<FetchPiece>, i32),
        actions: &mut Vec<Action>,
    ) {
        // §9.6: "Relays MUST forward all Track Properties ... in the
        // responses to requests for that Track".
        let live = self.tracks.live(track);
        let properties = live.map(|live| live.properties.clone()).unwrap_or_default();
        let end_of_track = live
            .and_then(|live| live.final_object)
            .is_some_and(|last| last_location(end) >= last);
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let request_id = request.value();
        let Ok((ok, owed)) = state
            .moqt
            .accept_fetch(request, end_of_track, end, properties)
        else {
            // As where a publisher's answer cannot be passed on: this FETCH
            // alone.
            let failed = Refusal::code(request_error_code::INTERNAL_ERROR);
            return self.refuse(session, stream, request, &failed, actions);
        };
        self.finish_slot(session, stream, Some(request), actions);
        actions.push(Action::SendRequest {
            session,
            stream,
            message: ok,
        });
        acknowledge_updates(session, stream, owed, actions);
        // The Objects of the answer are of one Group, the one the cache
        // has.
        let payload = |piece: &FetchPiece| match piece {
            FetchPiece::Object(cached) => Some((cached.group_id, cached.object.payload.len())),
            FetchPiece::EndOfRange(..) => None,
        };
        let born = self.tracks.live(track).map_or(0, |live| live.born);
        let holds = pieces.iter().find_map(payload).map(|(group, _)| {
            (
                KeptGroup { track: born, group },
                pieces
                    .iter()
                    .filter_map(payload)
                    .map(|(_, bytes)| bytes)
                    .sum(),
            )
        });
        actions.push(Action::WriteFetchStream {
            session,
            stream,
            request_id,
            pieces,
            wait_ms: FETCH_DOWNSTREAM_WAIT_MS,
            send_order,
        });
        // Counted as one of the session's FETCHes until its stream is
        // written, or given up. The request stream is ended then.
        self.fetches.write((session, stream), track.clone(), holds);
    }

    /// What the fetch streams the relay writes keep alive of Groups the
    /// cache has let go of ([`MAX_BYTES_HELD_BEYOND_THE_CACHE`]).
    ///
    /// WHY worked out anew each time, and not kept as a number: it changes
    /// when a stream ends, when a Group is dropped and when a track ends,
    /// and a number kept beside those would have to be right in each. The
    /// streams are 16 for a session at most.
    pub(crate) fn held_beyond_the_cache(&self) -> Held {
        // The largest stream of each Group, for the relay and for each
        // session.
        let mut groups: BTreeMap<(&FullTrackName, KeptGroup), usize> = BTreeMap::new();
        let mut of_sessions: BTreeMap<(SessionKey, &FullTrackName, KeptGroup), usize> =
            BTreeMap::new();
        for ((session, _), track, group, bytes) in self.fetches.writes() {
            if self.cache_has(track, group) {
                continue;
            }
            let most = groups.entry((track, group)).or_default();
            *most = (*most).max(bytes);
            let most = of_sessions.entry((*session, track, group)).or_default();
            *most = (*most).max(bytes);
        }
        let mut sessions: BTreeMap<SessionKey, usize> = BTreeMap::new();
        for ((session, _, _), bytes) in of_sessions {
            *sessions.entry(session).or_default() += bytes;
        }
        Held {
            total: groups.values().sum(),
            sessions,
        }
    }

    /// Whether the cache of the track with that name still has the Group:
    /// the same track, and the Group it keeps.
    fn cache_has(&self, track: &FullTrackName, kept: KeptGroup) -> bool {
        self.tracks
            .live(track)
            .is_some_and(|live| live.born == kept.track && live.cache.keeps(kept.group))
    }

    /// The cache of a track has let go of a Group, or a track is over: the
    /// fetch streams that carry what was let go of keep it alive from here
    /// on. Where that is more than a session may have kept, or the relay,
    /// streams are reset until it is not.
    ///
    /// WHY looked at here, and not only when a FETCH comes: a FETCH is
    /// answered from the Group the cache has, which costs nothing then, and
    /// becomes what keeps the Group when the cache moves on.
    pub(crate) fn trim_what_is_held_beyond_the_cache(&mut self, actions: &mut Vec<Action>) {
        let share = self.held_beyond_cache_share;
        loop {
            let held = self.held_beyond_the_cache();
            // The session that has the most over its share; then, where
            // the relay as a whole has too much, the one that has the most.
            let over_its_share = held
                .sessions
                .iter()
                .filter(|(_, bytes)| **bytes > share)
                .max_by_key(|(session, bytes)| (**bytes, std::cmp::Reverse(**session)));
            let most = held
                .sessions
                .iter()
                .max_by_key(|(session, bytes)| (**bytes, std::cmp::Reverse(**session)));
            let Some((&session, _)) =
                over_its_share.or(most.filter(|_| held.total > self.held_beyond_cache_limit))
            else {
                return;
            };
            // Of its streams of Groups the cache has let go of, the one that
            // carries the most, so that as few as can be are stopped; of
            // those that carry as much, the one on the request stream it
            // opened last.
            let cut = self
                .fetches
                .writes()
                .filter(|((of, _), track, group, _)| {
                    *of == session && !self.cache_has(track, *group)
                })
                .max_by_key(|(subscriber, _, _, bytes)| (*bytes, **subscriber))
                .map(|(subscriber, ..)| *subscriber);
            let Some(subscriber) = cut else {
                return;
            };
            // Never false: `writes` leaves out what was told to stop. WHY
            // looked at all the same: a stream that could not be told would
            // be found again and again.
            if !self.fetches.tell_to_stop(&subscriber) {
                return;
            }
            {
                actions.push(Action::NoteHeldBeyondCache {
                    session: subscriber.0,
                    stream: subscriber.1,
                    refused: false,
                    relay_bytes: held.total,
                    session_bytes: held.of(subscriber.0),
                });
                actions.push(Action::AbortFetchWrite {
                    session: subscriber.0,
                    stream: subscriber.1,
                    code: stream_reset_code::EXCESSIVE_LOAD,
                });
            }
        }
    }

    /// The subscriber cancelled a FETCH the relay is writing the answer of
    /// (§5.2: "It MUST reset the bidi request stream and unidirectional data
    /// stream associated with the FETCH"). The I/O side reports the stream
    /// given up, which lets go of the FETCH.
    pub(crate) fn cancel_written_fetch(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        if self.fetches.tell_to_stop(&(session, stream)) {
            actions.push(Action::AbortFetchWrite {
                session,
                stream,
                code: stream_reset_code::CANCELLED,
            });
        }
    }

    /// The I/O side is done with a fetch stream the relay made itself.
    ///
    /// WHY the request stream is ended here and not with the `FETCH_OK`: a
    /// subscriber that got `FETCH_OK` and a FIN is owed a fetch stream, and
    /// one that gives the relay no stream to write on would wait for it
    /// with nothing left to tell it by. The reset says that none comes.
    fn on_fetch_stream_written(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        given_up: Option<u32>,
        actions: &mut Vec<Action>,
    ) {
        // Not there when the session is gone.
        if !self.fetches.finish_written(&(session, stream)) {
            return;
        }
        let how = match given_up {
            None => AnswerEnd::Served,
            Some(code) => AnswerEnd::GivenUp(code),
        };
        self.end_answered_fetch(session, stream, how, actions);
    }

    /// Ends the answer of a FETCH that has its `FETCH_OK`, and with it the
    /// relay's direction of the request stream, while the session goes on
    /// holding the request: with a FIN at once, with a FIN after the
    /// subscriber has ended its own direction or
    /// [`FETCH_PEER_END_WAIT_MS`] have passed, or with a reset
    /// ([`AnswerEnd`]).
    ///
    /// Every FIN of an answered FETCH goes through here or follows from
    /// here (a stream that was passed on to its end, and one the relay
    /// wrote), and the reset of a stream the relay wrote and the subscriber
    /// did not take. The session
    /// has to hear of it (`Session::on_fetch_answer_ended`): until it has,
    /// it answers a `REQUEST_UPDATE` of the subscriber with a `REQUEST_OK`
    /// that the relay has no direction left to send on.
    ///
    /// WHY NOT the other resets of an answered FETCH (`Self::fail_fetch`
    /// where no stream came, `Self::drop_fetch` for a subscriber that
    /// stalled or is gone): they forget the request at once, in the session
    /// and in the slot. The reset stops the subscriber's direction as well,
    /// so what follows on it is not read; a `REQUEST_UPDATE` that was read
    /// before the stop took effect finds no request and uses up no Request
    /// ID there, where it does after the reset of this function. Which of
    /// the two a reset should be is not decided, since neither can count
    /// what the stop kept from being read (implementation notes, row on
    /// what still comes on the stream of a request that was refused).
    fn end_answered_fetch(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        how: AnswerEnd,
        actions: &mut Vec<Action>,
    ) {
        // The slot is gone where the subscriber has ended its direction: the
        // session was told that the stream is gone, then.
        let mut peer_is_there = false;
        if let Some(state) = self.sessions.get_mut(&session)
            && let Some(RequestSlot::Finished(Some(request))) = state.requests.get(stream)
        {
            let request = *request;
            // From here on a `REQUEST_UPDATE` gets no answer, also while
            // the relay's direction stays open below: the answer of the
            // FETCH has ended.
            state.moqt.on_fetch_answer_ended(request);
            peer_is_there = true;
        }
        actions.push(match how {
            // The relay's direction stays open with the subscriber's, for a
            // time ([`FETCH_PEER_END_WAIT_MS`]).
            AnswerEnd::Served if peer_is_there => {
                self.fetch_ends.insert((session, stream));
                Action::StartFetchEndTimer {
                    session,
                    stream,
                    wait_ms: FETCH_PEER_END_WAIT_MS,
                }
            }
            AnswerEnd::Served | AnswerEnd::Cut => Action::FinishRequest { session, stream },
            // The I/O side stops the subscriber's direction with the reset.
            AnswerEnd::GivenUp(code) => Action::ResetRequest {
                session,
                stream,
                code,
            },
        });
    }

    /// The subscriber of a FETCH that was served has not ended its
    /// direction of the request stream in time: the relay finishes its own,
    /// and the subscriber's is stopped when the time for that is over as
    /// well (`Relay::on_request_timeout`).
    fn on_fetch_end_timeout(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        // Not there when the subscriber ended its direction meanwhile, and
        // when the session is gone.
        if !self.fetch_ends.remove(&(session, stream)) {
            return;
        }
        actions.push(Action::FinishRequest { session, stream });
    }

    /// The track was found malformed, by what a subscription to it brought
    /// (§2.4.2): "If a relay detects a Malformed Track, it MUST immediately
    /// terminate downstream subscriptions with `PUBLISH_DONE` and reset any
    /// fetch streams with Status Code `MALFORMED_TRACK`."
    pub(crate) fn fetches_of_a_malformed_track(
        &mut self,
        track: &FullTrackName,
        actions: &mut Vec<Action>,
    ) {
        let mut of_the_track: Vec<(SessionKey, StreamKey)> = self
            .fetches
            .all_relayed()
            .filter(|(_, fetch)| fetch.track == *track)
            .map(|(subscriber, _)| *subscriber)
            .collect();
        of_the_track.sort_unstable_by_key(|(session, stream)| (session.0, stream.0));
        let malformed = Refusal::code(request_error_code::MALFORMED_TRACK);
        for subscriber in of_the_track {
            self.fail_fetch(
                subscriber,
                &malformed,
                stream_reset_code::MALFORMED_TRACK,
                actions,
            );
        }
        // The ones the relay answers from its cache have their `FETCH_OK`;
        // their streams are reset, the request stream when the I/O side
        // reports the fetch stream given up.
        for (session, stream) in self.fetches.written_for(track) {
            if !self.fetches.tell_to_stop(&(session, stream)) {
                continue;
            }
            actions.push(Action::AbortFetchWrite {
                session,
                stream,
                code: stream_reset_code::MALFORMED_TRACK,
            });
        }
    }

    /// Whether a request of the relay on `session` is a FETCH.
    fn is_own_fetch(&self, session: SessionKey, own: OwnRequestKey) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|state| state.own_fetches.contains_key(&own))
    }

    /// Forgets a relayed FETCH on the publisher's side: the relay's request
    /// is cancelled, and a fetch stream that is not passed on yet is stopped
    /// when it comes.
    fn cancel_upstream(&mut self, fetch: &RelayedFetch, actions: &mut Vec<Action>) {
        let (session, request) = (fetch.publisher, fetch.own);
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        if let Some(own) = state.own_fetches.remove(&request) {
            state.moqt.cancel(own.id);
            actions.push(Action::CancelOwnRequest {
                session,
                request,
                code: stream_reset_code::CANCELLED,
            });
        }
    }

    /// Ends a relayed FETCH that did not come to its end: the subscriber is
    /// refused if it has no answer yet, and what is passed on is cut with
    /// `pipe_code`.
    fn fail_fetch(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        refusal: &Refusal,
        pipe_code: u32,
        actions: &mut Vec<Action>,
    ) {
        let Some(fetch) = self.fetches.take_relayed(&subscriber) else {
            return;
        };
        let (session, stream) = subscriber;
        if fetch.pipe == Pipe::Running {
            actions.push(Action::AbortFetchPipe {
                session,
                stream,
                code: pipe_code,
            });
        }
        self.cancel_upstream(&fetch, actions);
        match (fetch.answered, fetch.pipe) {
            (false, _) => self.refuse(session, stream, fetch.request, refusal, actions),
            // WHY a reset of the request stream: the subscriber has its
            // FETCH_OK and waits for a fetch stream, which it is owed also
            // for a range without Objects (§10.12.3: "the publisher opens
            // the unidirectional stream, sends the FETCH_HEADER ... and
            // closes the stream with a FIN"). No stream will come, and a FIN
            // on the request stream would not say so.
            (true, Pipe::NotOpen) => {
                self.take_slot(session, stream, actions);
                if let Some(state) = self.sessions.get_mut(&session) {
                    state.moqt.on_peer_request_closed(fetch.request);
                }
                actions.push(Action::ResetRequest {
                    session,
                    stream,
                    code: pipe_code,
                });
            }
            // The reset of the fetch stream says what happened; nothing more
            // is to be said on the request stream.
            (true, Pipe::Running | Pipe::Done) => {
                self.finish_fetch_request(session, stream, fetch.request, AnswerEnd::Cut, actions);
            }
        }
    }

    /// Ends the relay's direction of an answered FETCH's request stream.
    fn finish_fetch_request(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request: PeerRequestId,
        how: AnswerEnd,
        actions: &mut Vec<Action>,
    ) {
        self.finish_slot(session, stream, Some(request), actions);
        self.end_answered_fetch(session, stream, how, actions);
    }

    /// The FETCH is served when the subscriber has its `FETCH_OK` and the
    /// fetch stream was passed on to its FIN.
    fn finish_if_served(&mut self, subscriber: (SessionKey, StreamKey), actions: &mut Vec<Action>) {
        let served = self
            .fetches
            .relayed(&subscriber)
            .is_some_and(|fetch| fetch.answered && fetch.pipe == Pipe::Done);
        if !served {
            return;
        }
        let Some(fetch) = self.fetches.take_relayed(&subscriber) else {
            return;
        };
        if let Some(state) = self.sessions.get_mut(&fetch.publisher)
            && state.own_fetches.remove(&fetch.own).is_some()
        {
            // The session let go of the request when both had come.
            actions.push(Action::FinishOwnRequest {
                session: fetch.publisher,
                request: fetch.own,
            });
        }
        let served = AnswerEnd::Served;
        self.finish_fetch_request(subscriber.0, subscriber.1, fetch.request, served, actions);
    }

    /// A message on the request stream of the relay's FETCH.
    fn on_own_fetch_message(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        message: ControlMessage,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let Some(own_fetch) = state.own_fetches.get(&own) else {
            return;
        };
        let subscriber = own_fetch.subscriber;
        let event = match state.moqt.on_request_message(own_fetch.id, message) {
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
            RequestEvent::FetchEstablished {
                end_of_track,
                end_location,
                track_properties,
            } => {
                if end_of_track && !self.ends_the_track(subscriber, end_location) {
                    actions.push(Action::StopUpstreamTimer {
                        session,
                        request: own,
                    });
                    // §10.6: "MALFORMED_TRACK: In response to a FETCH, a
                    // relay publisher detected the track was malformed".
                    let malformed = Refusal::code(request_error_code::MALFORMED_TRACK);
                    let reset = stream_reset_code::MALFORMED_TRACK;
                    return self.fail_fetch(subscriber, &malformed, reset, actions);
                }
                // WHY the timer goes on where no fetch stream has opened yet:
                // the answer promises one, and a publisher that never opens
                // it would leave the subscriber waiting with its FETCH_OK for
                // as long as the two sessions live. From the answer, for as
                // long as the publisher may send nothing on its stream: the
                // subscriber's FILL_TIMEOUT, which the publisher was told,
                // would otherwise be cut short by a publisher that answers
                // first and opens its stream after. With the stream open or passed on, the timer
                // is not running.
                let waits_for_the_stream = self
                    .fetches
                    .relayed(&subscriber)
                    .is_some_and(|fetch| fetch.pipe == Pipe::NotOpen);
                // WHY not less than a second, where the FILL_TIMEOUT is: the
                // answer and the stream travel on streams of their own, and
                // a wait of milliseconds would decide by the order the two
                // happen to be read in. A fetch stream that is open has no
                // such floor: the time it may send nothing is what the
                // subscriber said it would wait for Objects.
                let wait_ms = self
                    .fetches
                    .relayed(&subscriber)
                    .map_or(FETCH_UPSTREAM_WAIT_MS, |fetch| fetch.upstream_wait_ms)
                    .max(MIN_FETCH_STREAM_WAIT_MS);
                actions.push(if waits_for_the_stream {
                    Action::StartFetchStreamTimer {
                        session,
                        request: own,
                        wait_ms,
                    }
                } else {
                    Action::StopUpstreamTimer {
                        session,
                        request: own,
                    }
                });
                self.answer_fetch(
                    subscriber,
                    end_of_track,
                    end_location,
                    track_properties,
                    actions,
                );
            }
            RequestEvent::Rejected(error) => {
                actions.push(Action::StopUpstreamTimer {
                    session,
                    request: own,
                });
                // The session has let go of the request.
                state_forgets(self, session, own);
                actions.push(Action::FinishOwnRequest {
                    session,
                    request: own,
                });
                // §10.12.3: "If the upstream FETCH fails, the relay sends a
                // REQUEST_ERROR and can reset the unidirectional stream."
                let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
                self.ask_the_next_or_fail(
                    subscriber,
                    &Refusal::passed_on(&error, OwnRequestKind::Fetch),
                    unknown,
                    actions,
                );
            }
            // §2.5.1: a Mandatory Track Property the relay does not
            // understand. The session ended the request; its stream is
            // cancelled.
            RequestEvent::UnsupportedFetch { .. } => {
                actions.push(Action::StopUpstreamTimer {
                    session,
                    request: own,
                });
                state_forgets(self, session, own);
                actions.push(Action::CancelOwnRequest {
                    session,
                    request: own,
                    code: stream_reset_code::CANCELLED,
                });
                let unsupported = Refusal::code(request_error_code::UNSUPPORTED_EXTENSION);
                let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
                self.fail_fetch(subscriber, &unsupported, unknown, actions);
            }
            // Nothing else answers a FETCH; the session has dealt with it. A
            // GOAWAY on its stream is as for the relay's SUBSCRIBE
            // (`routing.rs`): the FETCH goes on until the publisher ends it.
            _ => {}
        }
    }

    /// A publisher's `FETCH_OK` with End Of Track 1 for the FETCH of
    /// `subscriber` said the track ends before `end_location`: whether that
    /// agrees with what is known of the track (`Track::ends_at`). A track
    /// the relay does not have is not looked at.
    fn ends_the_track(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        end_location: Location,
    ) -> bool {
        let Some(fetch) = self.fetches.relayed(&subscriber) else {
            return true;
        };
        self.tracks
            .live_mut(&fetch.track.clone())
            .is_none_or(|track| track.ends_at(last_location(end_location)))
    }

    /// The publisher said `FETCH_OK`: the subscriber gets one with the same
    /// End Location and Track Properties (§9.6).
    fn answer_fetch(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        end_of_track: bool,
        end_location: Location,
        track_properties: Vec<KeyValuePair>,
        actions: &mut Vec<Action>,
    ) {
        let (session, stream) = subscriber;
        let Some(fetch) = self.fetches.relayed_mut(&subscriber) else {
            return;
        };
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let accepted =
            state
                .moqt
                .accept_fetch(fetch.request, end_of_track, end_location, track_properties);
        let Ok((ok, owed)) = accepted else {
            // Not reached as far as the relay knows. The session refuses to
            // make a `FETCH_OK` for three reasons, and the relay has seen to
            // each. The request is no FETCH that waits for its answer: the
            // relay answers a FETCH once, and lets go of it when the session
            // does. The End Location is before the Start Location of a
            // Standalone FETCH: the publisher's session took this End
            // Location for a FETCH of the same range; a Joining FETCH names
            // no Start Location to compare with. The SUBSCRIBE of a Joining
            // FETCH is not accepted: such a FETCH is held until it is, and
            // let go of when the subscription ends (module `requests`).
            //
            // WHY this FETCH alone is refused, with `INTERNAL_ERROR`: if the
            // relay's picture and the session's do part, the subscriber did
            // nothing wrong, and its other requests are not in doubt. The
            // session stays where it still has the FETCH waiting for its
            // answer, and takes the refusal. Where it has not, the first of
            // the three, it does not make the refusal either, and is closed
            // (`Relay::respond`).
            let failed = Refusal::code(request_error_code::INTERNAL_ERROR);
            let reset = stream_reset_code::INTERNAL_ERROR;
            return self.fail_fetch(subscriber, &failed, reset, actions);
        };
        fetch.answered = true;
        actions.push(Action::SendRequest {
            session,
            stream,
            message: ok,
        });
        acknowledge_updates(session, stream, owed, actions);
        self.finish_if_served(subscriber, actions);
    }

    /// The request stream of the relay's FETCH ended, or never came about.
    fn on_own_fetch_closed(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        how: OwnRequestEnd,
        actions: &mut Vec<Action>,
    ) {
        if how == OwnRequestEnd::Stopped {
            return;
        }
        let Some(subscriber) = self.subscriber_of(session, own) else {
            return;
        };
        let answered = self
            .fetches
            .relayed(&subscriber)
            .is_some_and(|fetch| fetch.answered);
        // After its FETCH_OK a publisher may finish its direction; how the
        // FETCH went on shows on the fetch stream.
        if answered && how == OwnRequestEnd::Fin {
            return;
        }
        let gone = Refusal::code(request_error_code::DOES_NOT_EXIST);
        let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
        self.ask_the_next_or_fail(subscriber, &gone, unknown, actions);
    }

    /// The publisher did not answer the relay's FETCH in time, or answered
    /// and did not open its fetch stream in time.
    fn on_fetch_timeout(
        &mut self,
        session: SessionKey,
        own: OwnRequestKey,
        actions: &mut Vec<Action>,
    ) {
        let Some(subscriber) = self.subscriber_of(session, own) else {
            return;
        };
        // §10.6.2, TIMEOUT.
        let too_late = Refusal::code(request_error_code::TIMEOUT);
        let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
        self.fail_fetch(subscriber, &too_late, unknown, actions);
    }

    fn subscriber_of(
        &self,
        session: SessionKey,
        own: OwnRequestKey,
    ) -> Option<(SessionKey, StreamKey)> {
        let own_fetch = self.sessions.get(&session)?.own_fetches.get(&own)?;
        Some(own_fetch.subscriber)
    }

    /// The `FETCH_HEADER` of a stream of a publisher arrived.
    fn on_fetch_stream_opened(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        request_id: u64,
        actions: &mut Vec<Action>,
    ) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let stop = Action::StopDataStream {
            session,
            stream,
            code: stream_reset_code::CANCELLED,
        };
        let id = match state.moqt.on_fetch_stream(request_id) {
            Ok(FetchStreamOpened::Fetch(id)) => id,
            // Of a FETCH the relay cancelled: the publisher opened the stream
            // before it knew (§5.2).
            Ok(FetchStreamOpened::Ended) => {
                actions.push(stop);
                return;
            }
            Err(error) => {
                self.fail_session(session, &error, actions);
                return;
            }
        };
        let own = state
            .own_fetches
            .iter()
            .find(|(_, own_fetch)| own_fetch.id == id)
            .map(|(own, own_fetch)| (*own, own_fetch.subscriber));
        let Some((own, subscriber)) = own else {
            actions.push(stop);
            return;
        };
        let Some(fetch) = self.fetches.relayed_mut(&subscriber) else {
            actions.push(stop);
            return;
        };
        fetch.pipe = Pipe::Running;
        // WHY the wait for the answer ends here: a publisher may send Objects
        // before its FETCH_OK (§10.13), and holds the FETCH_OK back until it
        // knows the End Location. From here on what is watched is that the
        // stream goes on; the wait for the answer starts anew when the
        // stream has ended.
        actions.push(Action::StopUpstreamTimer {
            session,
            request: own,
        });
        actions.push(Action::PipeFetch {
            from: session,
            from_stream: stream,
            session: subscriber.0,
            stream: subscriber.1,
            request_id: fetch.request.value(),
            upstream_wait_ms: fetch.upstream_wait_ms,
            downstream_wait_ms: FETCH_DOWNSTREAM_WAIT_MS,
            send_order: fetch.send_order,
        });
    }

    /// The passing on of a fetch stream ended by itself.
    fn on_fetch_pipe_ended(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        how: FetchPipeEnd,
        actions: &mut Vec<Action>,
    ) {
        let subscriber = (session, stream);
        let Some(fetch) = self.fetches.relayed_mut(&subscriber) else {
            return;
        };
        if fetch.pipe != Pipe::Running {
            return;
        }
        // The stream is over for the publisher's session too.
        let (publisher, own, answered) = (fetch.publisher, fetch.own, fetch.answered);
        fetch.pipe = Pipe::Done;
        if let Some(state) = self.sessions.get_mut(&publisher)
            && let Some(own_fetch) = state.own_fetches.get(&own)
        {
            state.moqt.on_fetch_stream_closed(own_fetch.id);
        }
        // The I/O side has ended both streams. `Done` keeps `fail_fetch`
        // from asking for that again.
        let refusal = match how {
            FetchPipeEnd::Finished => {
                if !answered {
                    // The stream came whole and the FETCH_OK is still out.
                    actions.push(Action::StartUpstreamTimer {
                        session: publisher,
                        request: own,
                    });
                }
                self.finish_if_served(subscriber, actions);
                return;
            }
            // §10.12.3, on a publisher that cannot say what comes next.
            FetchPipeEnd::UpstreamAborted(_) | FetchPipeEnd::UpstreamLost => {
                request_error_code::INTERNAL_ERROR
            }
            FetchPipeEnd::UpstreamStalled => request_error_code::TIMEOUT,
            // The subscriber gave the FETCH up, or is gone: §5.2 has the
            // publisher of a FETCH reset the request stream for the first.
            FetchPipeEnd::DownstreamStopped(_)
            | FetchPipeEnd::DownstreamStalled
            | FetchPipeEnd::DownstreamLost => {
                let code = if how == FetchPipeEnd::DownstreamStalled {
                    stream_reset_code::TOO_FAR_BEHIND
                } else {
                    stream_reset_code::CANCELLED
                };
                self.drop_fetch(subscriber, code, actions);
                return;
            }
        };
        let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
        self.fail_fetch(subscriber, &Refusal::code(refusal), unknown, actions);
    }

    /// Lets go of a FETCH whose subscriber does not want it any more, or
    /// cannot take it: the relay's FETCH is cancelled, what is passed on is
    /// cut, and the subscriber's request stream is reset with `code` (§5.2:
    /// "It MUST reset the bidi request stream and unidirectional data stream
    /// associated with the FETCH").
    fn drop_fetch(
        &mut self,
        subscriber: (SessionKey, StreamKey),
        code: u32,
        actions: &mut Vec<Action>,
    ) {
        let (session, stream) = subscriber;
        let request = if let Some(held) = self.let_go_of_held(&subscriber, actions) {
            held.request
        } else if let Some(fetch) = self.fetches.take_relayed(&subscriber) {
            if fetch.pipe == Pipe::Running {
                actions.push(Action::AbortFetchPipe {
                    session,
                    stream,
                    code,
                });
            }
            self.cancel_upstream(&fetch, actions);
            fetch.request
        } else {
            return;
        };
        self.take_slot(session, stream, actions);
        if let Some(state) = self.sessions.get_mut(&session) {
            state.moqt.on_peer_request_closed(request);
            actions.push(Action::ResetRequest {
                session,
                stream,
                code,
            });
        }
    }

    /// A `REQUEST_UPDATE` came for a FETCH that has no answer yet.
    pub(crate) fn on_fetch_update_held(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        let subscriber = (session, stream);
        let too_many = Refusal::code(request_error_code::EXCESSIVE_LOAD);
        if let Some(held) = self.fetches.held_mut(&subscriber) {
            held.held_updates = held.held_updates.saturating_add(1);
            if held.held_updates > MAX_HELD_FETCH_UPDATES
                && let Some(held) = self.let_go_of_held(&subscriber, actions)
            {
                self.refuse(session, stream, held.request, &too_many, actions);
            }
            return;
        }
        let Some(fetch) = self.fetches.relayed_mut(&subscriber) else {
            return;
        };
        fetch.held_updates = fetch.held_updates.saturating_add(1);
        if fetch.held_updates > MAX_HELD_FETCH_UPDATES {
            let unknown = stream_reset_code::UNKNOWN_OBJECT_STATUS;
            self.fail_fetch(subscriber, &too_many, unknown, actions);
        }
    }

    /// The subscriber cancelled its FETCH: its request stream was reset or
    /// stopped (§5.2).
    pub(crate) fn on_fetch_cancelled(
        &mut self,
        session: SessionKey,
        stream: StreamKey,
        actions: &mut Vec<Action>,
    ) {
        self.drop_fetch((session, stream), stream_reset_code::CANCELLED, actions);
    }

    /// What a session that is gone leaves of relayed FETCH requests.
    pub(crate) fn drop_fetches_of(&mut self, session: SessionKey, actions: &mut Vec<Action>) {
        self.fetches.forget_idle_of(session);
        self.fetch_ends.retain(|(of, _)| *of != session);
        // By key, so that the same events lead to the same actions.
        let mut as_subscriber: Vec<(SessionKey, StreamKey)> = self
            .fetches
            .all_relayed()
            .map(|(subscriber, _)| subscriber)
            .filter(|(subscriber, _)| *subscriber == session)
            .copied()
            .collect();
        as_subscriber.sort_unstable_by_key(|(_, stream)| stream.0);
        for subscriber in as_subscriber {
            let Some(fetch) = self.fetches.take_relayed(&subscriber) else {
                continue;
            };
            if fetch.pipe == Pipe::Running {
                actions.push(Action::AbortFetchPipe {
                    session: subscriber.0,
                    stream: subscriber.1,
                    code: stream_reset_code::SESSION_CLOSED,
                });
            }
            self.cancel_upstream(&fetch, actions);
        }
        let mut as_publisher: Vec<(SessionKey, StreamKey)> = self
            .fetches
            .all_relayed()
            .filter(|(_, fetch)| fetch.publisher == session)
            .map(|(subscriber, _)| *subscriber)
            .collect();
        as_publisher.sort_unstable_by_key(|(subscriber, stream)| (subscriber.0, stream.0));
        // As for a SUBSCRIBE whose publisher left before it answered.
        let gone = Refusal::code(request_error_code::DOES_NOT_EXIST);
        for subscriber in as_publisher {
            self.ask_the_next_or_fail(
                subscriber,
                &gone,
                stream_reset_code::UNKNOWN_OBJECT_STATUS,
                actions,
            );
        }
    }

    /// FETCH requests of subscribers the relay has not let go of.
    pub(crate) fn fetch_count(&self) -> usize {
        self.fetches.len()
    }
}

/// Removes the relay's FETCH from the publisher's session state, after the
/// session itself let go of the request.
fn state_forgets(relay: &mut Relay, session: SessionKey, own: OwnRequestKey) {
    if let Some(state) = relay.sessions.get_mut(&session) {
        state.own_fetches.remove(&own);
    }
}

/// The `REQUEST_OK` messages owed to `REQUEST_UPDATE`s that came before the
/// FETCH was answered.
fn acknowledge_updates(
    session: SessionKey,
    stream: StreamKey,
    owed: u64,
    actions: &mut Vec<Action>,
) {
    for _ in 0..owed {
        actions.push(Action::SendRequest {
            session,
            stream,
            message: ControlMessage::RequestOk(RequestOk::default()),
        });
    }
}

/// What the reset of a publisher's fetch stream becomes for the subscriber
/// whose stream it is passed on to: the Stream Reset Error Code to reset
/// that one with (§3.3.3).
///
/// WHY not the publisher's code in every case: as for a `REQUEST_ERROR`
/// ([`Refusal::passed_on`]), a code speaks about the two endpoints of the
/// stream it ends. What says something about the Objects, or says nothing,
/// is passed on. What is about the publisher's session with the relay is
/// not the subscriber's business: that the publisher is going becomes
/// `UNKNOWN_OBJECT_STATUS`, which is what it means for the rest of the
/// range, and the others, with any code unknown here and a reset the
/// transport gave no code for, become `INTERNAL_ERROR`.
#[must_use]
pub fn fetch_reset_passed_on(code: Option<u32>) -> u32 {
    use stream_reset_code as reset;
    match code {
        Some(
            passed @ (reset::INTERNAL_ERROR
            | reset::CANCELLED
            | reset::DELIVERY_TIMEOUT
            | reset::UNKNOWN_OBJECT_STATUS
            | reset::EXCESSIVE_LOAD
            | reset::MALFORMED_TRACK),
        ) => passed,
        Some(reset::SESSION_CLOSED | reset::GOING_AWAY) => reset::UNKNOWN_OBJECT_STATUS,
        // TOO_FAR_BEHIND and EXPIRED_AUTH_TOKEN are about the relay as the
        // publisher's subscriber.
        _ => reset::INTERNAL_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::{Location, end_location};

    fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    /// §10.2.7: the `SUBSCRIBER_PRIORITY` of a FETCH is "relative to other
    /// subscriptions and fetch responses in the same session", so its
    /// stream stands where a subgroup stream of that priority and a
    /// Publisher Priority of 128 stands, below the request streams (0).
    #[test]
    fn a_fetch_stream_stands_by_the_subscriber_priority_of_its_fetch() {
        use xmoq_moqt::params::Parameter;
        // (the parameters of the FETCH; the number of its stream)
        let cases = [
            (vec![], -32_897),
            (vec![Parameter::SubscriberPriority(128)], -32_897),
            (vec![Parameter::SubscriberPriority(0)], -129),
            (vec![Parameter::SubscriberPriority(255)], -65_409),
            (vec![Parameter::FillTimeout(5)], -32_897),
        ];
        for (parameters, expected) in cases {
            assert_eq!(
                super::fetch_send_order(&parameters),
                expected,
                "{parameters:?}"
            );
        }
    }

    #[test]
    fn the_end_location_of_fetch_ok_ends_at_the_largest_object() {
        // (the End Location that was asked for; the Largest Object; the End
        //  Location of FETCH_OK)
        let cases = [
            (at(3, 2), at(3, 5), at(3, 2)),
            (at(3, 6), at(3, 5), at(3, 6)),
            // Beyond the largest: the Largest Object plus 1 (§10.13).
            (at(3, 7), at(3, 5), at(3, 6)),
            (at(3, 0), at(3, 5), at(3, 6)),
            (at(4, 1), at(3, 5), at(3, 6)),
            // The largest is the last Object ID there is: the whole Group,
            // which is 0.
            (at(4, 0), at(3, u64::MAX), at(3, 0)),
            (at(3, 0), at(3, u64::MAX), at(3, 0)),
        ];
        for (asked, largest, end) in cases {
            assert_eq!(end_location(asked, largest), end, "{asked:?} {largest:?}");
        }
    }
}
