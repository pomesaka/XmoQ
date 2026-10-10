//! Routing by namespace (§9.5), and what a FIN of a requester means.

use super::*;

const CAROL: SessionKey = SessionKey(4);

fn namespace_of(fields: &[&[u8]]) -> TrackNamespace {
    TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
}

fn track_in(fields: &[&[u8]]) -> FullTrackName {
    FullTrackName::new(namespace_of(fields), b"a".to_vec()).unwrap()
}

/// A relay with four sessions and nothing else.
fn relay() -> Relay {
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE, BOB, CAROL] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    relay
}

/// `session` announces a namespace on `stream`, with the Request ID `stream`
/// times two (the streams of a session are numbered from 0 in these tests).
fn announce(relay: &mut Relay, session: SessionKey, stream: u64, fields: &[&[u8]]) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session,
        stream: StreamKey(stream),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: stream * 2,
            track_namespace: namespace_of(fields),
            parameters: vec![],
        }),
    })
}

fn subscribe_to(
    relay: &mut Relay,
    session: SessionKey,
    stream: u64,
    fields: &[&[u8]],
) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session,
        stream: StreamKey(stream),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: stream * 2,
            full_track_name: track_in(fields),
            parameters: vec![],
        }),
    })
}

/// The SUBSCRIBE the relay sends upstream, and the timer for its answer. The
/// relay is the server of every session, so its Request IDs are odd (§10.1).
fn asked(session: SessionKey, own: u64, request_id: u64, fields: &[&[u8]]) -> [Action; 2] {
    [
        Action::OpenRequest {
            session,
            request: OwnRequestKey(own),
            message: ControlMessage::Subscribe(Subscribe {
                request_id,
                full_track_name: track_in(fields),
                // §9.2: Forward State 1 upstream, which is what no FORWARD
                // parameter means (§10.2.12), and nothing of the downstream
                // SUBSCRIBE.
                parameters: vec![],
            }),
        },
        Action::StartUpstreamTimer {
            session,
            request: OwnRequestKey(own),
        },
    ]
}

fn refused(session: SessionKey, stream: u64, error: RequestError) -> [Action; 2] {
    [
        Action::SendRequest {
            session,
            stream: StreamKey(stream),
            message: ControlMessage::RequestError(error),
        },
        Action::FinishRequest {
            session,
            stream: StreamKey(stream),
        },
    ]
}

fn error(error_code: u64) -> RequestError {
    RequestError {
        error_code,
        retry_interval: 0,
        error_reason: ReasonPhrase::default(),
        redirect: None,
    }
}

fn upstream(session: SessionKey, own: u64, message: ControlMessage) -> Event {
    Event::OwnRequestMessage {
        session,
        request: OwnRequestKey(own),
        message,
    }
}

/// The relay stops waiting for the publisher's answer to its request 0.
fn answered() -> Action {
    Action::StopUpstreamTimer {
        session: PUBLISHER,
        request: OwnRequestKey(0),
    }
}

fn upstream_subscribe_ok(largest: Option<Location>) -> ControlMessage {
    ControlMessage::SubscribeOk(SubscribeOk {
        track_alias: UPSTREAM_ALIAS,
        parameters: largest.map(Parameter::LargestObject).into_iter().collect(),
        track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
    })
}

/// A relay where PUBLISHER announced `(xmoq)`, Alice subscribed to
/// `(xmoq)/a`, and the publisher accepted.
fn relay_with_routed_track() -> Relay {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    assert_eq!(
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]),
        asked(PUBLISHER, 0, 1, &[b"xmoq"])
    );
    assert_eq!(
        relay.step(upstream(PUBLISHER, 0, upstream_subscribe_ok(None))),
        [answered(), subscribe_ok(ALICE, None)]
    );
    relay
}

/// What the relay holds for routing:
/// `(namespaces, pending tracks, own requests, tracks, subscriptions)`.
fn routed(relay: &Relay) -> (usize, usize, usize, usize, usize) {
    let counts = relay.counts();
    (
        counts.namespaces,
        counts.pending_tracks,
        counts.own_requests,
        counts.tracks,
        counts.subscriptions,
    )
}

enum Step {
    Announce(SessionKey, u64, &'static [&'static [u8]]),
    /// Resets the stream of a `PUBLISH_NAMESPACE`: withdraws it (§9.5).
    Withdraw(SessionKey, u64),
    /// Finishes the publisher's direction of that stream and nothing else.
    Fin(SessionKey, u64),
    Leave(SessionKey),
    /// Sends PUBLISH for `(xmoq)/other`, and no `PUBLISH_NAMESPACE`.
    PublishOther(SessionKey),
}

/// What happened before, the namespace Alice subscribes in, and the session
/// that gets the SUBSCRIBE; `None`: `REQUEST_ERROR` `DOES_NOT_EXIST`.
type RoutingCase = (
    &'static str,
    Vec<Step>,
    &'static [&'static [u8]],
    Option<SessionKey>,
);

fn routing_cases() -> [RoutingCase; 14] {
    use Step::{Announce, Fin, Leave, PublishOther, Withdraw};
    [
        ("nobody announced anything", vec![], &[b"xmoq"], None),
        (
            "the same namespace",
            vec![Announce(PUBLISHER, 0, &[b"xmoq"])],
            &[b"xmoq"],
            Some(PUBLISHER),
        ),
        (
            "a prefix of the track's namespace (§9.5: fewer fields match)",
            vec![Announce(PUBLISHER, 0, &[b"xmoq"])],
            &[b"xmoq", b"live"],
            Some(PUBLISHER),
        ),
        (
            "a namespace without fields is a prefix of every other (§2.4.1, §9.5)",
            vec![Announce(PUBLISHER, 0, &[])],
            &[b"xmoq"],
            Some(PUBLISHER),
        ),
        (
            "a namespace longer than the track's does not match",
            vec![Announce(PUBLISHER, 0, &[b"xmoq", b"live"])],
            &[b"xmoq"],
            None,
        ),
        (
            "a field is matched whole (§9.5: (foobar) is not (foo, bar))",
            vec![Announce(PUBLISHER, 0, &[b"xmo"])],
            &[b"xmoq"],
            None,
        ),
        (
            "another second field",
            vec![Announce(PUBLISHER, 0, &[b"xmoq", b"vod"])],
            &[b"xmoq", b"live"],
            None,
        ),
        (
            "two publishers of the namespace: the one that announced first",
            vec![
                Announce(BOB, 0, &[b"xmoq"]),
                Announce(PUBLISHER, 0, &[b"xmoq"]),
            ],
            &[b"xmoq"],
            Some(BOB),
        ),
        (
            "the order of announcing decides, not the longer match",
            vec![
                Announce(PUBLISHER, 0, &[b"xmoq"]),
                Announce(BOB, 0, &[b"xmoq", b"live"]),
            ],
            &[b"xmoq", b"live"],
            Some(PUBLISHER),
        ),
        (
            "the first one withdrew: the next",
            vec![
                Announce(BOB, 0, &[b"xmoq"]),
                Announce(PUBLISHER, 0, &[b"xmoq"]),
                Withdraw(BOB, 0),
            ],
            &[b"xmoq"],
            Some(PUBLISHER),
        ),
        (
            "a FIN of the announcing stream does not withdraw (§6.2, §3.3.2)",
            vec![Announce(PUBLISHER, 0, &[b"xmoq"]), Fin(PUBLISHER, 0)],
            &[b"xmoq"],
            Some(PUBLISHER),
        ),
        (
            "the first one left: the next",
            vec![
                Announce(BOB, 0, &[b"xmoq"]),
                Announce(PUBLISHER, 0, &[b"xmoq"]),
                Leave(BOB),
            ],
            &[b"xmoq"],
            Some(PUBLISHER),
        ),
        (
            "a PUBLISH of another track announces nothing (§9.5)",
            vec![PublishOther(PUBLISHER)],
            &[b"xmoq"],
            None,
        ),
        (
            "the subscriber's own namespace (§5.1: relays handle a self-subscription)",
            vec![Announce(ALICE, 1, &[b"xmoq"])],
            &[b"xmoq"],
            Some(ALICE),
        ),
    ]
}

