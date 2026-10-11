//! Random event sequences through two versions of the relay core at once.
//! `Relay::handle` is patched by `compare.py` to feed every event to both, to
//! print a `CORE_MISMATCH` line where their actions differ, and to panic in
//! tests where they differ in more than order.
//!
//! The sequences are the same in every run (the seeds are fixed). What may
//! differ from run to run is the number of order-only mismatches, where a
//! core iterates over a `HashMap`, whose order is random per process.
//!
//! `XMOQ_FUZZ_WORLD` chooses what the sequences are made of:
//!
//! * `v1` (the default): 25 kinds of event; every data stream is of Subgroup
//!   0 of one of three Groups, so that streams of the same Subgroup are
//!   common. The world of the first records.
//! * `distinct`: three kinds more (a control message, a session that failed
//!   on the I/O side, a `SUBSCRIBE_OK` with a Mandatory Track Property), and
//!   no two data streams of a session are of the same Subgroup: each is of a
//!   new Group.
//! * `full`: the three kinds more, the three ways a Subgroup Header gives the
//!   Subgroup ID (§11.4.2), and streams of the same Subgroup.
//! * `split`: two halves that have nothing to do with each other. Sessions 1
//!   and 2 publish, announce and subscribe to one track, with the streams of
//!   `full`; sessions 3 and 4 do the same with another track in another
//!   namespace, with the streams of `distinct`. What a change to how
//!   Subgroups are taken does can only begin in the first half, and
//!   `compare.py` checks that the two cores do the same to sessions 3 and 4
//!   from the first event to the last.
//!
//! * `one-publisher`: the streams of `full`, and one session, the first, is
//!   the only one that sends PUBLISH, `PUBLISH_NAMESPACE` or data streams. No
//!   track can get a second publisher.
//! * `split-one`: `split`, with one publisher in the second half: session 3
//!   publishes, announces and sends, session 4 subscribes. In the first half
//!   both sessions do all of it for the one track, so that a track with two
//!   publishers can only come about there.
//! * `shared`: `split-one`, and session 4 also subscribes to the track of the
//!   first half. `compare.py` then looks at session 3 and at what session 4
//!   gets for the track of the second half.
//!
//! With `XMOQ_FUZZ_STEADY` set, session 3 never says `PUBLISH_DONE`: not on
//! a PUBLISH of its own, and not in answer to a SUBSCRIBE of the relay. In
//! `split-one` and `shared` that makes the second half one in which no
//! publisher drains, so that none can publish anew while it does.
//! `compare.py` calls the two `split-steady` and `shared-steady`.
//!
//! With `XMOQ_FUZZ_PRIORITIES` set, the subgroup streams have one of three
//! Publisher Priorities (0, 1, and none, which is the default of 128), by
//! the number of the stream and its Group; without it they all have 0. In
//! the worlds with two halves only the streams of the first half vary, so
//! that the second stays one in which no Subgroup comes with two
//! priorities. `compare.py` sets it for `--priorities`.
//!
//! With `XMOQ_FUZZ_NO_ANNOUNCE` set, nobody announces a namespace, so that
//! the relay has no publisher to ask. `compare.py` calls `full` with it
//! `no-announce`.
//!
//! With `XMOQ_FUZZ_FETCH` set, one event in three is about a FETCH:
//! `compare.py` calls `full`, `split-one` and `shared` with it `fetch`,
//! `split-fetch` and `shared-fetch`. A subscriber sends a Standalone FETCH
//! (half of them for the whole of Group 2, the last one the streams of
//! these worlds have; the rest for an old Group, for several, for a range
//! that ends beyond the last Group, and for one that begins there), a
//! Relative or an Absolute Joining FETCH (for a SUBSCRIBE it sent, for one it
//! is yet to send, for a Request ID that is no SUBSCRIBE), a REQUEST_UPDATE
//! on the stream of a FETCH, or ends such a stream. A publisher answers a
//! FETCH the relay sent it, opens the fetch stream for it, or lets the time
//! for either run out. The I/O side reports how the passing on of a fetch
//! stream ended, and that a stream the relay wrote is written. WHY the
//! events of the publisher and of the I/O side name what the relay did
//! before in the sequence, and are not drawn at random: a FETCH the relay
//! never sent has no answer, and almost every draw would be one. In the
//! worlds with two halves, only sessions 1 and 2 have any of this, for the
//! track of the first half.
//!
//! `FUZZ_FETCHED` says how much of it the sequences got to: the FETCH
//! requests the relay sent a publisher, the fetch streams it passed on and
//! wrote itself, the Joining FETCH requests it held for a SUBSCRIBE to come,
//! and the `FETCH_OK` it sent. A world in which these are near 0 has not
//! tried what it is for.
//!
//! `XMOQ_FUZZ_TOUCH` chooses which events count as the ones a change is
//! about: `subgroups` (the default; a stream that leaves the Subgroup ID to
//! its first Object, or a second stream for a Subgroup) or `publishers` (a
//! PUBLISH for a track another session published before, or after another
//! session announced a namespace), `republish` (a PUBLISH by a session that
//! sent PUBLISH for the track before, or that said `PUBLISH_DONE`) or `waiting-or-draining` (a
//! PUBLISH after another session announced a namespace, or after any
//! `PUBLISH_DONE`).
//!
//! Every world but `v1` starts with a track that is published and subscribed
//! to (two in `split`, one a half). WHY: from nothing, few sequences get as
//! far as an Object that reaches a subscriber, and the streams are what the
//! comparisons of these worlds are for; for the same reason three in four of
//! the events about data streams are about that track. `v1` starts from
//! nothing, as it did in the first records.
//!
//! The test also prints a digest of every list of actions the relay
//! returned, in order. Two runs of a core whose actions depend on nothing
//! but the events print the same digest.

