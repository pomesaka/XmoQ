//! `TRACK_STATUS` through the relay (§10.14).

use super::*;

/// Alice's request stream for her `TRACK_STATUS`.
const ASKING: StreamKey = StreamKey(5);

fn track_status(request_id: u64, name: &[u8]) -> ControlMessage {
    ControlMessage::TrackStatus(Subscribe {
        request_id,
        full_track_name: track(name),
        parameters: vec![],
    })
}

fn ask(relay: &mut Relay, name: &[u8]) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: ASKING,
        first: track_status(0, name),
    })
}

fn to_alice(message: ControlMessage) -> Vec<Action> {
    vec![
        Action::SendRequest {
            session: ALICE,
            stream: ASKING,
            message,
        },
        Action::FinishRequest {
            session: ALICE,
            stream: ASKING,
        },
    ]
}

fn ok(largest: Option<Location>, track_properties: Vec<KeyValuePair>) -> ControlMessage {
    ControlMessage::RequestOk(RequestOk {
        parameters: largest.map(Parameter::LargestObject).into_iter().collect(),
        track_properties,
    })
}

/// A track that has a source is answered by the relay at once, with what its
/// `SUBSCRIBE_OK` would have: the largest Location the relay knows, where it
/// knows one (§10.2.11), and the Track Properties of the source (§9.6). A
/// track that nobody publishes or announced: `DOES_NOT_EXIST` (0x10) at once.
#[test]
fn a_track_the_relay_has_is_answered_by_it() {
    // (the track asked for; whether an Object of Group 3 came first; what
    // Alice gets)
    let cases = [
        (
            "a",
            false,
            to_alice(ok(None, vec![KeyValuePair::varint(2, 5).unwrap()])),
        ),
        (
            "a",
            true,
            to_alice(ok(
                Some(Location {
                    group: 3,
                    object: 0,
                }),
                vec![KeyValuePair::varint(2, 5).unwrap()],
            )),
        ),
        (
            "nobody-published-this",
            false,
            to_alice(request_error(0x10)),
        ),
    ];
    for (name, object_before, answer) in cases {
        let mut relay = relay_with_track();
        if object_before {
            subscribe_to_track(&mut relay, BOB);
            data_stream_opened(&mut relay, 7);
            object_arrived(&mut relay, 7, 0);
        }
        assert_eq!(
            ask(&mut relay, name.as_bytes()),
            answer,
            "{name} {object_before}"
        );
        assert_eq!(relay.counts().track_statuses, 0, "{name}");
    }
}

/// A relay in which Bob announced the namespace, and Alice asked for the
/// status of the track "b", which has no source: the relay asked Bob.
fn asked_of_bob() -> (Relay, OwnRequestKey) {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 0,
            track_namespace: namespace(),
            parameters: vec![],
        }),
    });
    let actions = ask(&mut relay, b"b");
    let own = actions.iter().find_map(|action| match action {
        Action::OpenRequest {
            session: BOB,
            request,
            message: ControlMessage::TrackStatus(_),
        } => Some(*request),
        _ => None,
    });
    let own = own.unwrap_or_else(|| panic!("Bob was not asked: {actions:?}"));
    assert_eq!(
        actions,
        [
            Action::OpenRequest {
                session: BOB,
                request: own,
                // §10.2.1: Alice's parameters are not passed on.
                message: ControlMessage::TrackStatus(Subscribe {
                    request_id: 1,
                    full_track_name: track(b"b"),
                    parameters: vec![],
                }),
            },
            Action::StartUpstreamTimer {
                session: BOB,
                request: own,
            },
        ]
    );
    assert_eq!(relay.counts().track_statuses, 1);
    (relay, own)
}

#[test]
fn a_track_status_is_asked_without_the_requesters_parameters_and_past_a_source_on_its_way_out() {
    // (case; the track; whether its source has said PUBLISH_DONE, with a
    //  stream still to come; the parameters of Alice's TRACK_STATUS)
    let cases: [(&str, &[u8], bool, Vec<Parameter>); 2] = [
        (
            "a track without a source, asked with a token",
            b"b",
            false,
            vec![Parameter::AuthorizationToken(vec![1])],
        ),
        ("a track whose source is on its way out", b"a", true, vec![]),
    ];
    for (name, track_name, on_its_way_out, parameters) in cases {
        let mut relay = relay_with_track();
        if on_its_way_out {
            relay.step(Event::RequestMessage {
                session: PUBLISHER,
                stream: StreamKey(0),
                message: publish_done(0x2, 1),
            });
        }
        relay.step(Event::RequestOpened {
            session: BOB,
            stream: StreamKey(0),
            first: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: namespace(),
                parameters: vec![],
            }),
        });
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: ASKING,
            first: ControlMessage::TrackStatus(Subscribe {
                request_id: 0,
                full_track_name: track(track_name),
                parameters,
            }),
        });
        let sent: Vec<&ControlMessage> = actions
            .iter()
            .filter_map(|action| match action {
                Action::OpenRequest {
                    session: BOB,
                    message,
                    ..
                } => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(
            sent,
            [&ControlMessage::TrackStatus(Subscribe {
                request_id: 1,
                full_track_name: track(track_name),
                parameters: vec![],
            })],
            "{name}: {actions:?}"
        );
    }
}