fn apply(relay: &mut Relay, step: &Step) {
    match *step {
        Step::Announce(session, stream, fields) => {
            announce(relay, session, stream, fields);
        }
        Step::Withdraw(session, stream) => {
            relay.step(Event::RequestClosed {
                session,
                stream: StreamKey(stream),
                how: RequestEnd::Aborted,
            });
        }
        Step::Fin(session, stream) => {
            relay.step(Event::RequestClosed {
                session,
                stream: StreamKey(stream),
                how: RequestEnd::Fin,
            });
        }
        Step::Leave(session) => {
            relay.step(Event::SessionClosed { session });
        }
        Step::PublishOther(session) => {
            relay.step(Event::RequestOpened {
                session,
                stream: StreamKey(0),
                first: publish(0, b"other"),
            });
        }
    }
}

#[test]
fn a_subscribe_goes_to_the_first_publisher_of_a_matching_namespace() {
    for (name, before, fields, publisher) in routing_cases() {
        let mut relay = relay();
        for step in &before {
            apply(&mut relay, step);
        }
        let actions = subscribe_to(&mut relay, ALICE, 0, fields);
        let expected = match publisher {
            Some(publisher) => asked(publisher, 0, 1, fields).to_vec(),
            // DOES_NOT_EXIST (0x10), at once (§10.2.6).
            None => refused(ALICE, 0, error(0x10)).to_vec(),
        };
        assert_eq!(actions, expected, "{name}");
    }
}

/// What comes from the publisher, what the relay does to its own request
/// first, and what Alice and Bob get; `None`: `SUBSCRIBE_OK`.
type AnswerCase = (&'static str, Event, Vec<Action>, Option<RequestError>);

/// The largest Object the publisher names in its `SUBSCRIBE_OK`.
const UPSTREAM_LARGEST: Location = Location {
    group: 5,
    object: 3,
};

/// The publisher answers the relay's SUBSCRIBE with this `REQUEST_ERROR`.
fn rejected(error: RequestError) -> Event {
    upstream(PUBLISHER, 0, ControlMessage::RequestError(error))
}

fn answer_cases() -> [AnswerCase; 7] {
    // 200 bytes; 128 of them are passed on.
    let long_reason = ReasonPhrase::new(vec![b'r'; 200]).unwrap();
    let cut_reason = ReasonPhrase::new(vec![b'r'; 128]).unwrap();
    let cancelled = Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x1,
    };
    let finished = Action::FinishOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
    };
    [
        (
            "SUBSCRIBE_OK",
            upstream(PUBLISHER, 0, upstream_subscribe_ok(Some(UPSTREAM_LARGEST))),
            vec![answered()],
            None,
        ),
        (
            // UNAUTHORIZED (0x1), retry after 4 ms: passed on as it is.
            "REQUEST_ERROR",
            rejected(RequestError {
                error_code: 0x1,
                retry_interval: 5,
                error_reason: ReasonPhrase::new(b"no".to_vec()).unwrap(),
                redirect: None,
            }),
            vec![finished.clone()],
            Some(RequestError {
                error_code: 0x1,
                retry_interval: 5,
                error_reason: ReasonPhrase::new(b"no".to_vec()).unwrap(),
                redirect: None,
            }),
        ),
        (
            "REQUEST_ERROR with a long reason",
            rejected(RequestError {
                error_code: 0x0,
                retry_interval: 0,
                error_reason: long_reason,
                redirect: None,
            }),
            vec![finished.clone()],
            Some(RequestError {
                error_code: 0x0,
                retry_interval: 0,
                error_reason: cut_reason,
                redirect: None,
            }),
        ),
        (
            // REDIRECT (0x34) is not passed on: DOES_NOT_EXIST (0x10).
            "REQUEST_ERROR REDIRECT",
            rejected(RequestError {
                error_code: 0x34,
                retry_interval: 0,
                error_reason: ReasonPhrase::default(),
                redirect: Some(xmoq_moqt::message::Redirect {
                    connect_uri: vec![],
                    full_track_name: track(b"elsewhere"),
                }),
            }),
            vec![finished],
            Some(error(0x10)),
        ),
        (
            // No answer and no stream: DOES_NOT_EXIST (0x10).
            "the publisher ends the stream without an answer",
            Event::OwnRequestClosed {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                how: OwnRequestEnd::Fin,
            },
            vec![cancelled.clone()],
            Some(error(0x10)),
        ),
        (
            // TIMEOUT (0x2), §10.6.2.
            "the publisher does not answer in time",
            Event::UpstreamTimeout {
                session: PUBLISHER,
                request: OwnRequestKey(0),
            },
            vec![cancelled],
            Some(error(0x2)),
        ),
        (
            // Nothing can be done on a session that is gone.
            "the publisher leaves",
            Event::SessionClosed { session: PUBLISHER },
            vec![],
            Some(error(0x10)),
        ),
    ]
}

#[test]
fn the_publishers_answer_is_the_answer_to_everyone_who_waited() {
    for (name, from_publisher, own_request, answer) in answer_cases() {
        let mut relay = relay();
        announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
        assert_eq!(
            subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]),
            asked(PUBLISHER, 0, 1, &[b"xmoq"]),
            "{name}"
        );
        // Bob comes while the publisher has not answered: no second SUBSCRIBE
        // goes upstream (§9.4), and nothing is answered yet.
        assert_eq!(subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]), [], "{name}");

        let mut expected = own_request;
        match answer {
            None => expected.extend([
                subscribe_ok(ALICE, Some(UPSTREAM_LARGEST)),
                subscribe_ok(BOB, Some(UPSTREAM_LARGEST)),
            ]),
            Some(error) => {
                expected.extend(refused(ALICE, 0, error.clone()));
                expected.extend(refused(BOB, 0, error));
            }
        }
        assert_eq!(relay.step(from_publisher), expected, "{name}");
        assert_eq!(relay.counts().pending_tracks, 0, "{name}");
        // The timer that was started fires into nothing.
        assert_eq!(
            relay.step(Event::UpstreamTimeout {
                session: PUBLISHER,
                request: OwnRequestKey(0),
            }),
            [],
            "{name}"
        );
    }
}

#[test]
fn objects_of_a_routed_track_reach_the_subscriber() {
    let mut relay = relay_with_routed_track();
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [open_data_stream(ALICE, 0, header(0))]
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), [send_object(0, 0)]);
    // A subscriber that comes now is answered at once, with the largest
    // Object the relay has seen (§9.5, §10.2.11).
    assert_eq!(
        subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]),
        [subscribe_ok(
            BOB,
            Some(Location {
                group: 3,
                object: 0
            })
        )]
    );
    assert_eq!(routed(&relay), (1, 0, 1, 1, 2));
}

