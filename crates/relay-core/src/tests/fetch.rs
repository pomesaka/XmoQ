//! FETCH through the relay.

use xmoq_moqt::message::FetchOk;
use xmoq_moqt::params::GroupOrder;

use super::*;
use crate::FetchPipeEnd;

const OWN: OwnRequestKey = OwnRequestKey(0);
/// The request stream of Alice's FETCH.
const FETCH_STREAM: StreamKey = StreamKey(7);
/// The publisher's fetch stream.
const UPSTREAM_STREAM: StreamKey = StreamKey(30);

fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

fn standalone(
    request_id: u64,
    name: &[u8],
    (start, end): (Location, Location),
    parameters: Vec<Parameter>,
) -> ControlMessage {
    ControlMessage::Fetch(Fetch {
        request_id,
        range: FetchRange::Standalone {
            full_track_name: track(name),
            start,
            end,
        },
        parameters,
    })
}

fn fetch_ok(end_location: Location) -> ControlMessage {
    ControlMessage::FetchOk(FetchOk {
        end_of_track: false,
        end_location,
        parameters: vec![],
        track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
    })
}

fn send(stream: StreamKey, message: ControlMessage) -> Action {
    Action::SendRequest {
        session: ALICE,
        stream,
        message,
    }
}

const FINISH: Action = Action::FinishRequest {
    session: ALICE,
    stream: FETCH_STREAM,
};
const CANCEL_UPSTREAM: Action = Action::CancelOwnRequest {
    session: PUBLISHER,
    request: OWN,
    code: 0x1,
};
const STOP_TIMER: Action = Action::StopUpstreamTimer {
    session: PUBLISHER,
    request: OWN,
};
const START_TIMER: Action = Action::StartUpstreamTimer {
    session: PUBLISHER,
    request: OWN,
};
const PIPE: Action = Action::PipeFetch {
    from: PUBLISHER,
    from_stream: UPSTREAM_STREAM,
    session: ALICE,
    stream: FETCH_STREAM,
    request_id: 0,
    upstream_wait_ms: 5_000,
    downstream_wait_ms: 10_000,
};

fn abort_pipe(code: u32) -> Action {
    Action::AbortFetchPipe {
        session: ALICE,
        stream: FETCH_STREAM,
        code,
    }
}

/// A relay in which Alice has sent a FETCH for {0, 0} to {2, 0} of the track
/// "a", Request ID 0, and the relay has asked the publisher with its own,
/// Request ID 1.
fn fetching() -> Relay {
    let mut relay = relay_with_track();
    let range = (at(0, 0), at(2, 0));
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: FETCH_STREAM,
            first: standalone(0, b"a", range, vec![]),
        }),
        [
            Action::OpenRequest {
                session: PUBLISHER,
                request: OWN,
                message: standalone(1, b"a", range, vec![]),
            },
            START_TIMER,
        ]
    );
    relay
}

/// One thing that happens to the FETCH of [`fetching`].
#[derive(Debug, Clone)]
enum Happens {
    /// The publisher opens its fetch stream.
    Stream,
    /// The publisher answers `FETCH_OK` with the End Location {2, 0}.
    Ok,
    /// The publisher answers `REQUEST_ERROR` with this code.
    Refused(u64),
    /// The publisher does not answer in time.
    TimedOut,
    PipeEnded(FetchPipeEnd),
    /// Alice resets the request stream of her FETCH.
    Cancelled,
    /// Alice sends `REQUEST_UPDATE` with this Request ID.
    Update(u64),
    Closed(SessionKey),
}

fn happen(relay: &mut Relay, what: &Happens) -> Vec<Action> {
    relay.step(match what {
        Happens::Stream => Event::FetchStreamOpened {
            session: PUBLISHER,
            stream: UPSTREAM_STREAM,
            request_id: 1,
        },
        Happens::Ok => Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OWN,
            message: fetch_ok(at(2, 0)),
        },
        Happens::Refused(error_code) => Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OWN,
            message: request_error(*error_code),
        },
        Happens::TimedOut => Event::UpstreamTimeout {
            session: PUBLISHER,
            request: OWN,
        },
        Happens::PipeEnded(how) => Event::FetchPipeEnded {
            session: ALICE,
            stream: FETCH_STREAM,
            how: *how,
        },
        Happens::Cancelled => Event::RequestClosed {
            session: ALICE,
            stream: FETCH_STREAM,
            how: RequestEnd::Aborted,
        },
        Happens::Update(request_id) => Event::RequestMessage {
            session: ALICE,
            stream: FETCH_STREAM,
            message: ControlMessage::RequestUpdate(RequestUpdate {
                request_id: *request_id,
                parameters: vec![],
            }),
        },
        Happens::Closed(session) => Event::SessionClosed { session: *session },
    })
}