use std::hash::{DefaultHasher, Hasher};

use xmoq_moqt::data::{ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Fetch, FetchOk, FetchRange, Publish, PublishDone, PublishNamespace, ReasonPhrase,
    RequestError, RequestUpdate, Setup, Subscribe, SubscribeOk,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::{GroupOrder, Parameter};
use xmoq_moqt::session::Role;

use crate::{
    Action, Event, FetchPipeEnd, OutStreamKey, OwnRequestEnd, OwnRequestKey, Relay, RequestEnd, SessionKey,
    StreamEnd, StreamKey,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }
}

fn track(n: u64) -> FullTrackName {
    let namespace = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
    FullTrackName::new(namespace, vec![b'a' + u8::try_from(n).unwrap()]).unwrap()
}

/// The namespace of one half of the world `split`.
fn namespace_of_half(half: u64) -> Vec<u8> {
    format!("xmoq-{half}").into_bytes()
}

/// The one track of a half of the world `split`.
fn track_of_half(half: u64) -> FullTrackName {
    let namespace = TrackNamespace::new(vec![namespace_of_half(half)]).unwrap();
    FullTrackName::new(namespace, b"a".to_vec()).unwrap()
}

fn error(code: u64) -> ControlMessage {
    ControlMessage::RequestError(RequestError {
        error_code: code,
        retry_interval: 0,
        error_reason: ReasonPhrase::default(),
        redirect: None,
    })
}

fn done() -> ControlMessage {
    ControlMessage::PublishDone(PublishDone {
        status_code: 0x2,
        stream_count: 1,
        error_reason: ReasonPhrase::default(),
    })
}

/// What the generator remembers of one of the four sessions, so that most of
/// what it sends is valid: Request IDs go up by two, streams are numbered.
struct Peer {
    next_request: u64,
    next_stream: u64,
    next_group: u64,
    open: bool,
    /// A Request ID that was left out for a Joining FETCH to name before
    /// its request comes: the next request has it.
    left_out: Option<u64>,
    /// The Request ID of the last SUBSCRIBE.
    subscribed: Option<u64>,
    /// The streams of the FETCH requests it sent.
    fetches: Vec<StreamKey>,
}

impl Peer {
    fn new() -> Self {
        Self { next_request: 0, next_stream: 0, next_group: 0, open: true, left_out: None, subscribed: None, fetches: Vec::new() }
    }
}

/// What the relay did in a sequence that an event about a FETCH can name.
#[derive(Default)]
struct Fetched {
    /// The FETCH requests the relay sent: the publisher's session, the
    /// request, and the Request ID the relay gave it.
    asked: Vec<(SessionKey, OwnRequestKey, u64)>,
    /// The fetch streams it passes on, and the ones it writes, by the
    /// subscriber's session and request stream.
    passed_on: Vec<(SessionKey, StreamKey)>,
    written: Vec<(SessionKey, StreamKey)>,
    /// The Joining FETCH requests it holds for a SUBSCRIBE to come.
    held: Vec<(SessionKey, StreamKey)>,
}

impl Fetched {
    fn note(&mut self, actions: &[Action], counts: &mut [u64; 5]) {
        for action in actions {
            match action {
                Action::OpenRequest { session, request, message: ControlMessage::Fetch(fetch) } => {
                    self.asked.push((*session, *request, fetch.request_id));
                    counts[0] += 1;
                }
                Action::PipeFetch { session, stream, .. } => {
                    self.passed_on.push((*session, *stream));
                    counts[1] += 1;
                }
                Action::WriteFetchStream { session, stream, .. } => {
                    self.written.push((*session, *stream));
                    counts[2] += 1;
                }
                Action::StartJoiningFetchTimer { session, stream } => {
                    self.held.push((*session, *stream));
                    counts[3] += 1;
                }
                Action::SendRequest { message: ControlMessage::FetchOk(_), .. } => counts[4] += 1,
                _ => {}
            }
        }
    }
}

fn one_of<T: Copy>(rng: &mut Rng, known: &[T]) -> Option<T> {
    let index = usize::try_from(rng.next(u64::try_from(known.len()).unwrap().max(1))).unwrap();
    // The later ones are the ones that are still open, more often than not.
    known.get(index).copied().filter(|_| rng.next(3) == 0).or_else(|| known.last().copied())
}

fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