#[test]
fn a_stream_that_overtakes_subscribe_ok_belongs_to_the_routed_track() {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
    // The publisher's first stream is read before its SUBSCRIBE_OK.
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [Action::StartEarlyTimer {
            session: PUBLISHER,
            stream: StreamKey(7),
        }]
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), []);
    assert_eq!(
        relay.step(upstream(PUBLISHER, 0, upstream_subscribe_ok(None))),
        [
            answered(),
            subscribe_ok(ALICE, None),
            open_data_stream(ALICE, 0, header(0)),
            send_object(0, 0),
        ]
    );
}

#[test]
fn the_upstream_subscription_is_cancelled_when_its_last_subscriber_leaves() {
    // (how Alice leaves, what she is told)
    let leavings = [
        (
            "cancels the request",
            Event::RequestClosed {
                session: ALICE,
                stream: StreamKey(0),
                how: RequestEnd::Aborted,
            },
            vec![Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(0),
            }],
        ),
        (
            "disconnects",
            Event::SessionClosed { session: ALICE },
            vec![],
        ),
    ];
    for (name, leaving, towards_alice) in leavings {
        let mut relay = relay_with_routed_track();
        data_stream_opened(&mut relay, 7);

        // Her stream is reset if she is still there, the publisher's stream
        // is stopped, and the relay cancels its own SUBSCRIBE, all with
        // CANCELLED (0x1).
        let mut expected = towards_alice.clone();
        if !towards_alice.is_empty() {
            expected.push(Action::ResetDataStream {
                out_stream: OutStreamKey(0),
                code: 0x1,
            });
        }
        expected.extend([
            Action::StopDataStream {
                session: PUBLISHER,
                stream: StreamKey(7),
                code: 0x1,
            },
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code: 0x1,
            },
        ]);
        assert_eq!(relay.step(leaving), expected, "{name}");
        // The namespace stays; the track and the relay's request are gone.
        assert_eq!(routed(&relay), (1, 0, 0, 0, 0), "{name}");

        // A subscriber that comes right after starts over: a new SUBSCRIBE
        // upstream, with the relay's next Request ID. The publisher may see
        // it before the cancellation of the old one, and what it answers
        // then is passed on.
        assert_eq!(
            subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]),
            asked(PUBLISHER, 1, 3, &[b"xmoq"]),
            "{name}"
        );
    }
}

#[test]
fn a_track_that_came_with_publish_stays_without_subscribers() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: StreamKey(0),
        how: RequestEnd::Aborted,
    });
    assert_eq!(held(&relay), (3, 1, 0, 0, 0));
}

#[test]
fn the_upstream_subscribe_is_cancelled_when_nobody_waits_for_its_answer() {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
    subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]);

    // Alice gives up; Bob still waits.
    assert_eq!(
        relay.step(Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        }),
        [Action::FinishRequest {
            session: ALICE,
            stream: StreamKey(0),
        }]
    );
    assert_eq!(routed(&relay), (1, 1, 1, 0, 0));
    // Bob leaves: the relay cancels with CANCELLED (0x1).
    assert_eq!(
        relay.step(Event::SessionClosed { session: BOB }),
        [Action::CancelOwnRequest {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            code: 0x1,
        }]
    );
    assert_eq!(routed(&relay), (1, 0, 0, 0, 0));
    // An answer that was under way finds nobody.
    assert_eq!(
        relay.step(upstream(PUBLISHER, 0, upstream_subscribe_ok(None))),
        []
    );
}

#[test]
fn only_so_many_subscribers_wait_for_one_answer() {
    // The expectation below is spelled out for a limit of 64.
    assert_eq!(crate::MAX_WAITING_SUBSCRIBERS, 64);
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    for number in 0..65 {
        let session = SessionKey(100 + number);
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
        let actions = subscribe_to(&mut relay, session, 0, &[b"xmoq"]);
        match number {
            0 => assert_eq!(actions, asked(PUBLISHER, 0, 1, &[b"xmoq"])),
            1..=63 => assert_eq!(actions, [], "subscriber {number}"),
            // The 65th: EXCESSIVE_LOAD (0x9).
            _ => assert_eq!(actions, refused(session, 0, error(0x9))),
        }
    }
}

#[test]
fn only_so_many_namespaces_are_kept_for_one_session() {
    // The expectation below is spelled out for a limit of 64.
    assert_eq!(crate::MAX_NAMESPACES_PER_SESSION, 64);
    let mut relay = relay();
    for stream in 0..64 {
        let actions = announce(&mut relay, PUBLISHER, stream, &[b"xmoq"]);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::SendRequest {
                    message: ControlMessage::RequestOk(_),
                    ..
                }]
            ),
            "{stream}: {actions:?}"
        );
    }
    // The 65th: EXCESSIVE_LOAD (0x9). Another session is not held back.
    assert_eq!(
        announce(&mut relay, PUBLISHER, 64, &[b"xmoq"]),
        refused(PUBLISHER, 64, error(0x9))
    );
    assert_eq!(announce(&mut relay, BOB, 0, &[b"xmoq"]).len(), 1);
    assert_eq!(relay.counts().namespaces, 65);
}

#[test]
fn publish_done_of_a_routed_track_reaches_the_subscribers() {
    // (the Stream Count the publisher gives, what ends the wait, what the
    //  relay does to its own request)
    let cases = [
        (
            "the counted stream ends",
            Event::DataStreamClosed {
                session: PUBLISHER,
                stream: StreamKey(7),
                end: StreamEnd::Fin,
            },
            // Every counted stream has ended here: the relay finishes its
            // direction of its SUBSCRIBE.
            vec![
                Action::FinishDataStream {
                    out_stream: OutStreamKey(0),
                },
                Action::FinishOwnRequest {
                    session: PUBLISHER,
                    request: OwnRequestKey(0),
                },
            ],
        ),
        (
            "the wait runs out",
            Event::DrainTimeout {
                session: PUBLISHER,
                track_alias: UPSTREAM_ALIAS,
                wait: 0,
            },
            vec![
                Action::StopDataStream {
                    session: PUBLISHER,
                    stream: StreamKey(7),
                    code: 0x1,
                },
                Action::CancelOwnRequest {
                    session: PUBLISHER,
                    request: OwnRequestKey(0),
                    code: 0x1,
                },
                Action::ResetDataStream {
                    out_stream: OutStreamKey(0),
                    code: 0x1,
                },
            ],
        ),
    ];
    for (name, ending, mut expected) in cases {
        let mut relay = relay_with_routed_track();
        data_stream_opened(&mut relay, 7);
        assert_eq!(
            relay.step(upstream(PUBLISHER, 0, publish_done(0x2, 1))),
            [Action::StartDrainTimer {
                session: PUBLISHER,
                track_alias: UPSTREAM_ALIAS,
                wait: 0,
            }],
            "{name}"
        );
        // The publisher closes its direction after PUBLISH_DONE (§10.11);
        // the track goes on draining.
        assert_eq!(
            relay.step(Event::OwnRequestClosed {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                how: OwnRequestEnd::Fin,
            }),
            [],
            "{name}"
        );
        // TRACK_ENDED (0x2), one stream.
        expected.extend(done_actions(ALICE, 0x2, 1));
        assert_eq!(relay.step(ending), expected, "{name}");
        assert_eq!(routed(&relay), (1, 0, 0, 0, 0), "{name}");
    }
}