#[test]
fn a_fetch_is_asked_of_one_publisher_with_what_it_may_carry() {
    let mut relay = relay_with_track();
    let range = (at(4, 1), at(6, 0));
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: standalone(
            0,
            b"a",
            range,
            vec![
                Parameter::AuthorizationToken(vec![1]),
                Parameter::FillTimeout(700),
                Parameter::SubscriberPriority(9),
                Parameter::GroupOrder(GroupOrder::Descending),
            ],
        ),
    });
    // The Group Order and the FILL_TIMEOUT go with it; the priority and the
    // token do not.
    assert_eq!(
        actions,
        [
            Action::OpenRequest {
                session: PUBLISHER,
                request: OWN,
                message: standalone(
                    1,
                    b"a",
                    range,
                    vec![
                        Parameter::FillTimeout(700),
                        Parameter::GroupOrder(GroupOrder::Descending),
                    ],
                ),
            },
            START_TIMER,
        ]
    );
    // The FILL_TIMEOUT is how long the publisher may send nothing.
    let actions = happen(&mut relay, &Happens::Stream);
    assert!(
        matches!(
            actions.as_slice(),
            [
                _,
                Action::PipeFetch {
                    upstream_wait_ms: 700,
                    ..
                }
            ]
        ),
        "{actions:?}"
    );
}

#[test]
fn a_fill_timeout_is_not_waited_for_longer_than_the_relay_waits() {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: standalone(
            0,
            b"a",
            (at(0, 0), at(2, 0)),
            vec![Parameter::FillTimeout(3_600_000)],
        ),
    });
    let actions = happen(&mut relay, &Happens::Stream);
    assert!(
        matches!(
            actions.as_slice(),
            [
                _,
                Action::PipeFetch {
                    upstream_wait_ms: 30_000,
                    ..
                }
            ]
        ),
        "{actions:?}"
    );
}

#[test]
fn a_fetch_goes_to_a_publisher_of_the_namespace_when_the_track_has_no_source() {
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
    let range = (at(0, 0), at(2, 0));
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: standalone(0, b"unpublished", range, vec![]),
    });
    assert_eq!(
        actions,
        [
            Action::OpenRequest {
                session: BOB,
                request: OWN,
                message: standalone(1, b"unpublished", range, vec![]),
            },
            Action::StartUpstreamTimer {
                session: BOB,
                request: OWN,
            },
        ]
    );
}

/// What happens to the FETCH of [`fetching`], in order, and what the relay
/// does at each step; and whether the FETCH is over after the last step.
type Case = (&'static str, bool, Vec<(Happens, Vec<Action>)>);

fn fetch_ok_to_alice() -> Action {
    send(FETCH_STREAM, fetch_ok(at(2, 0)))
}

fn refuse(error_code: u64) -> Action {
    send(FETCH_STREAM, request_error(error_code))
}

fn request_ok() -> Action {
    send(
        FETCH_STREAM,
        ControlMessage::RequestOk(RequestOk::default()),
    )
}

const FINISH_UPSTREAM: Action = Action::FinishOwnRequest {
    session: PUBLISHER,
    request: OWN,
};

fn reset(code: u32) -> Action {
    Action::ResetRequest {
        session: ALICE,
        stream: FETCH_STREAM,
        code,
    }
}

fn fetches_that_come_to_their_end() -> Vec<Case> {
    use Happens::{Ok, PipeEnded, Stream, TimedOut};
    vec![
        (
            "the stream, the answer, the end of the stream",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (Ok, vec![STOP_TIMER, fetch_ok_to_alice()]),
                (
                    PipeEnded(FetchPipeEnd::Finished),
                    vec![FINISH_UPSTREAM, FINISH],
                ),
            ],
        ),
        (
            "the answer, then the stream: the wait goes on for the stream",
            true,
            vec![
                (Ok, vec![START_TIMER, fetch_ok_to_alice()]),
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    PipeEnded(FetchPipeEnd::Finished),
                    vec![FINISH_UPSTREAM, FINISH],
                ),
            ],
        ),
        (
            "the stream ends before the answer comes: the wait starts anew",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (PipeEnded(FetchPipeEnd::Finished), vec![START_TIMER]),
                (
                    Ok,
                    vec![STOP_TIMER, fetch_ok_to_alice(), FINISH_UPSTREAM, FINISH],
                ),
            ],
        ),
        (
            "the stream ends and no answer comes",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (PipeEnded(FetchPipeEnd::Finished), vec![START_TIMER]),
                // TIMEOUT (0x2).
                (TimedOut, vec![CANCEL_UPSTREAM, refuse(0x2), FINISH]),
            ],
        ),
        (
            "the answer comes and no stream: UNKNOWN_OBJECT_STATUS (0x6) on the request stream",
            true,
            vec![
                (Ok, vec![START_TIMER, fetch_ok_to_alice()]),
                (TimedOut, vec![CANCEL_UPSTREAM, reset(0x6)]),
            ],
        ),
        (
            "no answer and no stream",
            true,
            vec![(TimedOut, vec![CANCEL_UPSTREAM, refuse(0x2), FINISH])],
        ),
    ]
}

