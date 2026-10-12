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
            send_order: FETCH_SEND_ORDER,
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
        // The I/O side reports the reset and the stop of the request
        // stream as two ends: the stream is given up once.
        (
            "cancelled, and reported twice",
            vec![
                (closes(RequestEnd::Aborted), vec![abort(0x1)]),
                (closes(RequestEnd::Aborted), vec![]),
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
fn a_stream_the_relay_writes_is_told_to_stop_once_whatever_comes_after() {
    let cancel = || Event::RequestClosed {
        session: ALICE,
        stream: FETCH_STREAM,
        how: RequestEnd::Aborted,
    };
    let malformed = || Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(7),
    };
    // (what happens first, and the code the stream is given up with; what
    //  happens then). CANCELLED is 0x1, MALFORMED_TRACK 0x12.
    let cases = [
        (cancel(), 0x1, malformed()),
        (malformed(), 0x12, cancel()),
        (malformed(), 0x12, malformed()),
    ];
    for (first, code, then) in cases {
        let mut relay = relay_with_three_objects();
        relay.step(fetch(b"a", at(3, 0), at(3, 0)));
        let aborts = |actions: &[Action]| -> Vec<Action> {
            actions
                .iter()
                .filter(|action| matches!(action, Action::AbortFetchWrite { .. }))
                .cloned()
                .collect()
        };
        let what = format!("{first:?} {then:?}");
        assert_eq!(
            aborts(&relay.step(first)),
            [Action::AbortFetchWrite {
                session: ALICE,
                stream: FETCH_STREAM,
                code,
            }],
            "{what}"
        );
        assert_eq!(aborts(&relay.step(then)), [], "{what}");
    }
}

#[test]
fn a_subgroup_sent_again_from_its_middle_without_its_id_does_not_end_the_track() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // §11.4.2: the Subgroup ID is "the Object ID of the first Object
    // transmitted in this Subgroup". Stream 7 has that Object (FIRST_OBJECT)
    // and the Objects 4 and 5, and is cut short; stream 8 sends the Subgroup
    // again from Object 5, and is not its start.
    let of_the_first_object = |first_object| SubgroupHeader {
        subgroup_id: SubgroupId::FirstObjectId,
        first_object,
        ..header(UPSTREAM_ALIAS)
    };
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(7),
        header: of_the_first_object(true),
    });
    object_arrived(&mut relay, 7, 4);
    object_arrived(&mut relay, 7, 5);
    data_stream_closed(&mut relay, 7, StreamEnd::Aborted(Some(5)));
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(8),
        header: of_the_first_object(false),
    });
    for object_id in [5, 6] {
        let actions = object_arrived(&mut relay, 8, object_id);
        let sent_on = actions
            .iter()
            .any(|action| matches!(action, Action::SendObject { .. }));
        assert!(sent_on, "{object_id}: {actions:?}");
    }
    assert_eq!(relay.counts().tracks, 1);
    // Object 6 is of a Subgroup the relay cannot name: the Group is asked
    // of the publisher.
    let actions = relay.step(fetch(b"a", at(3, 6), at(3, 6)));
    assert!(asked_the_publisher(&actions), "{actions:?}");
}

#[test]
fn a_group_with_a_subgroup_the_relay_cannot_name_is_asked_of_the_publisher() {
    // Group 3 from its start, on a stream that leaves the Subgroup ID to
    // its first Object (§11.4.2).
    // (whether the stream says that its first Object is the first of the
    //  Subgroup; whether a FETCH for the Group is answered from the cache)
    for (first_object, from_the_cache) in [(true, true), (false, false)] {
        let mut relay = relay_with_track();
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(7),
            header: SubgroupHeader {
                subgroup_id: SubgroupId::FirstObjectId,
                first_object,
                ..header(UPSTREAM_ALIAS)
            },
        });
        object_arrived(&mut relay, 7, 0);
        object_arrived(&mut relay, 7, 1);
        let actions = relay.step(fetch(b"a", at(3, 0), at(3, 1)));
        let written = matches!(actions.last(), Some(Action::WriteFetchStream { .. }));
        assert_eq!(
            (written, asked_the_publisher(&actions)),
            (from_the_cache, !from_the_cache),
            "{first_object}: {actions:?}"
        );
    }
}

