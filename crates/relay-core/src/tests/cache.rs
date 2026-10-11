//! A FETCH the relay answers from its cache of the latest Group.

use xmoq_moqt::message::FetchOk;

use super::*;
use crate::{CachedObject, FetchPiece};

/// The request stream of Alice's FETCH.
const FETCH_STREAM: StreamKey = StreamKey(7);

fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

/// Alice's FETCH for a range of the track `name`, Request ID 2.
fn fetch(name: &[u8], start: Location, end: Location) -> Event {
    Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::Standalone {
                full_track_name: track(name),
                start,
                end,
            },
            parameters: vec![],
        }),
    }
}

/// An Object of a stream of [`header`], as the cache hands it over.
fn cached(group_id: u64, object_id: u64) -> FetchPiece {
    FetchPiece::Object(CachedObject {
        group_id,
        subgroup_id: 0,
        publisher_priority: 9,
        object: Arc::new(object(object_id)),
    })
}

/// `FETCH_OK` and the fetch stream the relay writes for Alice's FETCH.
fn answered(
    end_location: Location,
    track_properties: Vec<KeyValuePair>,
    pieces: Vec<FetchPiece>,
) -> Vec<Action> {
    vec![
        Action::SendRequest {
            session: ALICE,
            stream: FETCH_STREAM,
            message: ControlMessage::FetchOk(FetchOk {
                end_of_track: false,
                end_location,
                parameters: vec![],
                track_properties,
            }),
        },
        Action::WriteFetchStream {
            session: ALICE,
            stream: FETCH_STREAM,
            request_id: 2,
            pieces,
            wait_ms: 10_000,
        },
    ]
}