/// An event about a FETCH; see the head of this file. `None` where the relay
/// has done nothing yet that the event drawn could name.
fn fetch_event(rng: &mut Rng, peers: &mut [Peer], world: World, fetched: &Fetched) -> Option<Event> {
    // In the worlds with two halves, the first half alone.
    let who = if world.is_split() { rng.next(2) } else { rng.next(4) };
    let session = SessionKey(who + 1);
    let peer = &mut peers[usize::try_from(who).unwrap()];
    let name = if world.is_split() { track_of_half(0) } else { track(rng.next(2)) };
    let fetch = |peer: &mut Peer, request_id: u64, range: FetchRange, parameters: Vec<Parameter>| {
        let stream = StreamKey(peer.next_stream);
        peer.next_stream += 1;
        peer.fetches.push(stream);
        Event::RequestOpened {
            session,
            stream,
            first: ControlMessage::Fetch(Fetch { request_id, range, parameters }),
        }
    };
    Some(match rng.next(18) {
        0..=3 => {
            let (start, end) = match rng.next(8) {
                // The whole of the last Group.
                0..=3 => (at(2, 0), at(2, 0)),
                4 => (at(0, 0), at(0, 0)),
                5 => (at(0, 0), at(2, 0)),
                6 => (at(2, 0), at(9, 0)),
                _ => (at(5, 0), at(9, 0)),
            };
            let parameters = match rng.next(6) {
                // Answered by the relay itself.
                0..=1 => vec![Parameter::FillTimeout(0)],
                2 => vec![Parameter::GroupOrder(GroupOrder::Descending)],
                _ => vec![],
            };
            let (request_id, _) = new_request_id(peer);
            fetch(peer, request_id, FetchRange::Standalone { full_track_name: name, start, end }, parameters)
        }
        4..=5 => {
            let (request_id, joining_request_id) = match (rng.next(4), peer.subscribed) {
                // A SUBSCRIBE that is yet to come: its Request ID is left
                // out, and the next request of the session has it.
                (0, _) if peer.left_out.is_none() => {
                    let left_out = peer.next_request;
                    peer.next_request += 4;
                    peer.left_out = Some(left_out);
                    (left_out + 2, left_out)
                }
                (1, _) | (_, None) => (new_request_id(peer).0, rng.next(8)),
                (_, Some(subscribed)) => (new_request_id(peer).0, subscribed),
            };
            let range = if rng.next(2) == 0 {
                FetchRange::RelativeJoining { joining_request_id, joining_start: rng.next(3) }
            } else {
                FetchRange::AbsoluteJoining { joining_request_id, joining_start: rng.next(4) }
            };
            let parameters = if rng.next(6) == 0 { vec![Parameter::FillTimeout(0)] } else { vec![] };
            fetch(peer, request_id, range, parameters)
        }
        6 => {
            let stream = one_of(rng, &peer.fetches)?;
            let (request_id, _) = new_request_id(peer);
            let parameters = match rng.next(3) {
                0 => vec![],
                1 => vec![Parameter::SubscriberPriority(200)],
                // Not one an update of a FETCH may carry (§10.2.12).
                _ => vec![Parameter::Forward(true)],
            };
            Event::RequestMessage {
                session,
                stream,
                message: ControlMessage::RequestUpdate(RequestUpdate { request_id, parameters }),
            }
        }
        7 => Event::RequestClosed {
            session,
            stream: one_of(rng, &peer.fetches)?,
            how: if rng.next(2) == 0 { RequestEnd::Fin } else { RequestEnd::Aborted },
        },
        8..=10 => {
            let (session, request, _) = one_of(rng, &fetched.asked)?;
            let message = match rng.next(6) {
                0 => error(0x11),
                1 => error(0x10),
                2 => ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: at(2, 4),
                    parameters: vec![],
                    track_properties: vec![],
                }),
                _ => ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: at(2, 0),
                    parameters: vec![],
                    track_properties: vec![],
                }),
            };
            Event::OwnRequestMessage { session, request, message }
        }
        11..=12 => {
            let (session, _, request_id) = one_of(rng, &fetched.asked)?;
            Event::FetchStreamOpened { session, stream: StreamKey(300 + rng.next(8)), request_id }
        }
        13..=14 => {
            let (session, stream) = one_of(rng, &fetched.passed_on)?;
            let how = match rng.next(10) {
                0..=4 => FetchPipeEnd::Finished,
                5 => FetchPipeEnd::UpstreamAborted(Some(0x2)),
                6 => FetchPipeEnd::UpstreamLost,
                7 => FetchPipeEnd::UpstreamStalled,
                8 => FetchPipeEnd::DownstreamStopped(Some(0x1)),
                _ => if rng.next(2) == 0 { FetchPipeEnd::DownstreamStalled } else { FetchPipeEnd::DownstreamLost },
            };
            Event::FetchPipeEnded { session, stream, how }
        }
        15 => {
            let (session, stream) = one_of(rng, &fetched.written)?;
            // TOO_FAR_BEHIND (0x5), for a subscriber that did not take it.
            let given_up = if rng.next(4) == 0 { Some(0x5) } else { None };
            Event::FetchStreamWritten { session, stream, given_up }
        }
        16 => {
            let (session, stream) = one_of(rng, &fetched.held)?;
            Event::JoiningFetchTimeout { session, stream }
        }
        _ => {
            let (session, request, _) = one_of(rng, &fetched.asked)?;
            Event::UpstreamTimeout { session, request }
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum World {
    V1,
    Distinct,
    Full,
    Split,
    OnePublisher,
    SplitOne,
    Shared,
}

impl World {
    fn from_env() -> Self {
        match std::env::var("XMOQ_FUZZ_WORLD").as_deref() {
            Err(_) | Ok("v1") => Self::V1,
            Ok("distinct") => Self::Distinct,
            Ok("full") => Self::Full,
            Ok("split") => Self::Split,
            Ok("one-publisher") => Self::OnePublisher,
            Ok("split-one") => Self::SplitOne,
            Ok("shared") => Self::Shared,
            Ok(other) => panic!("XMOQ_FUZZ_WORLD={other}: not a world of fuzz.rs"),
        }
    }
}

/// The Group and the Subgroup ID of a new data stream, as the world has them.
fn subgroup_of_a_new_stream(rng: &mut Rng, peer: &mut Peer, world: World) -> (u64, SubgroupId) {
    match world {
        World::V1 => (rng.next(3), SubgroupId::Zero),
        World::Distinct => {
            peer.next_group += 1;
            (peer.next_group - 1, SubgroupId::Zero)
        }
        World::Split | World::OnePublisher | World::SplitOne | World::Shared => {
            unreachable!("it has the streams of full or of distinct")
        }
        World::Full => {
            let group = rng.next(3);
            let subgroup = match rng.next(3) {
                0 => SubgroupId::Zero,
                1 => SubgroupId::Explicit(rng.next(2)),
                _ => SubgroupId::FirstObjectId,
            };
            (group, subgroup)
        }
    }
}

/// Whose data stream an event is about, and under which Track Alias. In every
/// world but `v1`, three in four are about the track the sequence started
/// with (of the same half, in `split`), so that Objects reach a subscriber.
fn data_source(rng: &mut Rng, world: World, who: u64, alias: u64) -> (u64, u64) {
    if world == World::V1 || rng.next(4) == 0 {
        return (publisher_for(world, who), alias);
    }
    (if world.is_split() { who / 2 * 2 } else { 0 }, 40)
}

/// The session that publishes where `who` would: itself, unless the world
/// has one publisher there.
fn publisher_for(world: World, who: u64) -> u64 {
    match world {
        World::OnePublisher => 0,
        World::SplitOne | World::Shared if who / 2 == 1 => 2,
        _ => who,
    }
}

impl World {
    fn is_split(self) -> bool {
        matches!(self, Self::Split | Self::SplitOne | Self::Shared)
    }
}

fn new_request(peer: &mut Peer) -> (u64, StreamKey) {
    let (request_id, _) = new_request_id(peer);
    let opened = (request_id, StreamKey(peer.next_stream));
    peer.next_stream += 1;
    opened
}

/// The next Request ID of a session: one that was left out for a Joining
/// FETCH, where there is one. The second value says that it was.
fn new_request_id(peer: &mut Peer) -> (u64, bool) {
    if let Some(left_out) = peer.left_out.take() {
        return (left_out, true);
    }
    peer.next_request += 2;
    (peer.next_request - 2, false)
}

/// One of 25 kinds of event, about one of 4 sessions, 2 tracks, 2 Track
/// Aliases, 4 data streams a session, 6 requests of the relay and 12 streams
/// to subscribers.
fn random_event(rng: &mut Rng, peers: &mut [Peer], world: World) -> Event {
    let who = rng.next(4);
    let session = SessionKey(who + 1);
    let peer = &mut peers[usize::try_from(who).unwrap()];
    let stream = StreamKey(rng.next(peer.next_stream.max(1) + 1));
    let data_stream = StreamKey(100 + rng.next(4));
    let request = OwnRequestKey(rng.next(6));
    let name = rng.next(2);
    let alias = 40 + rng.next(2);
    let half = who / 2;
    let published = |name: u64| if world.is_split() { track_of_half(half) } else { track(name) };
    // Session 4 of `shared` subscribes to the tracks of both halves.
    let subscribed = |name: u64| if world == World::Shared && who == 3 { track_of_half(name) } else { published(name) };
    let publishes = publisher_for(world, who);
    let streams = match world {
        split if split.is_split() && half == 0 => World::Full,
        split if split.is_split() => World::Distinct,
        World::OnePublisher => World::Full,
        other => other,
    };
    let kinds = if world == World::V1 { 30 } else { 33 };
    match rng.next(kinds) {
        30 => Event::ControlMessage {
            session,
            message: match rng.next(3) {
                0 => ControlMessage::Setup(Setup { setup_options: vec![] }),
                1 => done(),
                _ => error(0x10),
            },
        },
        31 => Event::SessionFailed {
            session,
            error: xmoq_moqt::Error::ProtocolViolation("made up by the random events"),
        },
        // 0x4000 is a Mandatory Track Property this relay does not know.
        32 => Event::OwnRequestMessage {
            session,
            request,
            message: ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: alias,
                parameters: vec![],
                track_properties: vec![KeyValuePair::varint(0x4000, 1).unwrap()],
            }),
        },
        0..=2 => {
            let (id, new) = new_request(peer);
            peer.subscribed = Some(id);
            let forward_0 = rng.next(4) == 0;
            Event::RequestOpened {
                session,
                stream: new,
                first: ControlMessage::Subscribe(Subscribe {
                    request_id: id,
                    full_track_name: subscribed(name),
                    parameters: if forward_0 { vec![Parameter::Forward(false)] } else { vec![] },
                }),
            }
        }
        3..=4 => {
            let session = SessionKey(publishes + 1);
            let (id, new) = new_request(&mut peers[usize::try_from(publishes).unwrap()]);
            Event::RequestOpened {
                session,
                stream: new,
                first: ControlMessage::Publish(Publish {
                    request_id: id,
                    full_track_name: published(name),
                    track_alias: alias,
                    parameters: vec![],
                    track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
                }),
            }
        }
        5..=6 => {
            let session = SessionKey(publishes + 1);
            let (id, new) = new_request(&mut peers[usize::try_from(publishes).unwrap()]);
            let fields = if rng.next(3) == 0 { vec![] } else { vec![b"xmoq".to_vec()] };
            // The namespace without fields matches every track.
            let fields = if world.is_split() { vec![namespace_of_half(half)] } else { fields };
            Event::RequestOpened {
                session,
                stream: new,
                first: ControlMessage::PublishNamespace(PublishNamespace {
                    request_id: id,
                    track_namespace: TrackNamespace::new(fields).unwrap(),
                    parameters: vec![],
                }),
            }
        }
        // WHY not from session 4 of `shared`: a message that is an error on a
        // request closes the session in the core where the request is still
        // open, and not in the one where it is over. On a stream of the first
        // half that is the change itself, and it takes the session's other
        // subscription with it.
        7 | 8 if world == World::Shared && who == 3 => Event::RequestClosed {
            session,
            stream,
            how: RequestEnd::Fin,
        },
        7 => Event::RequestMessage { session, stream, message: done() },
        8 => {
            peer.next_request += 2;
            Event::RequestMessage {
                session,
                stream,
                message: ControlMessage::RequestUpdate(RequestUpdate {
                    request_id: peer.next_request - 2,
                    parameters: vec![],
                }),
            }
        }
        9..=10 => Event::RequestClosed {
            session,
            stream,
            how: if rng.next(2) == 0 { RequestEnd::Fin } else { RequestEnd::Aborted },
        },
        11..=13 => Event::OwnRequestMessage {
            session,
            request,
            message: match rng.next(6) {
                0..=2 => ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: alias,
                    parameters: vec![],
                    track_properties: vec![],
                }),
                3 => error(0x19),
                4 => error(0x10),
                _ => done(),
            },
        },
        14 => Event::OwnRequestClosed {
            session,
            request,
            how: match rng.next(4) {
                0 => OwnRequestEnd::Unsent,
                1 => OwnRequestEnd::Fin,
                2 => OwnRequestEnd::Reset,
                _ => OwnRequestEnd::Stopped,
            },
        },
        15 => Event::UpstreamTimeout { session, request },
        16..=18 => {
            let (who, alias) = data_source(rng, world, who, alias);
            let session = SessionKey(who + 1);
            let peer = &mut peers[usize::try_from(who).unwrap()];
            let (group_id, subgroup_id) = subgroup_of_a_new_stream(rng, peer, streams);
            Event::DataStreamOpened {
            session,
            stream: data_stream,
            header: SubgroupHeader {
                track_alias: alias,
                group_id,
                subgroup_id,
                publisher_priority: Some(0),
                properties: false,
                end_of_group: false,
                first_object: true,
            },
            }
        }
        19..=20 => Event::Object {
            session: SessionKey(data_source(rng, world, who, alias).0 + 1),
            stream: data_stream,
            object: SubgroupObject {
                object_id: rng.next(4),
                properties: vec![],
                status: ObjectStatus::Normal,
                payload: vec![7; 3],
            },
        },
        21..=22 => Event::DataStreamClosed {
            session: SessionKey(data_source(rng, world, who, alias).0 + 1),
            stream: data_stream,
            end: match rng.next(4) {
                0..=1 => StreamEnd::Fin,
                2 => StreamEnd::Aborted(Some(0x2)),
                _ => StreamEnd::Lost,
            },
        },
        23 => Event::TrackMalformed { session, stream: data_stream },
        24 => Event::OutStreamOpened { out_stream: OutStreamKey(rng.next(12)) },
        25 => Event::OutStreamClosed { out_stream: OutStreamKey(rng.next(12)) },
        26 => match rng.next(4) {
            0 => Event::SendQueueFull { out_stream: OutStreamKey(rng.next(12)) },
            // The number of the wait is put in by the caller, which has seen
            // the timers the relay started.
            1 => Event::DrainTimeout { session, track_alias: alias, wait: 0 },
            2 => Event::EarlyStreamTimeout { session, stream: data_stream },
            _ => Event::RequestTimeout { session, stream },
        },
        27 => {
            peer.open = false;
            Event::SessionClosed { session }
        }
        // The I/O side never opens a session under a key that is in use.
        28 if !peer.open => {
            *peer = Peer::new();
            Event::SessionOpened { session, role: Role::Server }
        }
        _ => Event::DataStreamClosed {
            session,
            stream: data_stream,
            end: StreamEnd::Aborted(None),
        },
    }
}