#[test]
fn a_publisher_that_ends_the_relays_subscribe_without_publish_done_ends_the_track() {
    let mut relay = relay_with_routed_track();
    data_stream_opened(&mut relay, 7);
    let mut expected = vec![
        Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(7),
            code: 0x1,
        },
        Action::CancelOwnRequest {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            code: 0x1,
        },
        Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x1,
        },
    ];
    expected.extend(done_actions(ALICE, 0x2, 1));
    assert_eq!(
        relay.step(Event::OwnRequestClosed {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            how: OwnRequestEnd::Fin,
        }),
        expected
    );
    assert_eq!(routed(&relay), (1, 0, 0, 0, 0));
}

/// PUBLISHER announced `(xmoq)`, Alice subscribed to `(xmoq)/a`, and the
/// relay asked PUBLISHER for the track (its request 0). Before PUBLISHER
/// answered, Bob sent PUBLISH for the track: the track has Bob as its source,
/// Alice is subscribed, and what the relay asked of PUBLISHER is still asked.
fn relay_with_a_source_and_an_ask() -> Relay {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
    // Bob's PUBLISH is accepted, and Alice, who waited, is answered with what
    // Bob's PUBLISH brought.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: BOB,
            stream: StreamKey(0),
            first: publish(0, b"a"),
        }),
        [
            Action::SendRequest {
                session: BOB,
                stream: StreamKey(0),
                message: ControlMessage::RequestOk(RequestOk {
                    parameters: vec![Parameter::Forward(true)],
                    track_properties: vec![],
                }),
            },
            subscribe_ok(ALICE, None),
        ]
    );
    let counts = relay.counts();
    assert_eq!(
        (
            counts.tracks,
            counts.pending_tracks,
            counts.sources,
            counts.asks,
            counts.own_requests
        ),
        (1, 0, 1, 1, 1)
    );
    relay
}

/// A name, the events, what the last of them leads to, and `(sources, asks,
/// requests of the relay)` after it.
type AskCase = (&'static str, Vec<Event>, Vec<Action>, (usize, usize, usize));

/// What can become of what the relay asked of PUBLISHER while the track has
/// Bob as a source. Nothing of it is told to Alice, who has her
/// `SUBSCRIBE_OK`.
fn what_becomes_of_the_ask() -> Vec<AskCase> {
    let cancelled = Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x1,
    };
    let finished = Action::FinishOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
    };
    let publishes = Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: publish(2, b"a"),
    };
    let publish_ok = Action::SendRequest {
        session: PUBLISHER,
        stream: StreamKey(1),
        message: ControlMessage::RequestOk(RequestOk {
            parameters: vec![Parameter::Forward(true)],
            track_properties: vec![],
        }),
    };
    let timeout = Event::UpstreamTimeout {
        session: PUBLISHER,
        request: OwnRequestKey(0),
    };
    let duplicate = upstream(PUBLISHER, 0, ControlMessage::RequestError(error(0x19)));
    vec![
        (
            "the publisher accepts: a second source",
            vec![upstream(PUBLISHER, 0, upstream_subscribe_ok(None))],
            vec![answered()],
            (2, 0, 1),
        ),
        (
            "the publisher refuses",
            vec![upstream(
                PUBLISHER,
                0,
                ControlMessage::RequestError(error(0x10)),
            )],
            // The hub stops the timer of a request that is finished.
            vec![finished.clone()],
            (1, 0, 0),
        ),
        (
            "the publisher does not answer in time",
            vec![timeout.clone()],
            vec![cancelled.clone()],
            (1, 0, 0),
        ),
        (
            "the publisher sends PUBLISH: the SUBSCRIBE gives way",
            vec![publishes.clone()],
            vec![cancelled, publish_ok.clone()],
            (2, 0, 0),
        ),
        (
            "the publisher goes",
            vec![Event::SessionClosed { session: PUBLISHER }],
            vec![],
            (1, 0, 0),
        ),
        // DUPLICATE_SUBSCRIPTION (0x19): a PUBLISH of the publisher is on its
        // way, and the relay waits for it, for the time an answer gets.
        (
            "the publisher answers DUPLICATE_SUBSCRIPTION",
            vec![duplicate.clone()],
            vec![
                finished,
                Action::StartUpstreamTimer {
                    session: PUBLISHER,
                    request: OwnRequestKey(0),
                },
            ],
            (1, 1, 1),
        ),
        (
            "DUPLICATE_SUBSCRIPTION, and the PUBLISH comes",
            vec![duplicate.clone(), publishes],
            vec![answered(), publish_ok],
            (2, 0, 0),
        ),
        (
            "DUPLICATE_SUBSCRIPTION, and no PUBLISH comes in time",
            vec![duplicate, timeout],
            vec![],
            (1, 0, 0),
        ),
    ]
}

#[test]
fn what_was_asked_of_a_publisher_stays_asked_when_another_publishes_the_track() {
    for (name, events, expected, held) in what_becomes_of_the_ask() {
        let mut relay = relay_with_a_source_and_an_ask();
        let mut last = Vec::new();
        for event in events {
            last = relay.step(event);
        }
        assert_eq!(last, expected, "{name}");
        let counts = relay.counts();
        assert_eq!(
            (counts.sources, counts.asks, counts.own_requests),
            held,
            "{name}"
        );
        // Alice is subscribed throughout, to the one track.
        assert_eq!((counts.tracks, counts.subscriptions), (1, 1), "{name}");
    }
}

#[test]
fn a_timer_that_fires_late_for_what_was_asked_does_nothing() {
    let timeout = Event::UpstreamTimeout {
        session: PUBLISHER,
        request: OwnRequestKey(0),
    };
    // (what ended the wait for the answer before its timer was reported)
    let cases = [
        (
            "the publisher accepted",
            upstream(PUBLISHER, 0, upstream_subscribe_ok(None)),
        ),
        (
            "the publisher sent PUBLISH",
            Event::RequestOpened {
                session: PUBLISHER,
                stream: StreamKey(1),
                first: publish(2, b"a"),
            },
        ),
        ("the track ended", Event::SessionClosed { session: BOB }),
    ];
    for (name, event) in cases {
        let mut relay = relay_with_a_source_and_an_ask();
        relay.step(event);
        let before = relay.counts();
        assert_eq!(relay.step(timeout.clone()), [], "{name}");
        assert_eq!(relay.counts(), before, "{name}");
    }
}

#[test]
fn a_malformed_track_takes_what_was_asked_with_it() {
    let mut relay = relay_with_a_source_and_an_ask();
    relay.step(Event::DataStreamOpened {
        session: BOB,
        stream: StreamKey(7),
        header: header(UPSTREAM_ALIAS),
    });
    // What Bob sends breaks the object model. Bob and Alice are told so, with
    // MALFORMED_TRACK (0x12); what was asked of PUBLISHER is cancelled with
    // CANCELLED (0x1).
    let mut expected = vec![
        Action::StopDataStream {
            session: BOB,
            stream: StreamKey(7),
            code: 0x12,
        },
        Action::ResetRequest {
            session: BOB,
            stream: StreamKey(0),
            code: 0x12,
        },
        Action::CancelOwnRequest {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            code: 0x1,
        },
        Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x12,
        },
    ];
    expected.extend(done_actions(ALICE, 0x12, 1));
    assert_eq!(
        relay.step(Event::TrackMalformed {
            session: BOB,
            stream: StreamKey(7),
        }),
        expected
    );
    let counts = relay.counts();
    assert_eq!((counts.tracks, counts.asks, counts.own_requests), (0, 0, 0));
}