/// The End Location of `FETCH_OK` and the Object IDs on the stream; `None`
/// where the publisher is asked.
type Answer = Option<(Location, &'static [u64])>;

/// What happens, and what the relay does about it.
type Step = (Event, Vec<Action>);

/// What a FETCH for the track "b" from {5, 0} meets: the `LARGEST_OBJECT` of
/// the PUBLISH; its Track Properties; the Objects of Group 5 that came; the
/// End Location asked for; the Object IDs of the answer, `None` where the
/// publisher is asked.
type Case = (
    Vec<Parameter>,
    Vec<KeyValuePair>,
    &'static [u64],
    Location,
    Option<&'static [u64]>,
);

/// Whether the relay asked the publisher.
fn asked_the_publisher(actions: &[Action]) -> bool {
    matches!(
        actions,
        [
            Action::OpenRequest {
                session: PUBLISHER,
                ..
            },
            Action::StartUpstreamTimer { .. }
        ]
    )
}

/// The track "a" after the Objects 0, 1 and 2 of its Group 3 came, on stream
/// 7, with nobody subscribed.
fn relay_with_three_objects() -> Relay {
    let mut relay = relay_with_track();
    assert_eq!(data_stream_opened(&mut relay, 7), []);
    for object_id in 0..3 {
        assert_eq!(object_arrived(&mut relay, 7, object_id), []);
    }
    // One byte of payload each.
    assert_eq!(relay.counts().cached_bytes, 3);
    relay
}

#[test]
fn a_fetch_within_the_latest_group_is_answered_from_the_cache() {
    // §9.6: the Track Properties of the PUBLISH.
    let properties = || vec![KeyValuePair::varint(2, 5).unwrap()];
    // (Start Location; End Location; the End Location of FETCH_OK and the
    //  Object IDs on the stream, or `None` where the publisher is asked)
    let cases: [(Location, Location, Answer); 9] = [
        // The whole Group: beyond the Largest Object {3, 2}, so the End
        // Location is {3, 3} (§10.13).
        (at(3, 0), at(3, 0), Some((at(3, 3), &[0, 1, 2]))),
        (at(3, 0), at(3, 3), Some((at(3, 3), &[0, 1, 2]))),
        (at(3, 0), at(3, 1), Some((at(3, 1), &[0]))),
        (at(3, 1), at(3, 2), Some((at(3, 2), &[1]))),
        // To a later Group: up to the Largest Object.
        (at(3, 2), at(4, 0), Some((at(3, 3), &[2]))),
        // A range that reaches into a Group before it is the publisher's.
        (at(2, 0), at(3, 0), None),
        (at(2, 5), at(3, 2), None),
        (at(0, 0), at(2, 0), None),
        // So is one that starts after the Largest Object, which the
        // publisher refuses.
        (at(3, 3), at(3, 9), None),
    ];
    for (start, end, answer) in cases {
        let mut relay = relay_with_three_objects();
        let actions = relay.step(fetch(b"a", start, end));
        match answer {
            Some((end_location, ids)) => {
                let pieces = ids.iter().map(|id| cached(3, *id)).collect();
                assert_eq!(
                    actions,
                    answered(end_location, properties(), pieces),
                    "{start:?} {end:?}"
                );
            }
            None => assert!(
                asked_the_publisher(&actions),
                "{start:?} {end:?} {actions:?}"
            ),
        }
        assert_eq!(relay.counts().fetches, 1);
    }
}

#[test]
fn a_fetch_the_cache_answered_ends_when_its_stream_is_written() {
    let written = |given_up| Event::FetchStreamWritten {
        session: ALICE,
        stream: FETCH_STREAM,
        given_up,
    };
    let finish = Action::FinishRequest {
        session: ALICE,
        stream: FETCH_STREAM,
    };
    let reset = |code| Action::ResetRequest {
        session: ALICE,
        stream: FETCH_STREAM,
        code,
    };
    let abort = |code| Action::AbortFetchWrite {
        session: ALICE,
        stream: FETCH_STREAM,
        code,
    };
    let closes = |how| Event::RequestClosed {
        session: ALICE,
        stream: FETCH_STREAM,
        how,
    };
    // (what happens after the FETCH was answered, and what the relay does
    //  at each step)
    let cases: Vec<(&str, Vec<Step>)> = vec![
        // Written to its end: the relay leaves the request stream to the
        // subscriber for 3 seconds.
        (
            "written",
            vec![(
                written(None),
                vec![Action::StartFetchEndTimer {
                    session: ALICE,
                    stream: FETCH_STREAM,
                    wait_ms: 3_000,
                }],
            )],
        ),
        // The subscriber did not take the stream: TOO_FAR_BEHIND (0x5).
        ("not taken", vec![(written(Some(0x5)), vec![reset(0x5)])]),
        // The subscriber cancels its FETCH (§5.2): the fetch stream is
        // reset with CANCELLED (0x1), and the relay's direction of the
        // request stream when that is reported.
        (
            "cancelled",
            vec![
                (closes(RequestEnd::Aborted), vec![abort(0x1)]),
                (written(Some(0x1)), vec![reset(0x1)]),
            ],
        ),
        // A subscriber that only finishes its direction has given nothing
        // up.
        (
            "the subscriber finished",
            vec![
                (closes(RequestEnd::Fin), vec![]),
                (written(None), vec![finish]),
            ],
        ),
    ];
    for (name, steps) in cases {
        let mut relay = relay_with_three_objects();
        relay.step(fetch(b"a", at(3, 0), at(3, 0)));
        for (event, expected) in steps {
            assert_eq!(relay.step(event), expected, "{name}");
        }
        assert_eq!(relay.counts().fetches, 0, "{name}");
        // What comes late for it does nothing.
        assert_eq!(relay.step(written(None)), [], "{name}");
    }
}

#[test]
fn a_track_found_malformed_resets_the_stream_the_relay_writes_from_its_cache() {
    let mut relay = relay_with_three_objects();
    relay.step(fetch(b"a", at(3, 0), at(3, 0)));
    let actions = relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(7),
    });
    // §2.4.2: "reset any fetch streams with Status Code MALFORMED_TRACK"
    // (0x12).
    assert_eq!(
        actions.first(),
        Some(&Action::AbortFetchWrite {
            session: ALICE,
            stream: FETCH_STREAM,
            code: 0x12,
        }),
        "{actions:?}"
    );
    // The cache went with the track.
    assert_eq!(relay.counts().cached_bytes, 0);
    assert_eq!(
        relay.step(Event::FetchStreamWritten {
            session: ALICE,
            stream: FETCH_STREAM,
            given_up: Some(0x12),
        }),
        [Action::ResetRequest {
            session: ALICE,
            stream: FETCH_STREAM,
            code: 0x12,
        }]
    );
}