#[test]
fn a_status_after_the_end_of_a_group_does_not_end_the_track() {
    let status = |object_id, status| SubgroupObject {
        status,
        payload: vec![],
        ..object(object_id)
    };
    // Object 0 and End of Group at 1 on stream 7; then, on stream 8 of
    // another Subgroup of the Group:
    let cases = [
        status(2, ObjectStatus::EndOfTrack),
        status(2, ObjectStatus::EndOfGroup),
    ];
    for last in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        relay.step(Event::Object {
            session: PUBLISHER,
            stream: StreamKey(7),
            object: status(1, ObjectStatus::EndOfGroup),
        });
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(8),
            header: SubgroupHeader {
                subgroup_id: SubgroupId::Explicit(1),
                ..header(UPSTREAM_ALIAS)
            },
        });
        let actions = relay.step(Event::Object {
            session: PUBLISHER,
            stream: StreamKey(8),
            object: last.clone(),
        });
        let sent_on = actions
            .iter()
            .any(|action| matches!(action, Action::SendObject { object, .. } if **object == last));
        assert!(sent_on, "{last:?}: {actions:?}");
        assert_eq!(
            (relay.counts().tracks, relay.counts().cached_bytes),
            (1, 0),
            "{last:?}"
        );
    }
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
fn an_object_is_cached_with_the_subgroup_its_stream_turned_out_to_be_of() {
    let mut relay = relay_with_track();
    // Group 3 on two streams: Subgroup 0 with Object 0, and a stream whose
    // Subgroup ID is the Object ID of its first Object (§11.4.2), which is
    // 1, with the Objects 1 and 2.
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(8),
        header: SubgroupHeader {
            subgroup_id: SubgroupId::FirstObjectId,
            publisher_priority: Some(200),
            ..header(UPSTREAM_ALIAS)
        },
    });
    object_arrived(&mut relay, 8, 1);
    object_arrived(&mut relay, 8, 2);
    let actions = relay.step(fetch(b"a", at(3, 0), at(3, 0)));
    let Some(Action::WriteFetchStream { pieces, .. }) = actions.last() else {
        panic!("{actions:?}");
    };
    let carried: Vec<(u64, u64, u8)> = pieces
        .iter()
        .map(|piece| match piece {
            FetchPiece::Object(cached) => (
                cached.object.object_id,
                cached.subgroup_id,
                cached.publisher_priority,
            ),
            FetchPiece::EndOfRange(..) => panic!("{piece:?}"),
        })
        .collect();
    // (Object ID; Subgroup ID; Publisher Priority)
    assert_eq!(carried, [(0, 0, 9), (1, 1, 200), (2, 1, 200)]);
}