#[test]
fn an_answer_that_comes_when_the_track_has_all_its_sources_is_cancelled() {
    // The expectation below is spelled out for 4 sources.
    assert_eq!(crate::MAX_SOURCES, 4);
    let mut relay = relay_with_a_source_and_an_ask();
    // Three more publishers send PUBLISH for the track: it has four sources.
    for session in [CAROL, SessionKey(11), SessionKey(12)] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
        relay.step(Event::RequestOpened {
            session,
            stream: StreamKey(0),
            first: publish(0, b"a"),
        });
    }
    assert_eq!(relay.counts().sources, 4);
    // PUBLISHER sent a stream ahead of its SUBSCRIBE_OK, which comes now.
    // For PUBLISHER the subscription is Established; the relay cancels it,
    // and stops the stream, both with CANCELLED (0x1).
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(5),
        header: header(UPSTREAM_ALIAS),
    });
    assert_eq!(relay.counts().early_streams, 1);
    assert_eq!(
        relay.step(upstream(PUBLISHER, 0, upstream_subscribe_ok(None))),
        [
            answered(),
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code: 0x1,
            },
            Action::StopDataStream {
                session: PUBLISHER,
                stream: StreamKey(5),
                code: 0x1,
            },
        ]
    );
    let counts = relay.counts();
    assert_eq!(
        (
            counts.sources,
            counts.asks,
            counts.own_requests,
            counts.early_streams
        ),
        (4, 0, 0, 0)
    );
}

#[test]
fn what_was_asked_is_cancelled_when_the_last_subscriber_goes() {
    let mut relay = relay_with_a_source_and_an_ask();
    // Alice cancels. Nobody would get what PUBLISHER might answer with, so
    // the SUBSCRIBE to it is cancelled. The track stays: Bob sends unasked.
    assert_eq!(
        relay.step(Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        }),
        [
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(0),
            },
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code: 0x1,
            },
        ]
    );
    let counts = relay.counts();
    assert_eq!(
        (
            counts.tracks,
            counts.sources,
            counts.asks,
            counts.own_requests
        ),
        (1, 1, 0, 0)
    );
}

#[test]
fn what_was_asked_is_cancelled_when_the_track_ends_with_its_last_source() {
    let mut relay = relay_with_a_source_and_an_ask();
    // Bob goes before PUBLISHER answered. The track has no source and ends
    // for Alice; the relay does not wait for PUBLISHER on her behalf.
    let mut expected = vec![Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x1,
    }];
    expected.extend(done_actions(ALICE, 0x2, 0));
    assert_eq!(relay.step(Event::SessionClosed { session: BOB }), expected);
    assert_eq!(routed(&relay), (1, 0, 0, 0, 0));
}

#[test]
fn everyone_who_waited_is_answered_when_another_publisher_publishes_the_track() {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
    // Carol waits as well, with Forward State 0.
    relay.step(Event::RequestOpened {
        session: CAROL,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track_in(&[b"xmoq"]),
            parameters: vec![Parameter::Forward(false)],
        }),
    });
    let actions = relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: publish(0, b"a"),
    });
    assert_eq!(
        actions[1..],
        [subscribe_ok(ALICE, None), subscribe_ok(CAROL, None)]
    );
    // What Bob sends is opened for Alice only (§5.1, Forward State 0).
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: BOB,
            stream: StreamKey(7),
            header: header(UPSTREAM_ALIAS),
        }),
        [open_data_stream(ALICE, 0, header(0))]
    );
}

#[test]
fn a_subscriber_that_only_finishes_its_direction_stays_subscribed() {
    // (the kind of track)
    for routed_track in [false, true] {
        let mut relay = if routed_track {
            relay_with_routed_track()
        } else {
            let mut relay = relay_with_track();
            subscribe_to_track(&mut relay, ALICE);
            relay
        };
        // §5.1: a subscriber ends a subscription with STOP_SENDING. A FIN of
        // its own direction asks for nothing.
        assert_eq!(
            relay.step(Event::RequestClosed {
                session: ALICE,
                stream: StreamKey(0),
                how: RequestEnd::Fin,
            }),
            [],
            "routed {routed_track}"
        );
        assert_eq!(
            data_stream_opened(&mut relay, 7),
            [open_data_stream(ALICE, 0, header(0))],
            "routed {routed_track}"
        );
        assert_eq!(relay.counts().subscriptions, 1, "routed {routed_track}");

        // STOP_SENDING after the FIN still cancels.
        let cancelled = relay.step(Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        });
        assert_eq!(
            cancelled[..2],
            [
                Action::FinishRequest {
                    session: ALICE,
                    stream: StreamKey(0),
                },
                Action::ResetDataStream {
                    out_stream: OutStreamKey(0),
                    code: 0x1,
                },
            ],
            "routed {routed_track}"
        );
        assert_eq!(relay.counts().subscriptions, 0, "routed {routed_track}");
    }
}

#[test]
fn a_subscription_whose_stream_was_finished_goes_with_its_session() {
    let mut relay = relay_with_routed_track();
    let before = relay.counts().requests;
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: StreamKey(0),
        how: RequestEnd::Fin,
    });
    assert_eq!(relay.counts().requests, before);
    relay.step(Event::SessionClosed { session: ALICE });
    assert_eq!(routed(&relay), (1, 0, 0, 0, 0));
    assert_eq!(relay.counts().requests, before - 1);
}

#[test]
fn publish_done_ends_a_finished_subscribe_stream_without_a_wait() {
    let mut relay = relay_with_track();
    let before = relay.counts().requests;
    subscribe_to_track(&mut relay, ALICE);
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: StreamKey(0),
        how: RequestEnd::Fin,
    });
    // The publisher leaves. Alice's direction is over already, so the relay
    // has nothing to wait for after its PUBLISH_DONE and FIN: no timer.
    assert_eq!(
        relay.step(Event::SessionClosed { session: PUBLISHER }),
        done_actions(ALICE, 0x2, 0)[..2]
    );
    assert_eq!(relay.counts().requests, before - 1);
}

#[test]
fn a_publish_whose_stream_is_finished_without_publish_done_is_over() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // A publisher ends its PUBLISH with PUBLISH_DONE on that direction (§5.1);
    // once the direction is finished, none can come.
    let mut expected = vec![Action::FinishRequest {
        session: PUBLISHER,
        stream: StreamKey(0),
    }];
    expected.extend(done_actions(ALICE, 0x2, 0));
    assert_eq!(
        relay.step(Event::RequestClosed {
            session: PUBLISHER,
            stream: StreamKey(0),
            how: RequestEnd::Fin,
        }),
        expected
    );
}

#[test]
fn nothing_of_the_routing_is_left_when_everyone_is_gone() {
    let mut relay = relay_with_routed_track();
    subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]);
    announce(&mut relay, CAROL, 0, &[b"other"]);
    // Bob waits for a track of Carol's namespace.
    subscribe_to(&mut relay, BOB, 1, &[b"other"]);
    assert_eq!(routed(&relay), (2, 1, 2, 1, 2));
    for session in [PUBLISHER, ALICE, BOB, CAROL] {
        relay.step(Event::SessionClosed { session });
    }
    assert_eq!(routed(&relay), (0, 0, 0, 0, 0));
    assert_eq!(held(&relay), (0, 0, 0, 0, 0));
    let counts = relay.counts();
    assert_eq!(
        (
            counts.requests,
            counts.request_streams,
            counts.half_closed_streams,
            counts.early_streams
        ),
        (0, 0, 0, 0)
    );
}