#[test]
fn a_stream_the_relay_writes_goes_on_when_its_track_ends() {
    let mut relay = relay_with_three_objects();
    relay.step(fetch(b"a", at(3, 0), at(3, 0)));
    // The publisher goes: what was handed over to be written is written.
    let actions = relay.step(Event::SessionClosed { session: PUBLISHER });
    let aborted = actions
        .iter()
        .any(|action| matches!(action, Action::AbortFetchWrite { .. }));
    assert!(!aborted, "{actions:?}");
    assert_eq!(
        (relay.counts().cached_bytes, relay.counts().fetches),
        (0, 1)
    );
}

/// A relay whose publisher offers the track "b" under the alias 41, with
/// these parameters and Track Properties.
fn relay_with_track_b(parameters: Vec<Parameter>, track_properties: Vec<KeyValuePair>) -> Relay {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"b"),
            track_alias: 41,
            parameters,
            track_properties,
        }),
    });
    relay
}

/// The Objects `objects` of Group 5 of the track "b" come on stream 9.
fn group_5_comes(relay: &mut Relay, objects: &[u64]) {
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(9),
        header: header_of_group(41, 5),
    });
    for object_id in objects {
        object_arrived(relay, 9, *object_id);
    }
}

#[test]
fn what_the_cache_cannot_answer_for_is_asked_of_the_publisher() {
    let largest = |object| vec![Parameter::LargestObject(at(5, object))];
    // MAX_CACHE_DURATION (0x04, §12.3).
    let short_lived = || vec![KeyValuePair::varint(4, 1_000).unwrap()];
    let immutable = || vec![KeyValuePair::bytes(0x0b, vec![0x02, 0x05]).unwrap()];
    let cases: [Case; 9] = [
        (vec![], vec![], &[0, 1], at(5, 0), Some(&[0, 1])),
        // The publisher said that the track is at {5, 2}, and Object 2 has
        // not come: what lies before the End Location of FETCH_OK and is
        // not on the stream would read as not existing.
        (largest(2), vec![], &[0, 1], at(5, 0), None),
        (largest(2), vec![], &[0, 1], at(5, 3), None),
        // Up to what came, the relay knows.
        (largest(2), vec![], &[0, 1], at(5, 2), Some(&[0, 1])),
        // The Group is not there from its first Object.
        (vec![], vec![], &[1, 2], at(5, 0), None),
        // An Object is missing in it.
        (vec![], vec![], &[0, 2], at(5, 0), None),
        // A track whose Objects may be forwarded for a time only is not
        // cached, and one with Immutable Properties (0x0B), which can say
        // so where the relay does not look.
        (vec![], short_lived(), &[0, 1], at(5, 0), None),
        (vec![], immutable(), &[0, 1], at(5, 0), None),
        // Nothing came.
        (largest(2), vec![], &[], at(5, 0), None),
    ];
    for (parameters, properties, came, end, answer) in cases {
        let mut relay = relay_with_track_b(parameters.clone(), properties.clone());
        group_5_comes(&mut relay, came);
        let actions = relay.step(fetch(b"b", at(5, 0), end));
        let what = format!("{parameters:?} {properties:?} {came:?} {end:?}");
        match answer {
            Some(ids) => {
                let pieces: Vec<FetchPiece> = ids.iter().map(|id| cached(5, *id)).collect();
                let Some(Action::WriteFetchStream {
                    pieces: written, ..
                }) = actions.last()
                else {
                    panic!("{what}: {actions:?}");
                };
                assert_eq!(written, &pieces, "{what}");
            }
            None => assert!(asked_the_publisher(&actions), "{what}: {actions:?}"),
        }
    }
}

#[test]
fn a_track_is_not_cached_when_any_of_its_publishers_said_so() {
    // MAX_CACHE_DURATION (0x04, §12.3).
    let short_lived = || vec![KeyValuePair::varint(4, 1_000).unwrap()];
    // (the Track Properties of the first publisher of the track "b"; those
    //  of the second; whether a FETCH is answered from the cache)
    let cases = [
        (vec![], vec![], true),
        // The Objects below come from the first publisher: the limit is
        // for what was "received through this subscription" (§12.3), and
        // the properties the track has are the ones that came last.
        (short_lived(), vec![], false),
        (vec![], short_lived(), false),
        (short_lived(), short_lived(), false),
    ];
    for (first, second, cached) in cases {
        let mut relay = relay_with_track_b(vec![], first.clone());
        // Bob publishes the track as well, under his alias 77.
        relay.step(Event::RequestOpened {
            session: BOB,
            stream: StreamKey(0),
            first: ControlMessage::Publish(Publish {
                request_id: 0,
                full_track_name: track(b"b"),
                track_alias: 77,
                parameters: vec![],
                track_properties: second.clone(),
            }),
        });
        // The track "a" has one source, and "b" two.
        assert_eq!(relay.counts().sources, 3, "{first:?} {second:?}");
        group_5_comes(&mut relay, &[0, 1]);
        let actions = relay.step(fetch(b"b", at(5, 0), at(5, 0)));
        let written = matches!(actions.last(), Some(Action::WriteFetchStream { .. }));
        assert_eq!(written, cached, "{first:?} {second:?}: {actions:?}");
        assert_eq!(
            asked_the_publisher(&actions),
            !cached,
            "{first:?} {second:?}: {actions:?}"
        );
        assert_eq!(relay.counts().cached_bytes, if cached { 2 } else { 0 });
    }
}