/// What becomes of Alice's `TRACK_STATUS` that the relay asked Bob for.
#[derive(Debug, Clone, Copy)]
enum Then {
    /// Bob answers with Location {4, 1} and one Track Property.
    BobAnswers,
    /// Alice finishes her direction first, and Bob answers then.
    AliceFinishesThenBobAnswers,
    /// Bob refuses with `DOES_NOT_EXIST`.
    BobRefuses,
    /// Bob does not answer in time, and there is nobody else.
    BobTimesOut,
    /// Bob's session goes.
    BobGoes,
    /// Alice resets her request stream.
    AliceGivesUp,
}

/// What of `actions` reaches Alice's request stream, and what the relay does
/// with its own `TRACK_STATUS` on Bob.
fn split(actions: &[Action]) -> (Vec<Action>, Vec<Action>) {
    let alice = actions
        .iter()
        .filter(|action| match action {
            Action::SendRequest { session, .. }
            | Action::FinishRequest { session, .. }
            | Action::ResetRequest { session, .. } => *session == ALICE,
            _ => false,
        })
        .cloned()
        .collect();
    let bob = actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                Action::StopUpstreamTimer { .. }
                    | Action::CancelOwnRequest { .. }
                    | Action::FinishOwnRequest { .. }
            )
        })
        .cloned()
        .collect();
    (alice, bob)
}

#[test]
fn a_track_the_relay_does_not_have_is_asked_of_a_publisher_that_announced_it() {
    let bobs_answer = || {
        ok(
            Some(Location {
                group: 4,
                object: 1,
            }),
            vec![KeyValuePair::varint(4, 9).unwrap()],
        )
    };
    let stop_timer = |own| Action::StopUpstreamTimer {
        session: BOB,
        request: own,
    };
    let cancel = |own| Action::CancelOwnRequest {
        session: BOB,
        request: own,
        code: 0x1,
    };
    // (what happens; what Alice gets)
    let cases = [
        (Then::BobAnswers, to_alice(bobs_answer())),
        (Then::AliceFinishesThenBobAnswers, to_alice(bobs_answer())),
        (Then::BobRefuses, to_alice(request_error(0x10))),
        (Then::BobTimesOut, to_alice(request_error(0x2))),
        (Then::BobGoes, to_alice(request_error(0x10))),
        // The relay finishes its direction, as for a SUBSCRIBE that is
        // cancelled.
        (
            Then::AliceGivesUp,
            vec![Action::FinishRequest {
                session: ALICE,
                stream: ASKING,
            }],
        ),
    ];
    for (then, alice_gets) in cases {
        let (mut relay, own) = asked_of_bob();
        let bob_says = |relay: &mut Relay, message| {
            relay.step(Event::OwnRequestMessage {
                session: BOB,
                request: own,
                message,
            })
        };
        let actions = match then {
            Then::BobAnswers => bob_says(&mut relay, bobs_answer()),
            Then::AliceFinishesThenBobAnswers => {
                // A FIN is no cancellation (the notes, "A requester finishes
                // its direction of a request stream").
                let finished = relay.step(Event::RequestClosed {
                    session: ALICE,
                    stream: ASKING,
                    how: RequestEnd::Fin,
                });
                assert_eq!(finished, [], "{then:?}");
                bob_says(&mut relay, bobs_answer())
            }
            Then::BobRefuses => bob_says(&mut relay, request_error(0x10)),
            Then::BobTimesOut => relay.step(Event::UpstreamTimeout {
                session: BOB,
                request: own,
            }),
            Then::BobGoes => relay.step(Event::SessionClosed { session: BOB }),
            Then::AliceGivesUp => relay.step(Event::RequestClosed {
                session: ALICE,
                stream: ASKING,
                how: RequestEnd::Aborted,
            }),
        };
        let (alice, bob) = split(&actions);
        assert_eq!(alice, alice_gets, "{then:?}: {actions:?}");
        let finish = Action::FinishOwnRequest {
            session: BOB,
            request: own,
        };
        let expected_of_bob = match then {
            Then::BobAnswers | Then::AliceFinishesThenBobAnswers | Then::BobRefuses => {
                vec![stop_timer(own), finish]
            }
            Then::BobTimesOut | Then::AliceGivesUp => vec![stop_timer(own), cancel(own)],
            // Nothing can be done on a session that is gone.
            Then::BobGoes => vec![],
        };
        assert_eq!(bob, expected_of_bob, "{then:?}");
        let counts = relay.counts();
        assert_eq!(
            (counts.track_statuses, counts.own_requests),
            (0, 0),
            "{then:?}"
        );
    }
}