#[test]
fn what_ends_on_the_relays_own_request_decides_what_becomes_of_the_track() {
    let cancelled = Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x1,
    };
    // (what the I/O side reports, whether the publisher had answered with
    //  SUBSCRIBE_OK, what the relay does)
    let mut ended_track = vec![cancelled.clone()];
    ended_track.extend(done_actions(ALICE, 0x2, 0));
    let mut refused_alice = vec![cancelled];
    // DOES_NOT_EXIST (0x10).
    refused_alice.extend(refused(ALICE, 0, error(0x10)));
    let cases = [
        // Before an answer, each of these means: no publisher has the track.
        (OwnRequestEnd::Unsent, false, refused_alice.clone()),
        (OwnRequestEnd::Fin, false, refused_alice.clone()),
        (OwnRequestEnd::Reset, false, refused_alice),
        // STOP_SENDING closes only the relay's direction; the answer can
        // still come on the other one.
        (OwnRequestEnd::Stopped, false, vec![]),
        // After SUBSCRIBE_OK and without PUBLISH_DONE, the publisher is gone
        // from the track: TRACK_ENDED (0x2) for the subscriber.
        (OwnRequestEnd::Fin, true, ended_track.clone()),
        (OwnRequestEnd::Reset, true, ended_track),
        (OwnRequestEnd::Stopped, true, vec![]),
    ];
    for (how, answered, expected) in cases {
        let mut relay = if answered {
            relay_with_routed_track()
        } else {
            let mut relay = relay();
            announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
            subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
            relay
        };
        assert_eq!(
            relay.step(Event::OwnRequestClosed {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                how,
            }),
            expected,
            "{how:?}, answered {answered}"
        );
    }
}

#[test]
fn a_waiting_subscriber_that_finished_its_direction_leaves_nothing_when_refused() {
    // (what ends the wait)
    let endings = [
        upstream(PUBLISHER, 0, request_error(0x10)),
        Event::UpstreamTimeout {
            session: PUBLISHER,
            request: OwnRequestKey(0),
        },
        Event::OwnRequestClosed {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            how: OwnRequestEnd::Fin,
        },
        Event::SessionClosed { session: PUBLISHER },
    ];
    for ending in endings {
        let mut relay = relay();
        announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
        let streams_before = relay.counts().request_streams;
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
        // Alice finishes her direction while she waits. No further report
        // about her stream will come.
        relay.step(Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Fin,
        });
        let publisher_left = ending == Event::SessionClosed { session: PUBLISHER };
        let what = format!("{ending:?}");
        relay.step(ending);
        // Nothing of Alice's request is left. A publisher that left took its
        // own entry with it.
        let expected = if publisher_left { 0 } else { streams_before };
        let counts = relay.counts();
        assert_eq!(
            (counts.request_streams, counts.half_closed_streams),
            (expected, 0),
            "{what}"
        );
    }
}

#[test]
fn only_so_many_requests_of_the_relay_are_open_on_one_publisher() {
    // The expectation below is spelled out for a limit of 64.
    assert_eq!(crate::MAX_OWN_REQUESTS_PER_SESSION, 64);
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    announce(&mut relay, CAROL, 0, &[b"other"]);
    // Alice asks for one track after the other of the publisher's namespace.
    for number in 0..65u64 {
        let name = format!("track-{number}");
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(number),
            first: subscribe(number * 2, name.as_bytes()),
        });
        match number {
            0..=63 => assert!(
                matches!(actions.first(), Some(Action::OpenRequest { .. })),
                "track {number}: {actions:?}"
            ),
            // The 65th: EXCESSIVE_LOAD (0x9), and nothing goes upstream.
            _ => assert_eq!(actions, refused(ALICE, 64, error(0x9))),
        }
    }
    assert_eq!(relay.counts().own_requests, 64);
    // Another publisher is not held back by this one.
    assert_eq!(
        subscribe_to(&mut relay, BOB, 0, &[b"other"]),
        asked(CAROL, 64, 1, &[b"other"])
    );
}

#[test]
fn the_relays_subscribe_gives_way_to_a_publish_of_the_publisher_it_asked() {
    // The order a publisher runs into when viewers wait for it: it announces
    // its namespace, the relay asks it for the track, and its PUBLISH for
    // that track is already on its way.
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    assert_eq!(
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]),
        asked(PUBLISHER, 0, 1, &[b"xmoq"])
    );
    assert_eq!(subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]), []);

    // §5.1: the relay's own SUBSCRIBE is over before PUBLISH_OK goes out, and
    // the PUBLISH is what the subscribers waited for.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: publish(2, b"a"),
        }),
        [
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code: 0x1,
            },
            Action::SendRequest {
                session: PUBLISHER,
                stream: StreamKey(1),
                message: ControlMessage::RequestOk(RequestOk {
                    parameters: vec![Parameter::Forward(true)],
                    track_properties: vec![],
                }),
            },
            subscribe_ok(ALICE, None),
            subscribe_ok(BOB, None),
        ]
    );
    // One namespace, nothing pending, no request of the relay, one track,
    // two subscriptions.
    assert_eq!(routed(&relay), (1, 0, 0, 1, 2));

    // What still comes for the SUBSCRIBE that gave way changes nothing: the
    // publisher's answer to it, either way, the end of its stream, and the
    // timer for its answer.
    let late = [
        upstream(PUBLISHER, 0, upstream_subscribe_ok(None)),
        upstream(PUBLISHER, 0, request_error(0x19)),
        Event::OwnRequestClosed {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            how: OwnRequestEnd::Reset,
        },
        Event::UpstreamTimeout {
            session: PUBLISHER,
            request: OwnRequestKey(0),
        },
    ];
    for event in late {
        let what = format!("{event:?}");
        assert_eq!(relay.step(event), [], "{what}");
    }
    assert_eq!(routed(&relay), (1, 0, 0, 1, 2));

    // The Objects of the PUBLISH reach both.
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [
            open_data_stream(ALICE, 0, header(0)),
            open_data_stream(BOB, 1, header(0)),
        ]
    );
    assert_eq!(
        object_arrived(&mut relay, 7, 0),
        [send_object(0, 0), send_object(1, 0)]
    );
}

#[test]
fn a_publish_for_a_track_that_already_came_through_the_relays_subscribe_is_a_duplicate() {
    // §5.1: "at most one acting as a subscriber". The relay's
    // SUBSCRIBE was accepted, so it does not give way any more:
    // DUPLICATE_SUBSCRIPTION (0x19).
    let mut relay = relay_with_routed_track();
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: publish(2, b"a"),
        }),
        refused(PUBLISHER, 1, error(0x19))
    );
    assert_eq!(routed(&relay), (1, 0, 1, 1, 1));
}

#[test]
fn a_reason_that_is_passed_on_is_cut_between_characters() {
    // (the publisher's Error Reason, what the subscriber gets). 128 bytes
    // are passed on at most (§1.4.4: a Reason Phrase is UTF-8).
    let mut cut_inside = vec![b'r'; 127];
    // U+3042 in three bytes, of which only the first would fit.
    cut_inside.extend_from_slice(&[0xe3, 0x81, 0x82]);
    let cases = [
        (cut_inside, vec![b'r'; 127]),
        // Exactly as long as may be passed on, and nothing at all.
        (vec![b'r'; 128], vec![b'r'; 128]),
        (vec![], vec![]),
        // Not UTF-8: nothing of it is passed on.
        (vec![0xff, 0xfe], vec![]),
    ];
    for (reason, passed_on) in cases {
        let mut relay = relay();
        announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
        let actions = relay.step(rejected(RequestError {
            error_code: 0x10,
            retry_interval: 0,
            error_reason: ReasonPhrase::new(reason).unwrap(),
            redirect: None,
        }));
        let expected = RequestError {
            error_code: 0x10,
            retry_interval: 0,
            error_reason: ReasonPhrase::new(passed_on).unwrap(),
            redirect: None,
        };
        assert_eq!(
            actions.get(1..),
            Some(refused(ALICE, 0, expected).as_slice())
        );
    }
}

