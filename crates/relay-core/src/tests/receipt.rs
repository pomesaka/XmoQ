//! An Object counted as received from its first byte (§5.1.2): the Largest
//! Object, the filters, and a FETCH held until the Object is whole.

use xmoq_moqt::fetch::RangeEnd;
use xmoq_moqt::params::SubscriptionFilter;

use super::filters::{from, subscribe_with};
use super::*;
use crate::{CachedObject, FetchPiece};

/// The request stream of Alice's FETCH.
const FETCH_STREAM: StreamKey = StreamKey(7);

fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

fn begun(relay: &mut Relay, stream: u64, object_id: u64) -> Vec<Action> {
    relay.step(Event::ObjectBegun {
        session: PUBLISHER,
        stream: StreamKey(stream),
        object_id,
    })
}

/// The track "a" with the Objects 0, 1 and 2 of Group 3 whole on stream 7,
/// and stream 4, of Group 4, open.
fn relay_with_group_4_open() -> Relay {
    let mut relay = relay_with_track();
    data_stream_opened(&mut relay, 7);
    for object_id in 0..3 {
        object_arrived(&mut relay, 7, object_id);
    }
    data_stream_opened(&mut relay, 4);
    relay
}

/// [`relay_with_group_4_open`], with the first byte of Object 0 of Group 4
/// received: a key frame on its way.
fn relay_receiving_4_0() -> Relay {
    let mut relay = relay_with_group_4_open();
    assert_eq!(begun(&mut relay, 4, 0), []);
    relay
}

/// The `LARGEST_OBJECT` of the `SUBSCRIBE_OK` of `actions`.
fn largest_in_subscribe_ok(actions: &[Action]) -> Option<Location> {
    actions.iter().find_map(|action| match action {
        Action::SendRequest {
            message: ControlMessage::SubscribeOk(ok),
            ..
        } => ok.parameters.iter().find_map(|parameter| match parameter {
            Parameter::LargestObject(location) => Some(*location),
            _ => None,
        }),
        _ => None,
    })
}

/// What happens to the relay before a SUBSCRIBE.
type Happens = dyn Fn(&mut Relay);

#[test]
fn the_largest_object_is_raised_at_the_first_byte_of_an_object() {
    let other_subgroup = |stream, group, object_id| {
        move |relay: &mut Relay| {
            relay.step(Event::DataStreamOpened {
                session: PUBLISHER,
                stream: StreamKey(stream),
                header: SubgroupHeader {
                    subgroup_id: SubgroupId::Explicit(1),
                    ..header_of_group(UPSTREAM_ALIAS, group)
                },
            });
            begun(relay, stream, object_id);
        }
    };
    let cases: [(&str, Box<Happens>, Location); 6] = [
        ("nothing begun", Box::new(|_: &mut Relay| {}), at(3, 2)),
        (
            "Object 0 of Group 4",
            Box::new(|relay: &mut Relay| {
                begun(relay, 4, 0);
            }),
            at(4, 0),
        ),
        // An Object below the largest does not lower it.
        (
            "Object 1 of Group 3 on another Subgroup",
            Box::new(other_subgroup(11, 3, 1)),
            at(3, 2),
        ),
        // A Subgroup that was delivered whole (stream 7 ended with a FIN)
        // and comes again is left, and what it carries does not count.
        (
            "a stream the relay left",
            Box::new(|relay: &mut Relay| {
                data_stream_closed(relay, 7, StreamEnd::Fin);
                relay.step(Event::DataStreamOpened {
                    session: PUBLISHER,
                    stream: StreamKey(12),
                    header: header(UPSTREAM_ALIAS),
                });
                begun(relay, 12, 5);
            }),
            at(3, 2),
        ),
        // After the End of Group at {3, 3}, an Object at {3, 5} would make
        // the track malformed; that is decided when it is whole, and it is
        // not counted before.
        (
            "an Object after the End of Group",
            Box::new(move |relay: &mut Relay| {
                relay.step(Event::Object {
                    session: PUBLISHER,
                    stream: StreamKey(7),
                    object: SubgroupObject {
                        object_id: 3,
                        properties: vec![],
                        status: ObjectStatus::EndOfGroup,
                        payload: vec![],
                    },
                });
                other_subgroup(13, 3, 5)(relay);
            }),
            at(3, 3),
        ),
        // A header that leaves the Subgroup ID to the first Object: the
        // stream is admitted at that Object's first byte.
        (
            "a stream that leaves its Subgroup ID to its first Object",
            Box::new(|relay: &mut Relay| {
                relay.step(Event::DataStreamOpened {
                    session: PUBLISHER,
                    stream: StreamKey(14),
                    header: SubgroupHeader {
                        subgroup_id: SubgroupId::FirstObjectId,
                        ..header_of_group(UPSTREAM_ALIAS, 5)
                    },
                });
                begun(relay, 14, 2);
            }),
            at(5, 2),
        ),
    ];
    for (what, happens, largest) in cases {
        let mut relay = relay_with_group_4_open();
        happens(&mut relay);
        let actions = subscribe_to_track(&mut relay, ALICE);
        assert_eq!(largest_in_subscribe_ok(&actions), Some(largest), "{what}");
    }
}