/// Whether an event is one of those the change to how Subgroups are taken
/// is about: a data stream whose header leaves the Subgroup ID to its first
/// Object, or a data stream for a Group and Subgroup ID that a stream of the
/// same session and Track Alias was opened for before in the sequence.
fn touches_subgroup_selection(event: &Event, opened: &mut Vec<(u64, u64, u64, u64)>) -> bool {
    let Event::DataStreamOpened { session, header, .. } = event else {
        return false;
    };
    // A header without a Subgroup ID and one that names 0 are of the same
    // Subgroup.
    let subgroup = match header.subgroup_id {
        SubgroupId::Zero => 0,
        SubgroupId::Explicit(id) => id,
        SubgroupId::FirstObjectId => return true,
    };
    let key = (session.0, header.track_alias, header.group_id, subgroup);
    let again = opened.contains(&key);
    opened.push(key);
    again
}

/// Whether an event is a PUBLISH for a track that another session published
/// before in the sequence, or that comes after another session announced a
/// namespace. `published` has a session and its track, or no track for a
/// namespace, which is taken to match every track.
fn touches_the_publishers(event: &Event, published: &mut Vec<(u64, Option<FullTrackName>)>) -> bool {
    let Event::RequestOpened { session, first, .. } = event else {
        return false;
    };
    match first {
        ControlMessage::Publish(publish) => {
            let name = &publish.full_track_name;
            let again = published
                .iter()
                .any(|(other, track)| *other != session.0 && track.as_ref().is_none_or(|track| track == name));
            published.push((session.0, Some(name.clone())));
            again
        }
        ControlMessage::PublishNamespace(_) => {
            published.push((session.0, None));
            false
        }
        _ => false,
    }
}