#[test]
fn a_publishers_error_code_is_passed_on_or_replaced() {
    // (the publisher's Error Code, the subscriber's, whether the publisher's
    //  reason goes with it, the subscriber's Retry Interval). The publisher's
    // Retry Interval is 7 in every row. REDIRECT, which needs a Redirect, is
    // a row of the table of answers above.
    let cases: [(u64, u64, bool, u64); 17] = [
        // About the track at the publisher or its load, and the generic
        // error: as they are.
        (0x0, 0x0, true, 7),
        (0x2, 0x2, true, 7),
        (0x9, 0x9, true, 7),
        (0x10, 0x10, true, 7),
        (0x33, 0x33, true, 7),
        // The three about tokens: as they are, until authorization is decided.
        (0x1, 0x1, true, 7),
        (0x4, 0x4, true, 7),
        (0x5, 0x5, true, 7),
        // NOT_SUPPORTED and GOING_AWAY are true of the publisher and the
        // relay only: DOES_NOT_EXIST, to be retried when the publisher said.
        (0x3, 0x10, false, 7),
        (0x6, 0x10, false, 7),
        // No answer to this SUBSCRIBE: INTERNAL_ERROR, and no word on a
        // retry. INVALID_RANGE (the relay names no range), MALFORMED_TRACK
        // (defined for FETCH), UNINTERESTED, PREFIX_OVERLAP,
        // NAMESPACE_TOO_LARGE, INVALID_JOINING_REQUEST_ID, and a code draft 18
        // does not have.
        (0x11, 0x0, false, 0),
        (0x12, 0x0, false, 0),
        (0x20, 0x0, false, 0),
        (0x30, 0x0, false, 0),
        (0x31, 0x0, false, 0),
        (0x32, 0x0, false, 0),
        (0x7777, 0x0, false, 0),
    ];
    for (from_publisher, to_subscriber, with_reason, retry_interval) in cases {
        let mut relay = relay();
        announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
        let actions = relay.step(rejected(RequestError {
            error_code: from_publisher,
            retry_interval: 7,
            error_reason: ReasonPhrase::new(b"why".to_vec()).unwrap(),
            redirect: None,
        }));
        let reason: &[u8] = if with_reason { b"why" } else { b"" };
        let expected = RequestError {
            error_code: to_subscriber,
            retry_interval,
            error_reason: ReasonPhrase::new(reason.to_vec()).unwrap(),
            redirect: None,
        };
        assert_eq!(
            actions.get(1..),
            Some(refused(ALICE, 0, expected).as_slice()),
            "{from_publisher:#x}"
        );
    }
}

/// What the relay holds for routing, by name.
#[derive(Debug, PartialEq, Eq)]
struct Routed {
    namespaces: usize,
    pending_tracks: usize,
    own_requests: usize,
    tracks: usize,
    subscriptions: usize,
}

fn routed_by_name(relay: &Relay) -> Routed {
    let (namespaces, pending_tracks, own_requests, tracks, subscriptions) = routed(relay);
    Routed {
        namespaces,
        pending_tracks,
        own_requests,
        tracks,
        subscriptions,
    }
}

/// What can follow a publisher's `DUPLICATE_SUBSCRIPTION` while Alice and Bob
/// wait: what comes next, what the relay does, and what it holds then.
type AwaitedPublishCase = (&'static str, Vec<Event>, Vec<Action>, Routed);

fn awaited_publish_cases() -> [AwaitedPublishCase; 4] {
    let stop_waiting = Action::StopUpstreamTimer {
        session: PUBLISHER,
        request: OwnRequestKey(0),
    };
    let published = vec![
        stop_waiting.clone(),
        Action::SendRequest {
            session: PUBLISHER,
            stream: StreamKey(1),
            message: ControlMessage::RequestOk(RequestOk {
                parameters: vec![Parameter::Forward(true)],
                track_properties: vec![],
            }),
        },
        subscribe_ok(ALICE, None),
        subscribe_ok(BOB, None),
    ];
    // TIMEOUT (0x2) for both; the timer has run out and needs no stopping.
    let mut timed_out = refused(ALICE, 0, error(0x2)).to_vec();
    timed_out.extend(refused(BOB, 0, error(0x2)));
    // DOES_NOT_EXIST (0x10) for both.
    let mut publisher_gone = refused(ALICE, 0, error(0x10)).to_vec();
    publisher_gone.extend(refused(BOB, 0, error(0x10)));
    let nothing_left = Routed {
        namespaces: 1,
        pending_tracks: 0,
        own_requests: 0,
        tracks: 0,
        subscriptions: 0,
    };
    [
        (
            "the PUBLISH",
            vec![Event::RequestOpened {
                session: PUBLISHER,
                stream: StreamKey(1),
                first: publish(2, b"a"),
            }],
            published,
            Routed {
                namespaces: 1,
                pending_tracks: 0,
                own_requests: 0,
                tracks: 1,
                subscriptions: 2,
            },
        ),
        (
            "no PUBLISH in time",
            vec![Event::UpstreamTimeout {
                session: PUBLISHER,
                request: OwnRequestKey(0),
            }],
            timed_out,
            nothing_left,
        ),
        (
            "both subscribers leave",
            vec![
                Event::RequestClosed {
                    session: ALICE,
                    stream: StreamKey(0),
                    how: RequestEnd::Aborted,
                },
                Event::RequestClosed {
                    session: BOB,
                    stream: StreamKey(0),
                    how: RequestEnd::Aborted,
                },
            ],
            // Each gets the FIN of its stream; with the last one gone the
            // wait is over.
            vec![
                Action::FinishRequest {
                    session: ALICE,
                    stream: StreamKey(0),
                },
                Action::FinishRequest {
                    session: BOB,
                    stream: StreamKey(0),
                },
                stop_waiting,
            ],
            Routed {
                namespaces: 1,
                pending_tracks: 0,
                own_requests: 0,
                tracks: 0,
                subscriptions: 0,
            },
        ),
        (
            "the publisher leaves",
            vec![Event::SessionClosed { session: PUBLISHER }],
            publisher_gone,
            Routed {
                namespaces: 0,
                pending_tracks: 0,
                own_requests: 0,
                tracks: 0,
                subscriptions: 0,
            },
        ),
    ]
}

#[test]
fn a_publisher_that_answers_duplicate_subscription_is_waited_for() {
    // §5.1: a publisher answers the relay's SUBSCRIBE with
    // DUPLICATE_SUBSCRIPTION (0x19) when its own PUBLISH for the track is
    // under way, and also when it still holds another subscription of the
    // relay to the track, in which case no PUBLISH follows. The relay cannot
    // tell the two apart and waits. The cases are in `awaited_publish_cases`.
    for (name, next, expected, left) in awaited_publish_cases() {
        let mut relay = relay();
        announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
        // The relay's request is over on its stream, and the wait for the
        // PUBLISH is timed anew. Alice is not answered.
        assert_eq!(
            relay.step(rejected(error(0x19))),
            [
                Action::FinishOwnRequest {
                    session: PUBLISHER,
                    request: OwnRequestKey(0),
                },
                Action::StartUpstreamTimer {
                    session: PUBLISHER,
                    request: OwnRequestKey(0),
                },
            ],
            "{name}"
        );
        // Whatever else arrives for the request changes nothing: a second
        // message, which the I/O side may have read before the first was
        // dealt with, and the publisher's FIN after its answer.
        let ignored = [
            rejected(error(0x10)),
            upstream(PUBLISHER, 0, upstream_subscribe_ok(None)),
            Event::OwnRequestClosed {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                how: OwnRequestEnd::Fin,
            },
        ];
        for event in ignored {
            let what = format!("{event:?}");
            assert_eq!(relay.step(event), [], "{name}: {what}");
        }
        // Bob waits with Alice; the publisher is not asked a second time. The
        // request still counts as one of the relay's on that publisher.
        assert_eq!(subscribe_to(&mut relay, BOB, 0, &[b"xmoq"]), [], "{name}");
        assert_eq!(
            routed_by_name(&relay),
            Routed {
                namespaces: 1,
                pending_tracks: 1,
                own_requests: 1,
                tracks: 0,
                subscriptions: 0,
            },
            "{name}"
        );

        let actions: Vec<Action> = next
            .into_iter()
            .flat_map(|event| relay.step(event))
            .collect();
        assert_eq!(actions, expected, "{name}");
        assert_eq!(routed_by_name(&relay), left, "{name}");
    }
}

#[test]
fn a_request_that_waits_for_a_publish_counts_against_the_publishers_limit() {
    // The expectation below is spelled out for a limit of 64.
    assert_eq!(crate::MAX_OWN_REQUESTS_PER_SESSION, 64);
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    // The publisher answers every SUBSCRIBE of the relay with
    // DUPLICATE_SUBSCRIPTION (0x19) and never sends a PUBLISH.
    for number in 0..65u64 {
        let name = format!("track-{number}");
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(number),
            first: subscribe(number * 2, name.as_bytes()),
        });
        match number {
            0..=63 => {
                assert!(
                    matches!(actions.first(), Some(Action::OpenRequest { .. })),
                    "track {number}: {actions:?}"
                );
                relay.step(upstream(PUBLISHER, number, request_error(0x19)));
            }
            // The 65th: EXCESSIVE_LOAD (0x9).
            _ => assert_eq!(actions, refused(ALICE, 64, error(0x9))),
        }
    }
    assert_eq!(relay.counts().own_requests, 64);
    assert_eq!(relay.counts().pending_tracks, 64);
}