fn fetches_that_the_publisher_cuts_short() -> Vec<Case> {
    use Happens::{Ok, PipeEnded, Refused, Stream};
    vec![
        (
            "refused: INVALID_RANGE (0x11) is passed on",
            true,
            vec![(
                Refused(0x11),
                vec![STOP_TIMER, FINISH_UPSTREAM, refuse(0x11), FINISH],
            )],
        ),
        (
            "refused with NOT_SUPPORTED (0x3): DOES_NOT_EXIST (0x10)",
            true,
            vec![(
                Refused(0x3),
                vec![STOP_TIMER, FINISH_UPSTREAM, refuse(0x10), FINISH],
            )],
        ),
        (
            "refused while the stream is passed on: UNKNOWN_OBJECT_STATUS (0x6)",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    Refused(0x11),
                    vec![
                        STOP_TIMER,
                        FINISH_UPSTREAM,
                        abort_pipe(0x6),
                        refuse(0x11),
                        FINISH,
                    ],
                ),
            ],
        ),
        (
            "the publisher resets its stream before it answered",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                // INTERNAL_ERROR (0x0).
                (
                    PipeEnded(FetchPipeEnd::UpstreamAborted(Some(0x6))),
                    vec![CANCEL_UPSTREAM, refuse(0x0), FINISH],
                ),
            ],
        ),
        (
            "the publisher resets its stream after it answered",
            true,
            vec![
                (Ok, vec![START_TIMER, fetch_ok_to_alice()]),
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    PipeEnded(FetchPipeEnd::UpstreamAborted(Some(0x6))),
                    vec![CANCEL_UPSTREAM, FINISH],
                ),
            ],
        ),
        (
            "the publisher sends nothing for too long",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    PipeEnded(FetchPipeEnd::UpstreamStalled),
                    vec![CANCEL_UPSTREAM, refuse(0x2), FINISH],
                ),
            ],
        ),
    ]
}

fn fetches_that_the_subscriber_cuts_short() -> Vec<Case> {
    use Happens::{Cancelled, Closed, PipeEnded, Stream};
    vec![
        (
            "the subscriber stops the fetch stream: CANCELLED (0x1)",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    PipeEnded(FetchPipeEnd::DownstreamStopped(Some(0x1))),
                    vec![CANCEL_UPSTREAM, reset(0x1)],
                ),
            ],
        ),
        (
            "the subscriber takes nothing for too long: TOO_FAR_BEHIND (0x5)",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    PipeEnded(FetchPipeEnd::DownstreamStalled),
                    vec![CANCEL_UPSTREAM, reset(0x5)],
                ),
            ],
        ),
        (
            "the subscriber cancels before anything came",
            true,
            vec![(Cancelled, vec![CANCEL_UPSTREAM, reset(0x1)])],
        ),
        (
            "the subscriber cancels while the stream is passed on",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    Cancelled,
                    vec![abort_pipe(0x1), CANCEL_UPSTREAM, reset(0x1)],
                ),
            ],
        ),
        (
            "the subscriber's session goes: what was for it is dropped",
            true,
            vec![
                (Stream, vec![STOP_TIMER, PIPE]),
                (Closed(ALICE), vec![CANCEL_UPSTREAM]),
            ],
        ),
    ]
}