/// Whether an event is a PUBLISH by a session that sent PUBLISH for the track
/// before in the sequence, or that said `PUBLISH_DONE` for anything: on its
/// PUBLISH, or in answer to a SUBSCRIBE of the relay. A `SUBSCRIBE_OK` of
/// a session that said `PUBLISH_DONE` counts as well: it can use the Track
/// Alias of the subscription that drains. So does a subgroup stream of
/// such a session: under an alias that drains it is stopped where the
/// session is no source of the track, which is so when the track ended
/// and another publisher made one of the name. `seen` has a session and its
/// track, or no track for a `PUBLISH_DONE`.
fn publishes_again(event: &Event, seen: &mut Vec<(u64, Option<FullTrackName>)>) -> bool {
    match event {
        Event::RequestOpened { session, first: ControlMessage::Publish(publish), .. } => {
            let this = (session.0, Some(publish.full_track_name.clone()));
            let again = seen.contains(&this) || seen.contains(&(session.0, None));
            seen.push(this);
            again
        }
        Event::RequestMessage { session, message: ControlMessage::PublishDone(_), .. }
        | Event::OwnRequestMessage { session, message: ControlMessage::PublishDone(_), .. } => {
            seen.push((session.0, None));
            false
        }
        Event::OwnRequestMessage { session, message: ControlMessage::SubscribeOk(_), .. }
        | Event::DataStreamOpened { session, .. } => seen.contains(&(session.0, None)),
        _ => false,
    }
}