#[test]
fn an_object_begun_in_a_later_group_answers_the_new_group_request() {
    let mut relay = relay_with_group_4_open();
    let name = track(b"a");
    if let Some(live) = relay.tracks.live_mut(&name) {
        live.new_group_asked = Some(4);
    }
    begun(&mut relay, 4, 0);
    let asked = relay.tracks.live(&name).map(|live| live.new_group_asked);
    assert_eq!(asked, Some(None));
}

/// The streams the actions open for Bob, and how many Objects go on them.
fn sent_to_bob(actions: &[Action]) -> usize {
    let bobs: Vec<OutStreamKey> = actions
        .iter()
        .filter_map(|action| match action {
            Action::OpenDataStream {
                session: BOB,
                out_stream,
                ..
            } => Some(*out_stream),
            _ => None,
        })
        .collect();
    actions
        .iter()
        .filter(|action| {
            matches!(action, Action::SendObject { out_stream, .. } if bobs.contains(out_stream))
        })
        .count()
}

#[test]
fn a_subscription_accepted_while_an_object_is_received_counts_from_it() {
    // (the filter; the Objects Bob gets of {4, 0}, then of {4, 1})
    let cases: [(Option<SubscriptionFilter>, usize, usize); 4] = [
        // §5.1.2 "pass all Objects published or received": {4, 0} was
        // received before Bob came, and a Joining FETCH would bring it.
        (None, 0, 1),
        // {Largest Object.Group, Largest Object.Object + 1}: {4, 1}.
        (Some(SubscriptionFilter::LargestObject), 0, 1),
        // {Largest Object.Group + 1, 0}: Group 4 is skipped.
        (Some(SubscriptionFilter::NextGroupStart), 0, 0),
        (Some(from(4, 0)), 1, 0),
    ];
    for (filter, first, second) in cases {
        let mut relay = relay_receiving_4_0();
        let actions = subscribe_with(&mut relay, filter);
        assert_eq!(
            largest_in_subscribe_ok(&actions),
            Some(at(4, 0)),
            "{filter:?}"
        );
        let whole = object_arrived(&mut relay, 4, 0);
        assert_eq!(sent_to_bob(&whole), first, "{filter:?}");
        // AbsoluteStart has its stream from {4, 0} on; the others get one
        // with the next Object, where they pass it.
        begun(&mut relay, 4, 1);
        let next = object_arrived(&mut relay, 4, 1);
        assert_eq!(sent_to_bob(&next), second, "{filter:?}");
    }
}

/// Alice's Relative Joining FETCH of her subscription on stream 0, from the
/// Group of her Joining Location.
fn joining_fetch(parameters: Vec<Parameter>) -> Event {
    Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::RelativeJoining {
                joining_request_id: 0,
                joining_start: 0,
            },
            parameters,
        }),
    }
}