fn fetches_with_updates() -> Vec<Case> {
    use Happens::{Ok, Refused, Update};
    vec![
        (
            "updates before the answer are answered after it, the others at once",
            false,
            vec![
                (Update(2), vec![]),
                (Update(4), vec![]),
                (
                    Ok,
                    vec![START_TIMER, fetch_ok_to_alice(), request_ok(), request_ok()],
                ),
                (Update(6), vec![request_ok()]),
            ],
        ),
        (
            "updates before a refusal get no answer",
            true,
            vec![
                (Update(2), vec![]),
                (
                    Refused(0x11),
                    vec![STOP_TIMER, FINISH_UPSTREAM, refuse(0x11), FINISH],
                ),
                (Update(4), vec![]),
            ],
        ),
    ]
}

#[test]
fn what_happens_to_a_fetch_and_what_the_relay_does_about_it() {
    let cases = fetches_that_come_to_their_end()
        .into_iter()
        .chain(fetches_that_the_publisher_cuts_short())
        .chain(fetches_that_the_subscriber_cuts_short())
        .chain(fetches_with_updates());
    for (name, over, steps) in cases {
        let mut relay = fetching();
        let last = steps.len() - 1;
        for (index, (what, expected)) in steps.into_iter().enumerate() {
            assert_eq!(
                happen(&mut relay, &what),
                expected,
                "{name}, step {index}: {what:?}"
            );
            if index == last && over {
                let counts = relay.counts();
                assert_eq!((counts.fetches, counts.own_requests), (0, 0), "{name}");
            }
        }
    }
}

#[test]
fn a_publisher_that_goes_takes_its_fetches_with_it() {
    // Before it answered: DOES_NOT_EXIST (0x10), as for a SUBSCRIBE.
    let mut relay = fetching();
    let actions = happen(&mut relay, &Happens::Closed(PUBLISHER));
    assert_eq!(
        actions,
        [send(FETCH_STREAM, request_error(0x10)), FINISH],
        "before the answer"
    );
    assert_eq!(relay.counts().fetches, 0);

    // After the answer and before its stream: no stream will come, which
    // the reset of the request stream says, with UNKNOWN_OBJECT_STATUS (0x6).
    let mut relay = fetching();
    happen(&mut relay, &Happens::Ok);
    let actions = happen(&mut relay, &Happens::Closed(PUBLISHER));
    assert_eq!(
        actions,
        [Action::ResetRequest {
            session: ALICE,
            stream: FETCH_STREAM,
            code: 0x6,
        }],
        "after the answer"
    );
    assert_eq!(relay.counts().fetches, 0);

    // While its stream is passed on, after the answer: the stream is cut
    // with UNKNOWN_OBJECT_STATUS (0x6), and the request stream ends.
    let mut relay = fetching();
    happen(&mut relay, &Happens::Ok);
    happen(&mut relay, &Happens::Stream);
    let actions = happen(&mut relay, &Happens::Closed(PUBLISHER));
    assert_eq!(actions, [abort_pipe(0x6), FINISH], "while passing on");
    assert_eq!(relay.counts().fetches, 0);
}

#[test]
fn a_fetch_stream_for_a_fetch_the_relay_gave_up_is_stopped() {
    let mut relay = fetching();
    happen(&mut relay, &Happens::Cancelled);
    // The publisher opened it before it knew (§5.2).
    assert_eq!(
        happen(&mut relay, &Happens::Stream),
        [Action::StopDataStream {
            session: PUBLISHER,
            stream: UPSTREAM_STREAM,
            code: 0x1,
        }]
    );
    // What the pipe would report for it means nothing any more.
    assert_eq!(
        happen(&mut relay, &Happens::PipeEnded(FetchPipeEnd::Finished)),
        []
    );
}

#[test]
fn a_session_has_only_so_many_fetches_open() {
    assert_eq!(crate::fetch::MAX_FETCHES_PER_SESSION, 16);
    let mut relay = relay_with_track();
    for index in 0..16u64 {
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(index),
            first: standalone(index * 2, b"a", (at(0, 0), at(2, 0)), vec![]),
        });
        assert!(
            matches!(actions.as_slice(), [Action::OpenRequest { .. }, _]),
            "{index}: {actions:?}"
        );
    }
    // EXCESSIVE_LOAD (0x9) for the seventeenth.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(16),
            first: standalone(32, b"a", (at(0, 0), at(2, 0)), vec![]),
        }),
        [
            send(StreamKey(16), request_error(0x9)),
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(16),
            },
        ]
    );
}