#[test]
fn a_crossing_publish_brings_its_largest_object_properties_and_early_streams() {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
    // A subgroup stream of the publisher overtakes its PUBLISH (§10.10 lets
    // Objects come before PUBLISH_OK): it is held, with its Object and FIN.
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [Action::StartEarlyTimer {
            session: PUBLISHER,
            stream: StreamKey(7),
        }]
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), []);
    assert_eq!(data_stream_closed(&mut relay, 7, StreamEnd::Fin), []);

    let actions = relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"a"),
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(UPSTREAM_LARGEST)],
            track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
        }),
    });
    assert_eq!(
        actions,
        [
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code: 0x1,
            },
            Action::SendRequest {
                session: PUBLISHER,
                stream: StreamKey(1),
                message: ControlMessage::RequestOk(RequestOk {
                    parameters: vec![Parameter::Forward(true)],
                    track_properties: vec![],
                }),
            },
            // §9.6, §10.2.11: the Track Properties and the LARGEST_OBJECT of
            // the PUBLISH, as for a subscriber that comes after it.
            subscribe_ok(ALICE, Some(UPSTREAM_LARGEST)),
            // The held stream follows the SUBSCRIBE_OK, whole.
            open_data_stream(ALICE, 0, header(0)),
            send_object(0, 0),
            Action::FinishDataStream {
                out_stream: OutStreamKey(0),
            },
        ]
    );
    // A stream under another alias, such as one the publisher would have
    // given the SUBSCRIBE that gave way, is nothing the relay knows: held for
    // a while like any other, and then stopped.
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(8),
            header: header(41),
        }),
        [Action::StartEarlyTimer {
            session: PUBLISHER,
            stream: StreamKey(8),
        }]
    );
}

#[test]
fn the_relays_subscribes_of_a_subscriber_that_goes_are_cancelled_in_the_order_of_the_track_names() {
    // Alice subscribes to five tracks of the publisher's namespace, in an
    // order that is not that of their names, and the publisher accepts each.
    // An order left to chance would come out as the one below once in 120
    // times.
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    for (number, name) in (0u64..).zip([b"d", b"a", b"e", b"c", b"b"]) {
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(number),
            first: subscribe(number * 2, name),
        });
        relay.step(upstream(
            PUBLISHER,
            number,
            ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 40 + number,
                parameters: vec![],
                track_properties: vec![],
            }),
        ));
    }
    assert_eq!(routed(&relay), (1, 0, 5, 5, 5));

    // With Alice gone nobody wants the tracks, and the relay cancels its own
    // SUBSCRIBEs: the one for "a" was its request 1, for "b" 4, for "c" 3,
    // for "d" 0 and for "e" 2.
    let actions = relay.step(Event::SessionClosed { session: ALICE });
    let cancelled: Vec<u64> = actions
        .iter()
        .filter_map(|action| match action {
            Action::CancelOwnRequest { request, .. } => Some(request.0),
            _ => None,
        })
        .collect();
    assert_eq!(cancelled, [1, 4, 3, 0, 2]);
}

#[test]
fn a_subscriber_that_finished_its_direction_while_it_waited_has_still_finished_it_when_subscribed()
{
    // (what ends the subscription, what Alice's stream gets, the request
    //  streams the relay still has an entry for: the publisher's
    //  PUBLISH_NAMESPACE, unless the publisher went). Alice finishes
    // her direction while the publisher has not answered. No report about
    // her stream follows, so whatever ends her subscription later has to
    // leave nothing of the stream behind.
    let done_and_gone = vec![
        Action::SendRequest {
            session: ALICE,
            stream: StreamKey(0),
            message: publish_done(0x2, 0),
        },
        // No timer for a stream Alice has finished already.
        Action::FinishRequest {
            session: ALICE,
            stream: StreamKey(0),
        },
    ];
    let cases = [
        (
            "the publisher's PUBLISH_DONE",
            upstream(PUBLISHER, 0, publish_done(0x2, 0)),
            {
                let mut actions = vec![Action::FinishOwnRequest {
                    session: PUBLISHER,
                    request: OwnRequestKey(0),
                }];
                actions.extend(done_and_gone.clone());
                actions
            },
            1,
        ),
        (
            "the publisher goes",
            Event::SessionClosed { session: PUBLISHER },
            done_and_gone,
            0,
        ),
    ];
    for (name, ending, expected, streams_left) in cases {
        let mut relay = relay();
        announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
        subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
        assert_eq!(
            relay.step(Event::RequestClosed {
                session: ALICE,
                stream: StreamKey(0),
                how: RequestEnd::Fin,
            }),
            [],
            "{name}"
        );
        assert_eq!(
            relay.step(upstream(PUBLISHER, 0, upstream_subscribe_ok(None))),
            [answered(), subscribe_ok(ALICE, None)],
            "{name}"
        );
        // The mark went with her from the wait to the subscription.
        assert_eq!(relay.counts().half_closed_streams, 1, "{name}");

        assert_eq!(relay.step(ending), expected, "{name}");
        let counts = relay.counts();
        assert_eq!(
            (counts.half_closed_streams, counts.request_streams),
            (0, streams_left),
            "{name}"
        );
    }
}