#[test]
fn an_object_that_comes_again_with_another_payload_ends_the_track_as_malformed() {
    // The publisher sends Object 0 of Subgroup 0 of Group 3 on stream 7,
    // payload 0x61, and then the Subgroup again on stream 8: while the
    // first stream is open, which the second takes the place of, or after
    // the first was cut short, which the draft lets a publisher do (§2.2).
    //
    // (whether the first stream was cut short; the payload of Object 0 on
    //  the second; whether the track is malformed for it)
    let cases: [(bool, &[u8], bool); 5] = [
        (false, &[0x61], false),
        (true, &[0x61], false),
        // §9.1: "An endpoint that receives a duplicate Object with a
        // different ... Payload MUST treat the track as Malformed."
        (false, &[0x62], true),
        (true, &[0x62], true),
        (true, &[], true),
    ];
    for (cut_short, payload, malformed) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        if cut_short {
            data_stream_closed(&mut relay, 7, StreamEnd::Aborted(Some(5)));
        }
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(8),
            header: header(UPSTREAM_ALIAS),
        });
        let actions = relay.step(Event::Object {
            session: PUBLISHER,
            stream: StreamKey(8),
            object: SubgroupObject {
                payload: payload.to_vec(),
                ..object(0)
            },
        });
        let what = format!("{cut_short} {payload:?}: {actions:?}");
        let sent_on = actions
            .iter()
            .any(|action| matches!(action, Action::SendObject { .. }));
        // MALFORMED_TRACK (0x12): Alice's subscription ends with it, the
        // stream that brought the Object is stopped with it, and the Object
        // is not sent on.
        let ended = actions.iter().any(|action| {
            matches!(
                action,
                Action::SendRequest {
                    session: ALICE,
                    message: ControlMessage::PublishDone(PublishDone {
                        status_code: 0x12,
                        ..
                    }),
                    ..
                }
            )
        });
        let stopped = actions.contains(&Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(8),
            code: 0x12,
        });
        assert_eq!(
            (ended, stopped, sent_on),
            (malformed, malformed, !malformed),
            "{what}"
        );
        let counts = relay.counts();
        assert_eq!(
            (counts.tracks, counts.cached_bytes),
            if malformed { (0, 0) } else { (1, 1) },
            "{what}"
        );
    }
}

#[test]
fn a_group_that_takes_an_object_back_goes_on_and_is_not_answered_from_the_cache() {
    let end_of_group = |object_id| SubgroupObject {
        status: ObjectStatus::EndOfGroup,
        payload: vec![],
        ..object(object_id)
    };
    // Group 3 on stream 7; a second Subgroup of it, 1, on stream 8.
    // (what comes on stream 7 after Object 0; what comes on stream 8)
    let cases = [
        // An Object at the very Object ID of an End of Group (§2.1: "not a
        // protocol error and the Track is not malformed").
        (end_of_group(1), object(1)),
        // An End of Group where an Object is, and below one that came
        // (§9.1: an Object "can transition from existing to not existing").
        (object(1), end_of_group(1)),
        (object(2), end_of_group(1)),
    ];
    for (first, second) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        relay.step(Event::Object {
            session: PUBLISHER,
            stream: StreamKey(7),
            object: first.clone(),
        });
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(8),
            header: SubgroupHeader {
                subgroup_id: SubgroupId::Explicit(1),
                ..header(UPSTREAM_ALIAS)
            },
        });
        let actions = relay.step(Event::Object {
            session: PUBLISHER,
            stream: StreamKey(8),
            object: second.clone(),
        });
        let what = format!("{first:?} {second:?}: {actions:?}");
        // It is sent on as it came (§9.4: "A relay MUST NOT reorder or drop
        // objects received on a multi-object stream when forwarding"), and
        // the track goes on.
        let sent_on = actions.iter().any(
            |action| matches!(action, Action::SendObject { object, .. } if **object == second),
        );
        assert!(sent_on, "{what}");
        assert_eq!(relay.counts().tracks, 1, "{what}");
        // What the relay holds of the Group answers nothing any more.
        assert_eq!(relay.counts().cached_bytes, 0, "{what}");
        let actions = relay.step(fetch(b"a", at(3, 0), at(3, 1)));
        assert!(asked_the_publisher(&actions), "{what}: {actions:?}");
    }
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

/// A Standalone FETCH of `session` on `stream` for the whole Group `group`
/// of the track "a": "written" where the relay answers it from its cache,
/// "asked" where it asks the publisher, or the code it refuses with.
fn fetches_group(relay: &mut Relay, session: SessionKey, stream: u64, group: u64) -> String {
    let actions = relay.step(Event::RequestOpened {
        session,
        stream: StreamKey(stream),
        first: ControlMessage::Fetch(Fetch {
            request_id: stream * 2,
            range: FetchRange::Standalone {
                full_track_name: track(b"a"),
                start: at(group, 0),
                end: at(group, 0),
            },
            parameters: vec![],
        }),
    });
    actions
        .iter()
        .find_map(|action| match action {
            Action::WriteFetchStream { .. } => Some("written".to_string()),
            Action::OpenRequest { .. } => Some("asked".to_string()),
            Action::SendRequest {
                message: ControlMessage::RequestError(error),
                ..
            } => Some(format!(
                "{:#x}, retry {}",
                error.error_code, error.retry_interval
            )),
            _ => None,
        })
        .unwrap_or_else(|| "nothing".to_string())
}