/// The event, unless it is a `PUBLISH_DONE` of session 3: then one that
/// leaves its subscriptions as they are. For the worlds in which the
/// publisher of the second half is never to drain (`XMOQ_FUZZ_STEADY`).
fn without_a_publish_done_of_session_3(event: Event) -> Event {
    match event {
        Event::RequestMessage { session: SessionKey(3), stream, message: ControlMessage::PublishDone(_) } => {
            Event::RequestTimeout { session: SessionKey(3), stream }
        }
        Event::OwnRequestMessage { session: SessionKey(3), request, message: ControlMessage::PublishDone(_) } => {
            Event::UpstreamTimeout { session: SessionKey(3), request }
        }
        other => other,
    }
}

/// The event, with one of three Publisher Priorities if it opens a subgroup
/// stream (`XMOQ_FUZZ_PRIORITIES`). WHY by the numbers of the stream and
/// the Group and not at random: the sequences stay the ones they are
/// without it.
fn with_a_priority_of_three(event: Event, world: World) -> Event {
    match event {
        Event::DataStreamOpened { session, stream, header } if !world.is_split() || session.0 <= 2 => {
            let publisher_priority = match (stream.0 + header.group_id) % 3 {
                0 => Some(0),
                1 => Some(1),
                _ => None,
            };
            Event::DataStreamOpened { session, stream, header: SubgroupHeader { publisher_priority, ..header } }
        }
        other => other,
    }
}

/// Whether an event opens a subgroup stream of a Group that a stream was
/// opened for before with another Publisher Priority. Wider than what makes
/// a track malformed, which takes the same track and the same Subgroup: the
/// track and the Subgroup are not looked at. `seen` has the Group and the
/// priority of every stream so far.
fn opens_a_group_with_another_priority(event: &Event, seen: &mut Vec<(u64, Option<u8>)>) -> bool {
    let Event::DataStreamOpened { header, .. } = event else {
        return false;
    };
    let opened = (header.group_id, header.publisher_priority);
    let touches = seen.iter().any(|(group, priority)| *group == opened.0 && *priority != opened.1);
    seen.push(opened);
    touches
}

/// The event, unless it announces a namespace: then one that does nothing
/// (`XMOQ_FUZZ_NO_ANNOUNCE`).
fn without_an_announcement(event: Event) -> Event {
    match event {
        Event::RequestOpened { session, stream, first: ControlMessage::PublishNamespace(_) } => {
            Event::RequestTimeout { session, stream }
        }
        other => other,
    }
}

/// Whether an event comes when the relay can have more than one publisher
/// to ask for a track, or one to ask for a track that has a source: a
/// session has announced a namespace, and another has announced one or sent
/// PUBLISH. With one session that announces, and no other that publishes,
/// the relay asks whom it asked before. `seen` has a session and no track
/// for a namespace it announced, and a session and the track of its PUBLISH.
fn several_can_be_asked(event: &Event, seen: &mut Vec<(u64, Option<FullTrackName>)>) -> bool {
    match event {
        Event::RequestOpened { session, first: ControlMessage::PublishNamespace(_), .. } => {
            seen.push((session.0, None));
        }
        Event::RequestOpened { session, first: ControlMessage::Publish(publish), .. } => {
            seen.push((session.0, Some(publish.full_track_name.clone())));
        }
        _ => {}
    }
    seen.iter()
        .filter(|(_, track)| track.is_none())
        .any(|(announcer, _)| seen.iter().any(|(other, _)| other != announcer))
}