#[test]
fn a_publisher_is_asked_for_only_so_many_fetches_at_once() {
    assert_eq!(crate::fetch::MAX_OWN_FETCHES_PER_SESSION, 16);
    let mut relay = relay_with_track();
    // Eight each from Alice and Bob, all for the one publisher.
    for (session, index) in [ALICE, BOB]
        .into_iter()
        .flat_map(|session| (0..8u64).map(move |index| (session, index)))
    {
        let actions = relay.step(Event::RequestOpened {
            session,
            stream: StreamKey(index),
            first: standalone(index * 2, b"a", (at(0, 0), at(2, 0)), vec![]),
        });
        assert!(
            matches!(actions.as_slice(), [Action::OpenRequest { .. }, _]),
            "{session:?} {index}: {actions:?}"
        );
    }
    // The seventeenth is Alice's ninth: EXCESSIVE_LOAD (0x9), though she may
    // have sixteen.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(8),
            first: standalone(16, b"a", (at(0, 0), at(2, 0)), vec![]),
        }),
        [
            send(StreamKey(8), request_error(0x9)),
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(8),
            },
        ]
    );
}

#[test]
fn the_relays_requests_to_a_publisher_are_counted_together() {
    assert_eq!(crate::MAX_OWN_REQUESTS_PER_SESSION, 64);
    let subscribe_to = |relay: &mut Relay, index: u64| {
        relay.step(Event::RequestOpened {
            session: BOB,
            stream: StreamKey(index),
            first: subscribe(index * 2, format!("t{index}").as_bytes()),
        })
    };
    let fetch = |relay: &mut Relay, stream: u64| {
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(stream),
            first: standalone(stream * 2, b"a", (at(0, 0), at(2, 0)), vec![]),
        })
    };
    let asked = |actions: &[Action]| matches!(actions, [Action::OpenRequest { .. }, _]);
    let announce = |relay: &mut Relay| {
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 2,
                track_namespace: namespace(),
                parameters: vec![],
            }),
        });
    };

    // 63 SUBSCRIBE requests of the relay to the publisher, for tracks of its
    // namespace that Bob asks for, and one FETCH: 64. One more of either is
    // not asked: EXCESSIVE_LOAD (0x9) for Alice's FETCH and for Bob's
    // SUBSCRIBE.
    let mut relay = relay_with_track();
    announce(&mut relay);
    for index in 0..63 {
        assert!(asked(&subscribe_to(&mut relay, index)), "SUBSCRIBE {index}");
    }
    assert!(asked(&fetch(&mut relay, 0)));
    assert_eq!(relay.counts().own_requests, 64);
    assert_eq!(
        fetch(&mut relay, 1),
        [
            send(StreamKey(1), request_error(0x9)),
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(1),
            },
        ]
    );
    let refused = subscribe_to(&mut relay, 63);
    assert!(
        matches!(
            refused.as_slice(),
            [
                Action::SendRequest {
                    session: BOB,
                    message: ControlMessage::RequestError(error),
                    ..
                },
                Action::FinishRequest { .. }
            ] if error.error_code == 0x9
        ),
        "{refused:?}"
    );

    // The other way round: 16 FETCH requests, which is all a publisher is
    // asked for, and 48 SUBSCRIBE requests. The next SUBSCRIBE is refused.
    let mut relay = relay_with_track();
    announce(&mut relay);
    for stream in 0..16 {
        assert!(asked(&fetch(&mut relay, stream)), "FETCH {stream}");
    }
    for index in 0..48 {
        assert!(asked(&subscribe_to(&mut relay, index)), "SUBSCRIBE {index}");
    }
    assert_eq!(relay.counts().own_requests, 64);
    let refused = subscribe_to(&mut relay, 48);
    assert!(
        matches!(
            refused.as_slice(),
            [
                Action::SendRequest {
                    message: ControlMessage::RequestError(error),
                    ..
                },
                _
            ] if error.error_code == 0x9
        ),
        "{refused:?}"
    );
}