#[test]
fn what_was_cached_is_not_used_once_a_publisher_says_the_track_is_not_to_be_cached() {
    let mut relay = relay_with_track_b(vec![], vec![]);
    group_5_comes(&mut relay, &[0, 1]);
    assert_eq!(relay.counts().cached_bytes, 2);
    // Bob publishes the track with MAX_CACHE_DURATION (0x04), and no Object
    // has come since: what the relay holds is still there, and answers
    // nothing.
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: ControlMessage::Publish(Publish {
            request_id: 0,
            full_track_name: track(b"b"),
            track_alias: 77,
            parameters: vec![],
            track_properties: vec![KeyValuePair::varint(4, 1_000).unwrap()],
        }),
    });
    let actions = relay.step(fetch(b"b", at(5, 0), at(5, 0)));
    assert!(asked_the_publisher(&actions), "{actions:?}");
    // It goes with the next Object that is passed on.
    object_arrived(&mut relay, 9, 2);
    assert_eq!(relay.counts().cached_bytes, 0);
}

#[test]
fn a_joining_fetch_for_the_group_of_the_joining_location_is_answered_from_the_cache() {
    let mut relay = relay_with_track();
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    object_arrived(&mut relay, 7, 1);
    // Alice subscribes: her Joining Location is {3, 1}. Object 2 comes
    // after that, on her subscription.
    assert_eq!(
        subscribe_to_track(&mut relay, ALICE),
        [subscribe_ok(ALICE, Some(at(3, 1)))]
    );
    object_arrived(&mut relay, 7, 2);
    // A Joining FETCH from the start of that Group (offset 0) ends at the
    // Joining Location (§10.12.2.1): the Objects 0 and 1, and not Object 2.
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::RelativeJoining {
                joining_request_id: 0,
                joining_start: 0,
            },
            parameters: vec![],
        }),
    });
    assert_eq!(
        actions,
        answered(
            at(3, 2),
            vec![KeyValuePair::varint(2, 5).unwrap()],
            vec![cached(3, 0), cached(3, 1)]
        )
    );
    // One Group back, the range begins in a Group the relay does not keep.
    let mut relay = relay_with_three_objects();
    subscribe_to_track(&mut relay, ALICE);
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::RelativeJoining {
                joining_request_id: 0,
                joining_start: 1,
            },
            parameters: vec![],
        }),
    });
    assert!(asked_the_publisher(&actions), "{actions:?}");
}

#[test]
fn the_cache_keeps_what_the_relay_passes_on_and_lets_go_of_it_with_the_track() {
    let mut relay = relay_with_three_objects();
    // A later Group takes the place of Group 3: one Object, one byte.
    data_stream_opened(&mut relay, 4);
    object_arrived(&mut relay, 4, 0);
    assert_eq!(relay.counts().cached_bytes, 1);
    assert!(asked_the_publisher(&relay.step(fetch(
        b"a",
        at(3, 0),
        at(3, 0)
    ))));
    // A second stream for a Subgroup that was delivered is left by the
    // relay, and what it carries is not kept: the Object 5 of Group 4 does
    // not get in that way.
    data_stream_closed(&mut relay, 4, StreamEnd::Fin);
    data_stream_opened(&mut relay, 4);
    object_arrived(&mut relay, 4, 5);
    assert_eq!(relay.counts().cached_bytes, 1);
    // The track ends with its publisher, and its cache with it.
    relay.step(Event::SessionClosed { session: PUBLISHER });
    assert_eq!(relay.counts().cached_bytes, 0);
}