/// What the actions do with Alice's FETCH, one line each.
fn of_the_fetch(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::SendRequest {
                session: ALICE,
                stream: FETCH_STREAM,
                message,
            } => Some(match message {
                ControlMessage::FetchOk(ok) => format!(
                    "FETCH_OK {},{}",
                    ok.end_location.group, ok.end_location.object
                ),
                ControlMessage::RequestError(error) => {
                    format!("REQUEST_ERROR {:#x}", error.error_code)
                }
                other => format!("{other:?}"),
            }),
            Action::WriteFetchStream {
                session: ALICE,
                pieces,
                ..
            } => Some(format!(
                "WRITE {}",
                pieces
                    .iter()
                    .map(|piece| match piece {
                        FetchPiece::Object(CachedObject {
                            group_id, object, ..
                        }) => format!("{group_id},{}", object.object_id),
                        FetchPiece::EndOfRange(RangeEnd::Unknown, at) => {
                            format!("unknown to {},{}", at.group, at.object)
                        }
                        FetchPiece::EndOfRange(end, at) => {
                            format!("{end:?} to {},{}", at.group, at.object)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            )),
            Action::StartReceiptTimer {
                session: ALICE,
                wait_ms,
                ..
            } => Some(format!("HOLD {wait_ms}")),
            Action::StopReceiptTimer { session: ALICE, .. } => Some("STOP_HOLD".to_string()),
            Action::OpenRequest {
                session: PUBLISHER,
                message:
                    ControlMessage::Fetch(Fetch {
                        range: FetchRange::Standalone { start, end, .. },
                        ..
                    }),
                ..
            } => Some(format!(
                "ASK {},{}..{},{}",
                start.group, start.object, end.group, end.object
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn a_joining_fetch_for_an_object_that_is_received_is_held_until_it_is_whole() {
    // (the parameters of the FETCH; what is done with it at once)
    let cases: [(Vec<Parameter>, &[&str]); 3] = [
        (vec![], &["HOLD 5000"]),
        // A shorter FILL_TIMEOUT holds it for that long.
        (vec![Parameter::FillTimeout(800)], &["HOLD 800"]),
        // FILL_TIMEOUT 0: not waited for, and the Object is unknown
        // (§10.2.5).
        (
            vec![Parameter::FillTimeout(0)],
            &["FETCH_OK 4,1", "WRITE unknown to 4,0"],
        ),
    ];
    for (parameters, expected) in cases {
        let mut relay = relay_receiving_4_0();
        subscribe_to_track(&mut relay, ALICE);
        let actions = relay.step(joining_fetch(parameters.clone()));
        assert_eq!(of_the_fetch(&actions), expected, "{parameters:?}");
        // A held FETCH counts among the session's.
        assert_eq!(relay.counts().fetches, 1, "{parameters:?}");
    }
}

/// What happens while a FETCH is held; what is done with it then; what is
/// done with it when the Object is whole after that, where the stream is
/// still there.
type HeldCase = (
    &'static str,
    Event,
    &'static [&'static str],
    Option<&'static [&'static str]>,
);

#[test]
fn a_held_fetch_is_answered_from_the_cache_or_asked_of_the_publisher() {
    let reset = Event::DataStreamClosed {
        session: PUBLISHER,
        stream: StreamKey(4),
        end: StreamEnd::Aborted(Some(0x3)),
    };
    let whole = Event::Object {
        session: PUBLISHER,
        stream: StreamKey(4),
        object: object(0),
    };
    let cancelled = Event::RequestClosed {
        session: ALICE,
        stream: FETCH_STREAM,
        how: RequestEnd::Aborted,
    };
    let timed_out = Event::ReceiptTimeout {
        session: ALICE,
        stream: FETCH_STREAM,
    };
    let cases: [HeldCase; 4] = [
        (
            "the Object is whole",
            whole.clone(),
            &["FETCH_OK 4,1", "WRITE 4,0", "STOP_HOLD"],
            None,
        ),
        // Nothing of it is cached: the publisher is asked for the range.
        (
            "the stream is reset",
            reset,
            &["ASK 4,0..4,1", "STOP_HOLD"],
            None,
        ),
        (
            "the time is over",
            timed_out,
            &["ASK 4,0..4,1", "STOP_HOLD"],
            Some(&[]),
        ),
        // Alice cancels it: nothing is written when the Object comes.
        (
            "the FETCH is cancelled",
            cancelled,
            &["STOP_HOLD"],
            Some(&[]),
        ),
    ];
    for (what, happens, then, after) in cases {
        let mut relay = relay_receiving_4_0();
        subscribe_to_track(&mut relay, ALICE);
        relay.step(joining_fetch(vec![]));
        let actions = relay.step(happens);
        assert_eq!(of_the_fetch(&actions), then, "{what}");
        if let Some(after) = after {
            let actions = relay.step(whole.clone());
            assert_eq!(of_the_fetch(&actions), after, "{what}");
        }
        // The largest Location stays where the first byte put it.
        let name = track(b"a");
        let largest = relay.tracks.live(&name).and_then(|live| live.largest);
        assert_eq!(largest, Some(at(4, 0)), "{what}");
    }
}

#[test]
fn a_standalone_fetch_that_reaches_an_object_being_received_is_held_as_well() {
    let fetch_of = |start, end| Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::Standalone {
                full_track_name: track(b"a"),
                start,
                end,
            },
            parameters: vec![],
        }),
    };
    // (the range; what is done with it at once)
    let cases: [((Location, Location), &[&str]); 3] = [
        ((at(4, 0), at(4, 1)), &["HOLD 5000"]),
        // Up to the end of Group 4: up to the Largest Object {4, 0}.
        ((at(4, 0), at(5, 0)), &["HOLD 5000"]),
        // Group 3 alone is in the cache, and not held.
        ((at(3, 0), at(3, 3)), &["FETCH_OK 3,3", "WRITE 3,0 3,1 3,2"]),
    ];
    for ((start, end), expected) in cases {
        let mut relay = relay_receiving_4_0();
        let actions = relay.step(fetch_of(start, end));
        assert_eq!(of_the_fetch(&actions), expected, "{start:?} {end:?}");
    }
}