#[test]
fn a_publishers_refusal_reaches_the_subscriber_with_its_retry_interval_and_reason() {
    let mut relay = fetching();
    let refusal = |error_code, retry_interval, reason: &[u8]| {
        ControlMessage::RequestError(RequestError {
            error_code,
            retry_interval,
            error_reason: ReasonPhrase::new(reason.to_vec()).unwrap(),
            redirect: None,
        })
    };
    // INVALID_RANGE (0x11) answers a FETCH and is passed on whole.
    let actions = relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OWN,
        message: refusal(0x11, 3000, b"nothing there yet"),
    });
    assert!(
        actions.contains(&send(
            FETCH_STREAM,
            refusal(0x11, 3000, b"nothing there yet")
        )),
        "{actions:?}"
    );
}

#[test]
fn a_track_found_malformed_takes_its_fetches_with_it() {
    // Alice subscribes to the track "a" and has two FETCH requests for it,
    // one answered with its stream passed on, one not answered; and one for
    // the track "b", which goes on.
    let mut relay = fetching();
    happen(&mut relay, &Happens::Ok);
    happen(&mut relay, &Happens::Stream);
    let subscribed = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(3),
        first: subscribe(6, b"a"),
    });
    assert!(
        matches!(
            subscribed.as_slice(),
            [Action::SendRequest {
                message: ControlMessage::SubscribeOk(_),
                ..
            }]
        ),
        "{subscribed:?}"
    );
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"b"),
            track_alias: 41,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    for (stream, name, request_id) in [(8, &b"a"[..], 2), (9, b"b", 4)] {
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(stream),
            first: standalone(request_id, name, (at(0, 0), at(2, 0)), vec![]),
        });
        assert!(
            matches!(actions.as_slice(), [Action::OpenRequest { .. }, _]),
            "{actions:?}"
        );
    }
    // A subgroup stream of the track "a" breaks the object model.
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(20),
        header: header(UPSTREAM_ALIAS),
    });
    let actions = relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(20),
    });
    // MALFORMED_TRACK (0x12): the stream that is passed on is reset with it,
    // and the FETCH without an answer is refused with it (§2.4.2).
    let expected = [
        abort_pipe(0x12),
        CANCEL_UPSTREAM,
        FINISH,
        Action::CancelOwnRequest {
            session: PUBLISHER,
            request: OwnRequestKey(1),
            code: 0x1,
        },
        send(StreamKey(8), request_error(0x12)),
        Action::FinishRequest {
            session: ALICE,
            stream: StreamKey(8),
        },
    ];
    assert!(actions.ends_with(&expected), "{actions:?}");
    // The FETCH for the other track is still asked.
    assert_eq!(relay.counts().fetches, 1);
}

#[test]
fn the_reset_of_a_publishers_fetch_stream_is_passed_on_where_it_says_something() {
    // (the publisher's Stream Reset Error Code; the subscriber's)
    let cases = [
        (Some(0x0), 0x0),
        (Some(0x1), 0x1),
        (Some(0x2), 0x2),
        (Some(0x6), 0x6),
        (Some(0x9), 0x9),
        (Some(0x12), 0x12),
        // SESSION_CLOSED and GOING_AWAY: the publisher is going, and what
        // came next is not known. UNKNOWN_OBJECT_STATUS (0x6).
        (Some(0x3), 0x6),
        (Some(0x4), 0x6),
        // TOO_FAR_BEHIND and EXPIRED_AUTH_TOKEN speak of the relay; a code
        // draft 18 does not have, and none: INTERNAL_ERROR (0x0).
        (Some(0x5), 0x0),
        (Some(0x7), 0x0),
        (Some(0x77), 0x0),
        (None, 0x0),
    ];
    for (from_the_publisher, to_the_subscriber) in cases {
        assert_eq!(
            crate::fetch_reset_passed_on(from_the_publisher),
            to_the_subscriber,
            "{from_the_publisher:?}"
        );
    }
}

#[test]
fn a_fetch_waits_for_only_so_many_updates() {
    assert_eq!(crate::fetch::MAX_HELD_FETCH_UPDATES, 64);
    let mut relay = fetching();
    for index in 1..=64u64 {
        assert_eq!(
            happen(&mut relay, &Happens::Update(index * 2)),
            [],
            "{index}"
        );
    }
    // The sixty-fifth: EXCESSIVE_LOAD (0x9), and none of them is answered.
    assert_eq!(
        happen(&mut relay, &Happens::Update(130)),
        [
            CANCEL_UPSTREAM,
            send(FETCH_STREAM, request_error(0x9)),
            FINISH
        ]
    );
    assert_eq!(relay.counts().fetches, 0);
}