/// Whether an event is a PUBLISH that can meet a track that waits for a
/// publisher's answer or whose sources drain: one that comes after another
/// session announced a namespace, or after a `PUBLISH_DONE` from anyone.
/// Wider than it has to be, which only makes the check stricter. `seen` has
/// a session and no track for a namespace it announced, and session 0, which
/// is no session, for a `PUBLISH_DONE`.
fn publishes_to_a_waiting_or_draining_track(
    event: &Event,
    seen: &mut Vec<(u64, Option<FullTrackName>)>,
) -> bool {
    match event {
        Event::RequestOpened { session, first: ControlMessage::Publish(_), .. } => {
            seen.iter().any(|(other, track)| track.is_none() && *other != session.0)
        }
        Event::RequestOpened { session, first: ControlMessage::PublishNamespace(_), .. } => {
            seen.push((session.0, None));
            false
        }
        Event::RequestMessage { message: ControlMessage::PublishDone(_), .. }
        | Event::OwnRequestMessage { message: ControlMessage::PublishDone(_), .. } => {
            seen.push((0, None));
            false
        }
        _ => false,
    }
}

/// Session `subscriber` subscribes to the track. Returns the request stream.
fn subscribe_to(relay: &mut Relay, peers: &mut [Peer], subscriber: usize, name: &FullTrackName) -> StreamKey {
    let (request_id, stream) = new_request(&mut peers[subscriber - 1]);
    relay.handle(Event::RequestOpened {
        session: SessionKey(subscriber as u64),
        stream,
        first: ControlMessage::Subscribe(Subscribe {
            request_id,
            full_track_name: name.clone(),
            parameters: vec![],
        }),
    });
    stream
}

/// Session `publisher` publishes the track under Track Alias 40, and session
/// `subscriber` subscribes to it.
fn start_a_track(relay: &mut Relay, peers: &mut [Peer], publisher: usize, subscriber: usize, name: &FullTrackName) {
    let (request_id, stream) = new_request(&mut peers[publisher - 1]);
    relay.handle(Event::RequestOpened {
        session: SessionKey(publisher as u64),
        stream,
        first: ControlMessage::Publish(Publish {
            request_id,
            full_track_name: name.clone(),
            track_alias: 40,
            // WHY a Largest Object where the events are about FETCH: the
            // relay answers a FETCH with FILL_TIMEOUT 0 itself, and works
            // out the range of a Joining FETCH, only for a track it knows
            // one of, and few sequences get an Object through first.
            parameters: if std::env::var_os("XMOQ_FUZZ_FETCH").is_some() {
                vec![Parameter::LargestObject(at(2, 3))]
            } else {
                vec![]
            },
            track_properties: vec![],
        }),
    });
    subscribe_to(relay, peers, subscriber, name);
}

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

