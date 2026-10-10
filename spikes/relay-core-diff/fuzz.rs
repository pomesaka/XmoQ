//! Random event sequences through two versions of the relay core at once.
//! `Relay::handle` is patched by `compare.py` to feed every event to both, to
//! print a `CORE_MISMATCH` line where their actions differ, and to panic in
//! tests where they differ in more than order.
//!
//! The sequences are the same in every run (the seeds are fixed). What may
//! differ from run to run is the number of order-only mismatches: both cores
//! iterate over `HashMap`s, whose order is random per process.

use xmoq_moqt::data::{ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Publish, PublishDone, PublishNamespace, ReasonPhrase, RequestError,
    RequestUpdate, Subscribe, SubscribeOk,
};
use xmoq_moqt::name::{FullTrackName, TrackNamespace};
use xmoq_moqt::params::Parameter;
use xmoq_moqt::session::Role;

use crate::{
    Event, OutStreamKey, OwnRequestEnd, OwnRequestKey, Relay, RequestEnd, SessionKey, StreamEnd,
    StreamKey,
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
    open: bool,
}

fn new_request(peer: &mut Peer) -> (u64, StreamKey) {
    let opened = (peer.next_request, StreamKey(peer.next_stream));
    peer.next_request += 2;
    peer.next_stream += 1;
    opened
}

/// One of 25 kinds of event, about one of 4 sessions, 2 tracks, 2 Track
/// Aliases, 4 data streams a session, 6 requests of the relay and 12 streams
/// to subscribers.
fn random_event(rng: &mut Rng, peers: &mut [Peer]) -> Event {
    let who = rng.next(4);
    let session = SessionKey(who + 1);
    let peer = &mut peers[usize::try_from(who).unwrap()];
    let stream = StreamKey(rng.next(peer.next_stream.max(1) + 1));
    let data_stream = StreamKey(100 + rng.next(4));
    let request = OwnRequestKey(rng.next(6));
    let name = rng.next(2);
    let alias = 40 + rng.next(2);
    match rng.next(30) {
        0..=2 => {
            let (id, new) = new_request(peer);
            let forward_0 = rng.next(4) == 0;
            Event::RequestOpened {
                session,
                stream: new,
                first: ControlMessage::Subscribe(Subscribe {
                    request_id: id,
                    full_track_name: track(name),
                    parameters: if forward_0 { vec![Parameter::Forward(false)] } else { vec![] },
                }),
            }
        }
        3..=4 => {
            let (id, new) = new_request(peer);
            Event::RequestOpened {
                session,
                stream: new,
                first: ControlMessage::Publish(Publish {
                    request_id: id,
                    full_track_name: track(name),
                    track_alias: alias,
                    parameters: vec![],
                    track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
                }),
            }
        }
        5..=6 => {
            let (id, new) = new_request(peer);
            let fields = if rng.next(3) == 0 { vec![] } else { vec![b"xmoq".to_vec()] };
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
        16..=18 => Event::DataStreamOpened {
            session,
            stream: data_stream,
            header: SubgroupHeader {
                track_alias: alias,
                group_id: rng.next(3),
                subgroup_id: SubgroupId::Zero,
                publisher_priority: Some(0),
                properties: false,
                end_of_group: false,
                first_object: true,
            },
        },
        19..=20 => Event::Object {
            session,
            stream: data_stream,
            object: SubgroupObject {
                object_id: rng.next(4),
                properties: vec![],
                status: ObjectStatus::Normal,
                payload: vec![7; 3],
            },
        },
        21..=22 => Event::DataStreamClosed {
            session,
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
            1 => Event::DrainTimeout { session, track_alias: alias },
            2 => Event::EarlyStreamTimeout { session, stream: data_stream },
            _ => Event::RequestTimeout { session, stream },
        },
        27 => {
            peer.open = false;
            Event::SessionClosed { session }
        }
        // The I/O side never opens a session under a key that is in use.
        28 if !peer.open => {
            *peer = Peer { next_request: 0, next_stream: 0, open: true };
            Event::SessionOpened { session, role: Role::Server }
        }
        _ => Event::DataStreamClosed {
            session,
            stream: data_stream,
            end: StreamEnd::Aborted(None),
        },
    }
}

fn setting(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

#[test]
fn the_two_cores_answer_random_events_alike() {
    let (runs, length) = (setting("XMOQ_FUZZ_RUNS", 20_000), setting("XMOQ_FUZZ_LEN", 80));
    let mut events = 0u64;
    for seed in 1..=runs {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut relay = Relay::new("fuzz/0");
        let mut peers: Vec<Peer> = (0..4)
            .map(|_| Peer { next_request: 0, next_stream: 0, open: true })
            .collect();
        for session in 1..=4 {
            relay.handle(Event::SessionOpened { session: SessionKey(session), role: Role::Server });
        }
        for _ in 0..length {
            relay.handle(random_event(&mut rng, &mut peers));
            events += 1;
        }
    }
    println!("FUZZ runs={runs} events={events}");
}