/// A fetch stream the relay is told to stop: whose, on which request
/// stream, and the Stream Reset Error Code.
type Stopped = (SessionKey, u64, u32);

/// The publisher sends Group `group` (on the stream of that number) with
/// one Object: the cache lets go of the Group it had. The streams the relay
/// is told to stop for it, with their codes.
fn the_cache_moves_on_to(relay: &mut Relay, group: u64) -> Vec<Stopped> {
    let mut actions = data_stream_opened(relay, group);
    actions.extend(object_arrived(relay, group, 0));
    actions
        .iter()
        .filter_map(|action| match action {
            Action::AbortFetchWrite {
                session,
                stream,
                code,
            } => Some((*session, stream.0, *code)),
            _ => None,
        })
        .collect()
}

#[test]
fn what_a_fetch_stream_keeps_of_a_group_the_cache_let_go_of_is_counted_until_it_is_written() {
    // Group 3 has three Objects of one byte.
    let mut relay = relay_with_three_objects();
    assert_eq!(fetches_group(&mut relay, ALICE, 7, 3), "written");
    assert_eq!(fetches_group(&mut relay, BOB, 7, 3), "written");
    // The cache has the Group: the streams keep nothing it would not.
    assert_eq!(relay.counts().held_beyond_cache_bytes, 0);
    // The cache moves on: the three bytes are kept by the streams, and
    // counted once, though two streams carry them.
    assert_eq!(the_cache_moves_on_to(&mut relay, 8), []);
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
    // One stream is written: the other keeps them.
    relay.step(Event::FetchStreamWritten {
        session: ALICE,
        stream: StreamKey(7),
        given_up: None,
    });
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
    relay.step(Event::FetchStreamWritten {
        session: BOB,
        stream: StreamKey(7),
        given_up: None,
    });
    assert_eq!(relay.counts().held_beyond_cache_bytes, 0);
}

#[test]
fn a_session_that_has_its_share_kept_beyond_the_cache_is_refused_and_no_other() {
    // 24 bytes for the relay, so 3 for a session: what Group 3 has.
    let mut relay = relay_with_three_objects();
    relay.set_held_beyond_the_cache(24, 3);
    assert_eq!(fetches_group(&mut relay, ALICE, 7, 3), "written");
    // Her three bytes are her share and no more: the stream goes on.
    assert_eq!(the_cache_moves_on_to(&mut relay, 8), []);
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
    // Her next FETCH the cache could answer is refused: EXCESSIVE_LOAD
    // (0x9), to be retried after a second. Bob's is answered.
    assert_eq!(fetches_group(&mut relay, ALICE, 9, 8), "0x9, retry 1001");
    assert_eq!(fetches_group(&mut relay, BOB, 7, 8), "written");
    // Her stream is written: she is answered again.
    relay.step(Event::FetchStreamWritten {
        session: ALICE,
        stream: StreamKey(7),
        given_up: None,
    });
    assert_eq!(fetches_group(&mut relay, ALICE, 11, 8), "written");
}

#[test]
fn a_group_the_cache_lets_go_of_is_not_kept_for_a_session_over_its_share_or_a_relay_over_its_limit()
{
    // (the limit of the relay in bytes; who has a stream of Group 3, which
    //  has 3 bytes; the streams that are reset when the cache moves on,
    //  with EXCESSIVE_LOAD, 0x9)
    let cases: [(usize, &[SessionKey], &[Stopped]); 3] = [
        // A share of 3: nothing is reset.
        (24, &[ALICE, BOB], &[]),
        // A share of 2: Alice has 3, and her stream is reset.
        (16, &[ALICE], &[(ALICE, 7, 0x9)]),
        // The same for each of two.
        (16, &[ALICE, BOB], &[(ALICE, 7, 0x9), (BOB, 7, 0x9)]),
    ];
    for (limit, readers, reset) in cases {
        let mut relay = relay_with_three_objects();
        relay.set_held_beyond_the_cache(limit, limit / 8);
        for reader in readers {
            assert_eq!(fetches_group(&mut relay, *reader, 7, 3), "written");
        }
        assert_eq!(
            the_cache_moves_on_to(&mut relay, 8),
            reset,
            "{limit} {readers:?}"
        );
        // What is told to stop is counted no longer.
        let kept = if reset.is_empty() { 3 } else { 0 };
        assert_eq!(relay.counts().held_beyond_cache_bytes, kept, "{limit}");
    }
}