#[test]
fn a_joining_fetch_is_not_served() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // NOT_SUPPORTED (0x3).
    assert_eq!(
        relay.step(Event::RequestOpened {
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
        }),
        [send(FETCH_STREAM, request_error(0x3)), FINISH]
    );
}

#[test]
fn a_fetch_that_waits_for_nothing_is_answered_from_what_the_relay_knows() {
    let fill_timeout_0 = || vec![Parameter::FillTimeout(0)];
    let answered = |end: Location, marker: Vec<u8>| {
        vec![
            send(
                FETCH_STREAM,
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: end,
                    parameters: vec![],
                    // §9.6: the Track Properties of the PUBLISH.
                    track_properties: vec![KeyValuePair::varint(4, 9).unwrap()],
                }),
            ),
            Action::WriteFetchStream {
                session: ALICE,
                stream: FETCH_STREAM,
                request_id: 0,
                body: marker,
                wait_ms: 10_000,
            },
            FINISH,
        ]
    };
    let refused = |error_code| vec![send(FETCH_STREAM, request_error(error_code)), FINISH];
    // The track "b" has the largest Location {5, 2} and a Track Property;
    // "a" has a source that has published nothing; Bob announced the
    // namespace, which "announced" is in and no source of.
    //
    // (the track; the range asked for; what the relay does). The body of the
    // stream is one End of Unknown Range indicator: 0x10C, which is the two
    // bytes 81 0c, and the last Location of the range.
    let cases = [
        // To the end of Group 9: cut at the largest, {5, 2}; End Location
        // {5, 3}.
        (
            &b"b"[..],
            (at(0, 0), at(9, 0)),
            answered(at(5, 3), vec![0x81, 0x0c, 0x05, 0x02]),
        ),
        // The whole of Group 3: the last Location is {3, 2^64 - 1}, whose
        // Object ID is the nine bytes of §1.4.1: 0xff and eight more.
        (
            b"b",
            (at(0, 0), at(3, 0)),
            answered(
                at(3, 0),
                vec![
                    0x81, 0x0c, 0x03, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                ],
            ),
        ),
        // Up to {4, 1}.
        (
            b"b",
            (at(1, 0), at(4, 2)),
            answered(at(4, 2), vec![0x81, 0x0c, 0x04, 0x01]),
        ),
        // The largest itself.
        (
            b"b",
            (at(5, 2), at(5, 3)),
            answered(at(5, 3), vec![0x81, 0x0c, 0x05, 0x02]),
        ),
        // Starts after the largest: INVALID_RANGE (0x11).
        (b"b", (at(5, 3), at(9, 0)), refused(0x11)),
        // A source that has published no Object: INVALID_RANGE (0x11).
        (b"a", (at(0, 0), at(9, 0)), refused(0x11)),
        // A publisher of the namespace, and no source: TIMEOUT (0x2).
        (b"announced", (at(0, 0), at(9, 0)), refused(0x2)),
    ];
    for (name, range, expected) in cases {
        let mut relay = relay_with_track();
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: ControlMessage::Publish(Publish {
                request_id: 2,
                full_track_name: track(b"b"),
                track_alias: 41,
                parameters: vec![Parameter::LargestObject(at(5, 2))],
                track_properties: vec![KeyValuePair::varint(4, 9).unwrap()],
            }),
        });
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
            stream: FETCH_STREAM,
            first: standalone(0, name, range, fill_timeout_0()),
        });
        let writes = actions
            .iter()
            .any(|action| matches!(action, Action::WriteFetchStream { .. }));
        assert_eq!(actions, expected, "{name:?} {range:?}");
        // Nothing was asked upstream. A stream that is being written counts
        // as one of Alice's FETCHes until the I/O side is done with it.
        let counts = relay.counts();
        assert_eq!(
            (counts.fetches, counts.own_requests),
            (usize::from(writes), 0),
            "{name:?}"
        );
        relay.step(Event::FetchStreamWritten {
            session: ALICE,
            stream: FETCH_STREAM,
        });
        assert_eq!(relay.counts().fetches, 0, "{name:?}");
    }
}