#[test]
fn the_two_cores_answer_random_events_alike() {
    let (runs, length) = (setting("XMOQ_FUZZ_RUNS", 20_000), setting("XMOQ_FUZZ_LEN", 80));
    let world = World::from_env();
    let touch = std::env::var("XMOQ_FUZZ_TOUCH").unwrap_or_else(|_| "subgroups".to_string());
    let steady = std::env::var_os("XMOQ_FUZZ_STEADY").is_some();
    let no_announce = std::env::var_os("XMOQ_FUZZ_NO_ANNOUNCE").is_some();
    let priorities = std::env::var_os("XMOQ_FUZZ_PRIORITIES").is_some();
    let fetches = std::env::var_os("XMOQ_FUZZ_FETCH").is_some();
    // Requests the relay sent a publisher, streams it passed on, streams it
    // wrote, Joining FETCH requests it held, FETCH_OK it sent.
    let mut fetch_counts = [0u64; 5];
    let mut with_a_fetch_passed_on = 0u64;
    let mut events = 0u64;
    let (mut touching, mut differing, mut differing_before_touching) = (0u64, 0u64, 0u64);
    let (mut opened_downstream, mut objects_sent, mut with_an_object) = (0u64, 0u64, 0u64);
    let differences = || crate::DIFFERENCES.load(std::sync::atomic::Ordering::Relaxed);
    // The first difference of each sequence that differs, by its kind.
    let mut first_differences: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    // `DefaultHasher::new()` has fixed keys, unlike the hasher of a `HashMap`.
    let mut digest = DefaultHasher::new();
    for seed in 1..=runs {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut relay = Relay::new("fuzz/0");
        let mut peers: Vec<Peer> = (0..4).map(|_| Peer::new()).collect();
        let mut fetched = Fetched::default();
        for session in 1..=4 {
            relay.handle(Event::SessionOpened { session: SessionKey(session), role: Role::Server });
        }
        // What session 4 of `shared` asked for the track of the second half:
        // the part of that session `compare.py` holds the two cores to.
        let mut watched: Vec<(u64, u64)> = Vec::new();
        let watch = |watched: &[(u64, u64)]| {
            *crate::BYSTANDER_STREAMS.lock().unwrap() = watched.to_vec();
        };
        watch(&watched);
        let mut published = Vec::new();
        let mut priorities_of_groups = Vec::new();
        match world {
            World::V1 => {}
            World::Distinct | World::Full | World::OnePublisher => {
                start_a_track(&mut relay, &mut peers, 1, 2, &track(0));
                published.push((1, Some(track(0))));
            }
            World::Split | World::SplitOne | World::Shared => {
                start_a_track(&mut relay, &mut peers, 1, 2, &track_of_half(0));
                if world == World::Shared {
                    // The stream the next request of session 4 is opened on.
                    watched.push((4, peers[3].next_stream));
                    watch(&watched);
                }
                start_a_track(&mut relay, &mut peers, 3, 4, &track_of_half(1));
                if world == World::Shared {
                    subscribe_to(&mut relay, &mut peers, 4, &track_of_half(0));
                }
                published.push((1, Some(track_of_half(0))));
                published.push((3, Some(track_of_half(1))));
            }
        }
        let (mut opened, mut touched, mut differed) = (Vec::new(), false, false);
        let objects_before = objects_sent;
        let mut waits: Vec<((SessionKey, u64), u64)> = Vec::new();
        let passed_on_before = fetch_counts[1];
        for index in 0..length {
            let about_a_fetch = if fetches && rng.next(3) == 0 {
                fetch_event(&mut rng, &mut peers, world, &fetched)
            } else {
                None
            };
            let is_about_a_fetch = about_a_fetch.is_some();
            let event = about_a_fetch.unwrap_or_else(|| random_event(&mut rng, &mut peers, world));
            // A timer that runs out is one the relay started: the last one
            // for the alias, or, every fourth event, the first, which a
            // later wait under the alias must not be ended by.
            let event = match event {
                Event::DrainTimeout { session, track_alias, .. } => {
                    let started = waits.iter().filter(|(of, _)| *of == (session, track_alias)).map(|(_, wait)| *wait);
                    let wait = if index % 4 == 0 { started.min() } else { started.max() };
                    Event::DrainTimeout { session, track_alias, wait: wait.unwrap_or(0) }
                }
                other => other,
            };
            let event = if steady { without_a_publish_done_of_session_3(event) } else { event };
            let event = if no_announce { without_an_announcement(event) } else { event };
            let event = if priorities { with_a_priority_of_three(event, world) } else { event };
            let touches = match touch.as_str() {
                "subgroups" => touches_subgroup_selection(&event, &mut opened),
                "publishers" => touches_the_publishers(&event, &mut published),
                "republish" => publishes_again(&event, &mut published),
                "waiting-or-draining" => publishes_to_a_waiting_or_draining_track(&event, &mut published),
                "asks" => several_can_be_asked(&event, &mut published),
                "priority" => opens_a_group_with_another_priority(&event, &mut priorities_of_groups),
                "fetch" => is_about_a_fetch,
                other => panic!("XMOQ_FUZZ_TOUCH={other}: not a kind of event fuzz.rs knows"),
            };
            if world == World::Shared {
                match &event {
                    Event::RequestOpened {
                        session: SessionKey(4),
                        stream,
                        first: ControlMessage::Subscribe(subscribe),
                    } if subscribe.full_track_name == track_of_half(1) => watched.push((4, stream.0)),
                    Event::SessionOpened { session: SessionKey(4), .. }
                    | Event::SessionClosed { session: SessionKey(4) }
                    | Event::SessionFailed { session: SessionKey(4), .. } => watched.clear(),
                    _ => {}
                }
                watch(&watched);
            }
            let before = differences();
            let actions = relay.handle(event);
            fetched.note(&actions, &mut fetch_counts);
            for action in &actions {
                if let Action::StartDrainTimer { session, track_alias, wait } = action {
                    waits.push(((*session, *track_alias), *wait));
                }
            }
            if differences() != before && !differed {
                differed = true;
                differing += 1;
                *first_differences.entry(crate::LAST_DIFFERENCE.lock().unwrap().clone()).or_default() += 1;
                // The event itself may be the one that touches.
                if !touched && !touches {
                    differing_before_touching += 1;
                    // The `CORE_MISMATCH` line printed just above is the
                    // difference; this says which sequence it is in.
                    println!("FUZZ_BEFORE_TOUCHING seed={seed} event_index={index}");
                }
            }
            touched |= touches;
            for action in &actions {
                opened_downstream += u64::from(matches!(action, Action::OpenDataStream { .. }));
                objects_sent += u64::from(matches!(action, Action::SendObject { .. }));
            }
            digest.write(format!("{actions:?}").as_bytes());
            events += 1;
        }
        touching += u64::from(touched);
        with_an_object += u64::from(objects_sent != objects_before);
        with_a_fetch_passed_on += u64::from(fetch_counts[1] != passed_on_before);
    }
    println!("FUZZ runs={runs} events={events} digest={:016x}", digest.finish());
    println!("FUZZ_WORLD {} touch={touch}", std::env::var("XMOQ_FUZZ_WORLD").unwrap_or_else(|_| "v1".to_string()));
    println!(
        "FUZZ_TOUCH sequences={runs} touching={touching} differing={differing} differing_before_touching={differing_before_touching}"
    );
    let mut by_count: Vec<(&String, &u64)> = first_differences.iter().collect();
    by_count.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (what, sequences) in by_count {
        println!("FUZZ_FIRST_DIFFERENCE sequences={sequences} {what}");
    }
    println!(
        "FUZZ_CARRIED streams_opened_to_subscribers={opened_downstream} objects_sent={objects_sent} sequences_with_an_object={with_an_object} of {runs}"
    );
    if fetches {
        let [asked, passed_on, written, held, fetch_ok] = fetch_counts;
        println!(
            "FUZZ_FETCHED asked_of_publishers={asked} streams_passed_on={passed_on} streams_written={written} joining_held={held} fetch_ok_sent={fetch_ok} sequences_with_a_stream_passed_on={with_a_fetch_passed_on} of {runs}"
        );
    }
    println!(
        "FUZZ_UNKNOWN_TO_THE_OLD_CORE events={}",
        crate::UNKNOWN_TO_THE_OLD_CORE.load(std::sync::atomic::Ordering::Relaxed)
    );
    println!(
        "FUZZ_BYSTANDER differences={} actions={}",
        crate::BYSTANDER_DIFFERENCES.load(std::sync::atomic::Ordering::Relaxed),
        crate::BYSTANDER_ACTIONS.load(std::sync::atomic::Ordering::Relaxed)
    );
}