#[test]
fn where_the_relay_has_too_much_kept_and_no_session_more_than_its_share_the_one_with_the_most_is_stopped()
 {
    // 5 bytes for the relay and 3 for a session. Group 3 has three Objects
    // of one byte.
    let mut relay = relay_with_three_objects();
    relay.set_held_beyond_the_cache(5, 3);
    assert_eq!(fetches_group(&mut relay, ALICE, 7, 3), "written");
    // Group 8 comes, with three Objects as well: Alice's stream keeps 3
    // bytes, which is her share and less than the relay's 5.
    assert_eq!(the_cache_moves_on_to(&mut relay, 8), []);
    object_arrived(&mut relay, 8, 1);
    object_arrived(&mut relay, 8, 2);
    assert_eq!(fetches_group(&mut relay, BOB, 7, 8), "written");
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
    // Group 9 comes: Bob's stream keeps 3 bytes too, 6 together. Neither
    // has more than its share; the relay has more than its 5, and stops a
    // stream of the session that has the most, Alice's of the two that
    // have as much: EXCESSIVE_LOAD (0x9).
    assert_eq!(the_cache_moves_on_to(&mut relay, 9), [(ALICE, 7, 0x9)]);
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
}

#[test]
fn of_a_sessions_streams_the_one_that_carries_the_most_is_stopped_first() {
    // 40 bytes for the relay and 5 for a session.
    let mut relay = relay_with_three_objects();
    relay.set_held_beyond_the_cache(40, 5);
    // Alice has a stream of Group 3 (three bytes) on her stream 7, of
    // Group 8 (one byte) on 9, and of Group 9 (two bytes) on 11: each is
    // asked for while she has less than her 5 kept.
    assert_eq!(fetches_group(&mut relay, ALICE, 7, 3), "written");
    assert_eq!(the_cache_moves_on_to(&mut relay, 8), []);
    assert_eq!(fetches_group(&mut relay, ALICE, 9, 8), "written");
    assert_eq!(the_cache_moves_on_to(&mut relay, 9), []);
    object_arrived(&mut relay, 9, 1);
    assert_eq!(fetches_group(&mut relay, ALICE, 11, 9), "written");
    assert_eq!(relay.counts().held_beyond_cache_bytes, 4);
    // Group 10 comes: 6 bytes are kept for her. The stream that carries
    // the most is stopped, the one she opened first, and with the 3 that
    // are left she is within her share.
    assert_eq!(the_cache_moves_on_to(&mut relay, 10), [(ALICE, 7, 0x9)]);
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
}

#[test]
fn what_is_kept_beyond_the_cache_is_counted_when_the_track_ends_and_no_longer_when_the_reader_goes()
{
    let mut relay = relay_with_three_objects();
    assert_eq!(fetches_group(&mut relay, ALICE, 7, 3), "written");
    assert_eq!(relay.counts().held_beyond_cache_bytes, 0);
    // The publisher goes, and the track with it: the cache has let go of
    // the Group, and the stream keeps its three bytes.
    relay.step(Event::SessionClosed { session: PUBLISHER });
    assert_eq!(relay.counts().held_beyond_cache_bytes, 3);
    // The reader goes: nothing is kept.
    relay.step(Event::SessionClosed { session: ALICE });
    assert_eq!(relay.counts().held_beyond_cache_bytes, 0);
}
