//! FETCH through the relay.

use xmoq_moqt::message::FetchOk;
use xmoq_moqt::params::GroupOrder;

use super::*;
use crate::{FetchPiece, FetchPipeEnd};

/// An End of Unknown Range indicator with the Location {`group`, `object`}.
fn unknown_through(group: u64, object: u64) -> FetchPiece {
    FetchPiece::EndOfRange(xmoq_moqt::fetch::RangeEnd::Unknown, at(group, object))
}

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
/// The wait for Alice to end her direction of the request stream of a FETCH
/// that was served: 3 seconds.
const AWAIT_PEER: Action = Action::StartFetchEndTimer {
    session: ALICE,
    stream: FETCH_STREAM,
    wait_ms: 3_000,
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
/// The wait for the publisher's fetch stream after its `FETCH_OK`: 5 seconds
/// for a FETCH without `FILL_TIMEOUT`.
const AWAIT_STREAM: Action = Action::StartFetchStreamTimer {
    session: PUBLISHER,
    request: OWN,
    wait_ms: 5_000,
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
    send_order: FETCH_SEND_ORDER,
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
                    vec![FINISH_UPSTREAM, AWAIT_PEER],
                ),
            ],
        ),
        (
            "the answer, then the stream: the wait goes on for the stream",
            true,
            vec![
                (Ok, vec![AWAIT_STREAM, fetch_ok_to_alice()]),
                (Stream, vec![STOP_TIMER, PIPE]),
                (
                    PipeEnded(FetchPipeEnd::Finished),
                    vec![FINISH_UPSTREAM, AWAIT_PEER],
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
                    vec![STOP_TIMER, fetch_ok_to_alice(), FINISH_UPSTREAM, AWAIT_PEER],
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
                (Ok, vec![AWAIT_STREAM, fetch_ok_to_alice()]),
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
                (Ok, vec![AWAIT_STREAM, fetch_ok_to_alice()]),
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
                    vec![
                        AWAIT_STREAM,
                        fetch_ok_to_alice(),
                        request_ok(),
                        request_ok(),
                    ],
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

/// A `REQUEST_UPDATE` of Alice on the stream of her FETCH, with FORWARD,
/// which the update of a FETCH may not carry (§10.2.12).
fn update_with_forward(request_id: u64) -> Event {
    Event::RequestMessage {
        session: ALICE,
        stream: FETCH_STREAM,
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters: vec![Parameter::Forward(true)],
        }),
    }
}

/// What Alice still sends on the stream of a FETCH whose answer the relay
/// has ended: a `REQUEST_UPDATE` gets no answer, one with a parameter that
/// closes the session while the FETCH is served does not, and each uses up
/// its Request ID.
fn nothing_is_answered_any_more(ended: impl Fn() -> Relay, next: u64, name: &str) {
    // `next` is Alice's next Request ID.
    for request_id in [next, next + 2] {
        let mut relay = ended();
        assert_eq!(happen(&mut relay, &Happens::Update(next)), [], "{name}");
        assert_eq!(relay.step(update_with_forward(next + 2)), [], "{name}");
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(9),
            first: subscribe(request_id, b"b"),
        });
        // INVALID_REQUEST_ID (0x4).
        assert!(
            matches!(
                actions.as_slice(),
                [Action::CloseSession {
                    session: ALICE,
                    code: 0x4,
                    ..
                }]
            ),
            "{name}: {request_id}: {actions:?}"
        );
    }
}

#[test]
fn what_comes_after_the_relay_ended_its_direction_of_a_fetch_gets_no_answer() {
    use Happens::{Ok, PipeEnded, Stream};
    // While the FETCH is served, the same two: REQUEST_OK, and
    // PROTOCOL_VIOLATION (0x3).
    let mut relay = fetching();
    happen(&mut relay, &Stream);
    happen(&mut relay, &Ok);
    assert_eq!(happen(&mut relay, &Happens::Update(2)), [request_ok()]);
    let closed = relay.step(update_with_forward(4));
    assert!(
        matches!(
            closed.as_slice(),
            [
                Action::CloseSession {
                    session: ALICE,
                    code: 0x3,
                    ..
                },
                ..
            ]
        ),
        "{closed:?}"
    );

    // The relay finished its direction: where the stream ended after the
    // answer, and where the answer came after the stream had ended.
    let finished = FetchPipeEnd::Finished;
    let ways: [(&str, [Happens; 3]); 2] = [
        ("the stream ended last", [Stream, Ok, PipeEnded(finished)]),
        ("the answer came last", [Stream, PipeEnded(finished), Ok]),
    ];
    for (name, steps) in ways {
        let ended = || {
            let mut relay = fetching();
            for step in &steps {
                happen(&mut relay, step);
            }
            relay
        };
        nothing_is_answered_any_more(ended, 2, name);
    }

    // A fetch stream the relay wrote itself: finished, and reset where the
    // subscriber did not take it, TOO_FAR_BEHIND (0x5).
    for given_up in [None, Some(0x5)] {
        let ended = || {
            let mut relay = relay_with_a_subscription(true);
            relay.step(joining(true, 1, vec![Parameter::FillTimeout(0)]));
            relay.step(written(given_up));
            relay
        };
        // The SUBSCRIBE has the Request ID 0, the Joining FETCH 2.
        nothing_is_answered_any_more(ended, 4, &format!("written, {given_up:?}"));
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
    // The limit this test is about is the one for all subscribers; one
    // session takes all of it here.
    relay.set_share_of_one_subscriber(64, 16);
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
    // The limit this test is about is the one for all subscribers; one
    // session takes all of it here.
    relay.set_share_of_one_subscriber(64, 16);
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
fn one_subscribers_session_has_only_so_many_of_the_relays_fetches_on_a_publisher() {
    assert_eq!(crate::MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER, 4);
    // A Standalone FETCH of `session` on `stream` for Groups 0 and 1 of the
    // track "a", which the relay has not cached: "asked" where the relay
    // opens a FETCH on the publisher, or the code it refuses with.
    let fetches = |relay: &mut Relay, session: SessionKey, stream: u64| {
        let actions = relay.step(Event::RequestOpened {
            session,
            stream: StreamKey(stream),
            first: standalone(stream * 2, b"a", (at(0, 0), at(2, 0)), vec![]),
        });
        actions
            .iter()
            .find_map(|action| match action {
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
    };
    let mut relay = relay_with_track();
    // (who, the stream; what the relay does)
    let steps = [
        (ALICE, 0, "asked"),
        (ALICE, 1, "asked"),
        (ALICE, 2, "asked"),
        (ALICE, 3, "asked"),
        // Her fifth: EXCESSIVE_LOAD (0x9), to be retried after a second.
        (ALICE, 4, "0x9, retry 1001"),
        // Bob's are asked, though the publisher has four FETCH requests.
        (BOB, 0, "asked"),
        (BOB, 1, "asked"),
        (BOB, 2, "asked"),
        (BOB, 3, "asked"),
        (BOB, 4, "0x9, retry 1001"),
    ];
    for (session, stream, expected) in steps {
        assert_eq!(
            fetches(&mut relay, session, stream),
            expected,
            "{session:?} {stream}"
        );
    }
    assert_eq!(relay.counts().own_requests, 8);
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
    // The limit this test is about is the one for all subscribers; one
    // session takes all of it here.
    relay.set_share_of_one_subscriber(64, 16);
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
    relay.set_share_of_one_subscriber(64, 16);
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
    assert!(actions.starts_with(&expected), "{actions:?}");
    // The FETCH for the other track is still asked.
    assert_eq!(relay.counts().fetches, 1);
}

#[test]
fn a_joining_fetch_of_a_track_found_malformed_is_refused_as_that() {
    // Alice's Joining FETCH for the track "b" is asked of the publisher.
    let mut relay = relay_with_a_subscription(true);
    assert_eq!(
        relay.step(joining(true, 0, vec![])),
        asks_for(at(5, 0), at(5, 3))
    );
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(20),
        header: header(41),
    });
    let actions = relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(20),
    });
    // MALFORMED_TRACK (0x12), and not what a FETCH gets whose subscription
    // ended (0x7), though the subscription ends in the same step.
    let expected = [
        Action::CancelOwnRequest {
            session: PUBLISHER,
            request: OWN,
            code: 0x1,
        },
        send(FETCH_STREAM, request_error(0x12)),
        FINISH,
    ];
    assert!(actions.starts_with(&expected), "{actions:?}");
    assert_eq!(relay.counts().fetches, 0);
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

/// A Joining FETCH of Alice on [`FETCH_STREAM`], Request ID 2, that names
/// her SUBSCRIBE with Request ID 0.
fn joining(relative: bool, joining_start: u64, parameters: Vec<Parameter>) -> Event {
    let range = if relative {
        FetchRange::RelativeJoining {
            joining_request_id: 0,
            joining_start,
        }
    } else {
        FetchRange::AbsoluteJoining {
            joining_request_id: 0,
            joining_start,
        }
    };
    Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range,
            parameters,
        }),
    }
}

/// A relay in which the publisher offers the track "b" with the largest
/// Location {5, 2}, and Alice has subscribed to it on her stream 0 with the
/// Request ID 0 and this Forward State: her Joining Location is {5, 2}.
fn relay_with_a_subscription(forward: bool) -> Relay {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"b"),
            track_alias: 41,
            parameters: vec![Parameter::LargestObject(at(5, 2))],
            track_properties: vec![],
        }),
    });
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"b"),
            parameters: vec![Parameter::Forward(forward)],
        }),
    });
    assert_eq!(
        actions,
        [send(
            StreamKey(0),
            ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 0,
                parameters: vec![Parameter::LargestObject(at(5, 2))],
                track_properties: vec![],
            })
        )]
    );
    relay
}

/// The relay's FETCH to the publisher for a range of the track "b".
fn asks_for(start: Location, end: Location) -> Vec<Action> {
    vec![
        Action::OpenRequest {
            session: PUBLISHER,
            request: OWN,
            message: standalone(1, b"b", (start, end), vec![]),
        },
        START_TIMER,
    ]
}

#[test]
fn a_joining_fetch_becomes_a_range_counted_from_the_joining_location() {
    let refused = |error_code| vec![send(FETCH_STREAM, request_error(error_code)), FINISH];
    // The Joining Location is {5, 2}: the range ends with it, End Location
    // {5, 3} (§10.12.2.1).
    //
    // (Relative or Absolute; Joining Start; what the relay does)
    let cases = [
        // Relative: so many Groups before the one of the Joining Location.
        (true, 0, asks_for(at(5, 0), at(5, 3))),
        (true, 2, asks_for(at(3, 0), at(5, 3))),
        (true, 5, asks_for(at(0, 0), at(5, 3))),
        // More Groups back than there are: from Group 0.
        (true, 9, asks_for(at(0, 0), at(5, 3))),
        // Absolute: from that Group.
        (false, 1, asks_for(at(1, 0), at(5, 3))),
        (false, 5, asks_for(at(5, 0), at(5, 3))),
        // A Group after the Joining Location: INVALID_RANGE (0x11).
        (false, 6, refused(0x11)),
    ];
    for (relative, joining_start, expected) in cases {
        let mut relay = relay_with_a_subscription(true);
        assert_eq!(
            relay.step(joining(relative, joining_start, vec![])),
            expected,
            "relative: {relative}, Joining Start {joining_start}"
        );
    }
}

#[test]
fn a_joining_fetch_needs_a_subscription_that_forwards_and_has_a_joining_location() {
    // Without Objects when the SUBSCRIBE_OK was sent, the subscription has
    // no Joining Location.
    let without_objects = || {
        let mut relay = relay_with_track();
        assert_eq!(
            subscribe_to_track(&mut relay, ALICE),
            [subscribe_ok(ALICE, None)]
        );
        relay
    };
    // (the relay and Alice's subscription; the error code)
    let cases = [
        // Forward State 0 (§10.12.2): INVALID_RANGE (0x11).
        (relay_with_a_subscription(false), 0x11),
        (without_objects(), 0x11),
    ];
    for (mut relay, error_code) in cases {
        assert_eq!(
            relay.step(joining(true, 0, vec![])),
            [send(FETCH_STREAM, request_error(error_code)), FINISH]
        );
    }
}

#[test]
fn a_joining_fetch_that_waits_for_nothing_is_answered_up_to_the_joining_location() {
    let mut relay = relay_with_a_subscription(true);
    // FETCH_OK with End Location {5, 3}, and End of Unknown Range at {5, 2}.
    assert_eq!(
        relay.step(joining(true, 1, vec![Parameter::FillTimeout(0)])),
        [
            send(
                FETCH_STREAM,
                ControlMessage::FetchOk(FetchOk {
                    end_of_track: false,
                    end_location: at(5, 3),
                    parameters: vec![],
                    track_properties: vec![],
                })
            ),
            Action::WriteFetchStream {
                session: ALICE,
                stream: FETCH_STREAM,
                request_id: 2,
                pieces: vec![unknown_through(5, 2)],
                wait_ms: 10_000,
                send_order: FETCH_SEND_ORDER,
            },
        ]
    );
}

/// What happens, and what the relay does about it.
type Step = (Event, Vec<Action>);

/// A relay that answered a Joining FETCH of Alice for the track "b" itself
/// (`FILL_TIMEOUT` 0) and has its fetch stream to write.
fn relay_that_writes_a_fetch_stream() -> Relay {
    let mut relay = relay_with_a_subscription(true);
    let actions = relay.step(joining(true, 0, vec![Parameter::FillTimeout(0)]));
    assert!(
        matches!(actions.last(), Some(Action::WriteFetchStream { .. })),
        "{actions:?}"
    );
    assert_eq!(relay.counts().fetches, 1);
    relay
}

#[test]
fn a_fetch_the_relay_answered_itself_ends_when_its_stream_is_written_or_given_up() {
    let written = |given_up| Event::FetchStreamWritten {
        session: ALICE,
        stream: FETCH_STREAM,
        given_up,
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
        // Written to its end: the relay waits for the subscriber to end
        // its direction first.
        ("written", vec![(written(None), vec![AWAIT_PEER])]),
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
                (written(None), vec![FINISH]),
            ],
        ),
    ];
    for (name, steps) in cases {
        let mut relay = relay_that_writes_a_fetch_stream();
        for (event, expected) in steps {
            assert_eq!(relay.step(event), expected, "{name}");
        }
        assert_eq!(relay.counts().fetches, 0, "{name}");
        // What comes late for it does nothing.
        assert_eq!(relay.step(written(None)), [], "{name}");
    }
}

#[test]
fn the_request_stream_of_a_fetch_that_was_served_is_finished_after_the_subscribers() {
    let closes = |how| Event::RequestClosed {
        session: ALICE,
        stream: FETCH_STREAM,
        how,
    };
    let time_is_over = Event::FetchEndTimeout {
        session: ALICE,
        stream: FETCH_STREAM,
    };
    // The time Alice has to end her direction after the relay finished its
    // own (`Relay::on_request_timeout`).
    let late = Event::RequestTimeout {
        session: ALICE,
        stream: FETCH_STREAM,
    };
    let stop = Action::StopRequest {
        session: ALICE,
        stream: FETCH_STREAM,
        code: 0x1,
    };
    let update = || Event::RequestMessage {
        session: ALICE,
        stream: FETCH_STREAM,
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id: 4,
            parameters: vec![],
        }),
    };
    // (what happens after the fetch stream was written to its end, and
    //  what the relay does at each step)
    let cases: Vec<(&str, Vec<Step>)> = vec![
        // The subscriber ends its direction: the relay finishes its own,
        // and the time that ran for it ends nothing.
        (
            "the subscriber finishes",
            vec![
                (closes(RequestEnd::Fin), vec![FINISH]),
                (time_is_over.clone(), vec![]),
                (late.clone(), vec![]),
            ],
        ),
        (
            "the subscriber resets or stops",
            vec![
                (closes(RequestEnd::Aborted), vec![FINISH]),
                (time_is_over.clone(), vec![]),
            ],
        ),
        // It does not: the relay finishes its direction when the 3 seconds
        // are over, and stops the subscriber's when the time for that is
        // over as well.
        (
            "the subscriber does nothing",
            vec![
                (time_is_over.clone(), vec![FINISH]),
                (time_is_over.clone(), vec![]),
                (late, vec![stop]),
            ],
        ),
        // While the relay waits, a REQUEST_UPDATE gets no answer: the
        // answer of the FETCH has ended.
        (
            "an update while the relay waits",
            vec![(update(), vec![]), (closes(RequestEnd::Fin), vec![FINISH])],
        ),
    ];
    for (name, steps) in cases {
        let mut relay = relay_that_writes_a_fetch_stream();
        let written = Event::FetchStreamWritten {
            session: ALICE,
            stream: FETCH_STREAM,
            given_up: None,
        };
        assert_eq!(relay.step(written), [AWAIT_PEER], "{name}");
        // Not one of the session's FETCH requests any more, while the
        // relay waits.
        let counts = relay.counts();
        assert_eq!((counts.fetches, counts.fetch_ends), (0, 1), "{name}");
        for (event, expected) in steps {
            assert_eq!(relay.step(event), expected, "{name}");
        }
        assert_eq!(relay.counts().fetch_ends, 0, "{name}");
    }
    // A session that goes takes what waited with it.
    let mut relay = relay_that_writes_a_fetch_stream();
    relay.step(Event::FetchStreamWritten {
        session: ALICE,
        stream: FETCH_STREAM,
        given_up: None,
    });
    relay.step(Event::SessionClosed { session: ALICE });
    assert_eq!(relay.counts().fetch_ends, 0);
}

#[test]
fn a_track_found_malformed_resets_the_fetch_stream_the_relay_writes() {
    let mut relay = relay_that_writes_a_fetch_stream();
    // A second FETCH for the track that the relay answers itself, and one
    // for the track "a", which goes on.
    for (stream, name, request_id, range) in [
        (9, &b"b"[..], 4, (at(5, 0), at(5, 3))),
        (11, b"a", 6, (at(0, 0), at(1, 0))),
    ] {
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(stream),
            first: standalone(request_id, name, range, vec![Parameter::FillTimeout(0)]),
        });
    }
    assert_eq!(relay.counts().fetches, 2);
    // A subgroup stream of the track "b" breaks the object model.
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(20),
        header: header_of_group(41, 6),
    });
    let actions = relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(20),
    });
    // §2.4.2: "reset any fetch streams with Status Code MALFORMED_TRACK"
    // (0x12): both of the track, in the order of their request streams,
    // before the subscription the first FETCH joins is ended.
    let abort = |stream| Action::AbortFetchWrite {
        session: ALICE,
        stream,
        code: 0x12,
    };
    assert_eq!(
        actions[..2],
        [abort(FETCH_STREAM), abort(StreamKey(9))],
        "{actions:?}"
    );
    let aborts = actions
        .iter()
        .filter(|action| matches!(action, Action::AbortFetchWrite { .. }))
        .count();
    assert_eq!(aborts, 2, "{actions:?}");
    assert_eq!(
        relay.step(Event::FetchStreamWritten {
            session: ALICE,
            stream: FETCH_STREAM,
            given_up: Some(0x12),
        }),
        [reset(0x12)]
    );
    assert_eq!(relay.counts().fetches, 1);
}

#[test]
fn a_fetch_stream_the_relay_writes_goes_on_when_its_track_ends() {
    let mut relay = relay_that_writes_a_fetch_stream();
    // The publisher goes: what was handed over to be written is written.
    let actions = relay.step(Event::SessionClosed { session: PUBLISHER });
    let aborted = actions
        .iter()
        .any(|action| matches!(action, Action::AbortFetchWrite { .. }));
    assert!(!aborted, "{actions:?}");
    assert_eq!(relay.counts().fetches, 1);
}

/// A relay in which Bob announced the namespace, Alice has subscribed to the
/// track "c" in it with this Forward State, and the relay's SUBSCRIBE to Bob
/// (its request 0, Request ID 1) has no answer yet: Alice's SUBSCRIBE is
/// Pending.
fn relay_with_a_pending_subscription(forward: bool) -> Relay {
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
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"c"),
            parameters: vec![Parameter::Forward(forward)],
        }),
    });
    assert!(
        matches!(
            actions.as_slice(),
            [
                Action::OpenRequest {
                    session: BOB,
                    request: OwnRequestKey(0),
                    ..
                },
                _
            ]
        ),
        "{actions:?}"
    );
    relay
}

fn bob_answers(relay: &mut Relay, message: ControlMessage) -> Vec<Action> {
    relay.step(Event::OwnRequestMessage {
        session: BOB,
        request: OwnRequestKey(0),
        message,
    })
}

#[test]
fn a_joining_fetch_for_a_pending_subscribe_waits_for_its_answer() {
    let subscribe_ok_from_bob = ControlMessage::SubscribeOk(SubscribeOk {
        track_alias: UPSTREAM_ALIAS,
        parameters: vec![Parameter::LargestObject(at(7, 1))],
        track_properties: vec![],
    });
    let answer = |message| Event::OwnRequestMessage {
        session: BOB,
        request: OwnRequestKey(0),
        message,
    };
    // The FETCH is asked of Bob for {7, 0} to {7, 2}, with the relay's next
    // request (1, Request ID 3): Alice's SUBSCRIBE_OK had LARGEST_OBJECT
    // {7, 1}.
    let asked = vec![
        Action::OpenRequest {
            session: BOB,
            request: OwnRequestKey(1),
            message: standalone(3, b"c", (at(7, 0), at(7, 2)), vec![]),
        },
        Action::StartUpstreamTimer {
            session: BOB,
            request: OwnRequestKey(1),
        },
    ];
    // FETCH_OK with End Location {7, 2}, and End of Unknown Range at {7, 1}.
    let answered_at_once = vec![
        send(
            FETCH_STREAM,
            ControlMessage::FetchOk(FetchOk {
                end_of_track: false,
                end_location: at(7, 2),
                parameters: vec![],
                track_properties: vec![],
            }),
        ),
        Action::WriteFetchStream {
            session: ALICE,
            stream: FETCH_STREAM,
            request_id: 2,
            pieces: vec![unknown_through(7, 1)],
            wait_ms: 10_000,
            send_order: FETCH_SEND_ORDER,
        },
    ];
    let refused = |error_code| vec![send(FETCH_STREAM, request_error(error_code)), FINISH];
    // (the Forward State of the SUBSCRIBE; the parameters of the FETCH; what
    //  becomes of the SUBSCRIBE; the end of what the relay does; Alice's
    //  FETCH requests after it)
    let cases = [
        (
            true,
            vec![],
            answer(subscribe_ok_from_bob.clone()),
            asked,
            1,
        ),
        (
            true,
            vec![Parameter::FillTimeout(0)],
            answer(subscribe_ok_from_bob.clone()),
            answered_at_once,
            1,
        ),
        // Forward State 0: INVALID_RANGE (0x11), when the SUBSCRIBE is
        // accepted and not before.
        (
            false,
            vec![],
            answer(subscribe_ok_from_bob.clone()),
            refused(0x11),
            0,
        ),
        // The SUBSCRIBE is refused, or not answered in time:
        // INVALID_JOINING_REQUEST_ID (0x32).
        (true, vec![], answer(request_error(0x10)), refused(0x32), 0),
        (
            true,
            vec![],
            Event::UpstreamTimeout {
                session: BOB,
                request: OwnRequestKey(0),
            },
            refused(0x32),
            0,
        ),
    ];
    for (forward, parameters, becomes, expected, fetches) in cases {
        let mut relay = relay_with_a_pending_subscription(forward);
        assert_eq!(relay.step(joining(true, 0, parameters)), [], "{becomes:?}");
        assert_eq!(relay.counts().fetches, 1, "{becomes:?}");
        let actions = relay.step(becomes.clone());
        assert!(actions.ends_with(&expected), "{becomes:?}: {actions:?}");
        assert_eq!(relay.counts().fetches, fetches, "{becomes:?}");
    }
}

#[test]
fn a_joining_fetch_that_is_given_up_while_it_waits_is_not_asked_for() {
    let mut relay = relay_with_a_pending_subscription(true);
    relay.step(joining(true, 0, vec![]));
    // CANCELLED (0x1) on its request stream.
    assert_eq!(
        happen(&mut relay, &Happens::Cancelled),
        [Action::ResetRequest {
            session: ALICE,
            stream: FETCH_STREAM,
            code: 0x1,
        }]
    );
    assert_eq!(relay.counts().fetches, 0);
    let actions = bob_answers(
        &mut relay,
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(at(7, 1))],
            track_properties: vec![],
        }),
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, Action::OpenRequest { .. })),
        "{actions:?}"
    );
}

/// A relay in which the publisher offers the track "b" with the largest
/// Location {5, 2}, and Bob has announced the namespace.
fn relay_for_early_fetches() -> Relay {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"b"),
            track_alias: 41,
            parameters: vec![Parameter::LargestObject(at(5, 2))],
            track_properties: vec![],
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
    relay
}

/// A Relative Joining FETCH of Alice on [`FETCH_STREAM`], for the last Group.
fn joining_fetch(request_id: u64, joining_request_id: u64) -> Event {
    Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id,
            range: FetchRange::RelativeJoining {
                joining_request_id,
                joining_start: 0,
            },
            parameters: vec![],
        }),
    }
}

const HOLD: Action = Action::StartJoiningFetchTimer {
    session: ALICE,
    stream: FETCH_STREAM,
};

#[test]
fn a_joining_fetch_that_is_read_before_its_subscribe_waits_for_it() {
    let opens = |first| Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first,
    };
    let refused = vec![send(FETCH_STREAM, request_error(0x32)), FINISH];
    // Alice's FETCH with the Request ID 2 names the Request ID 0, which has
    // not come: it is held. Then the request with the ID 0 comes.
    //
    // (what it is; the end of what the relay does; Alice's FETCH requests
    //  after it)
    let cases = [
        // A SUBSCRIBE that is accepted at once: the FETCH is asked for,
        // counted from the LARGEST_OBJECT of its SUBSCRIBE_OK, {5, 2}.
        (opens(subscribe(0, b"b")), asks_for(at(5, 0), at(5, 3)), 1),
        // A SUBSCRIBE that the relay refuses at once, for a track nobody
        // has: INVALID_JOINING_REQUEST_ID (0x32).
        (
            opens(ControlMessage::Subscribe(Subscribe {
                request_id: 0,
                full_track_name: FullTrackName::new(
                    TrackNamespace::new(vec![b"other".to_vec()]).unwrap(),
                    b"x".to_vec(),
                )
                .unwrap(),
                parameters: vec![],
            })),
            refused.clone(),
            0,
        ),
        // Something that is no SUBSCRIBE: the same.
        (
            opens(ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 0,
                track_namespace: TrackNamespace::new(vec![b"alice".to_vec()]).unwrap(),
                parameters: vec![],
            })),
            refused.clone(),
            0,
        ),
        // Nothing, in the time a stream gets to say what it is: the same.
        (
            Event::JoiningFetchTimeout {
                session: ALICE,
                stream: FETCH_STREAM,
            },
            refused,
            0,
        ),
        // Alice gives the FETCH up: CANCELLED (0x1) on its request stream.
        (
            Event::RequestClosed {
                session: ALICE,
                stream: FETCH_STREAM,
                how: RequestEnd::Aborted,
            },
            vec![reset(0x1)],
            0,
        ),
        (Event::SessionClosed { session: ALICE }, vec![], 0),
    ];
    for (comes, expected, fetches) in cases {
        let mut relay = relay_for_early_fetches();
        assert_eq!(relay.step(joining_fetch(2, 0)), [HOLD], "{comes:?}");
        // Held, and one of the 16 Alice may have open.
        assert_eq!(relay.counts().fetches, 1, "{comes:?}");
        let actions = relay.step(comes.clone());
        // In this order, among what else the step brings (the relay asks
        // Bob for the track as well, §9.5).
        let mut rest = actions.iter();
        for action in &expected {
            assert!(
                rest.any(|done| done == action),
                "{comes:?}: {action:?} in {actions:?}"
            );
        }
        assert_eq!(relay.counts().fetches, fetches, "{comes:?}");
        // The time that runs out later finds nothing to do.
        let late = relay.step(Event::JoiningFetchTimeout {
            session: ALICE,
            stream: FETCH_STREAM,
        });
        assert_eq!(late, [], "{comes:?}");
    }
}

#[test]
fn a_joining_fetch_that_is_held_is_settled_by_its_own_session_alone() {
    // Alice's FETCH 2 names her Request ID 0. Bob, who used 0 for his
    // PUBLISH_NAMESPACE, sends the FETCH 4 that names 2, on a stream with
    // the number of Alice's.
    let mut relay = relay_for_early_fetches();
    assert_eq!(relay.step(joining_fetch(2, 0)), [HOLD]);
    let bobs_hold = Action::StartJoiningFetchTimer {
        session: BOB,
        stream: FETCH_STREAM,
    };
    let bobs = relay.step(Event::RequestOpened {
        session: BOB,
        stream: FETCH_STREAM,
        first: ControlMessage::Fetch(Fetch {
            request_id: 4,
            range: FetchRange::RelativeJoining {
                joining_request_id: 2,
                joining_start: 0,
            },
            parameters: vec![],
        }),
    });
    assert_eq!(bobs, [bobs_hold]);
    assert_eq!(relay.counts().fetches, 2);
    // Alice's time runs out: hers is refused, Bob's stays.
    assert_eq!(
        relay.step(Event::JoiningFetchTimeout {
            session: ALICE,
            stream: FETCH_STREAM,
        }),
        [send(FETCH_STREAM, request_error(0x32)), FINISH]
    );
    assert_eq!(relay.counts().fetches, 1);
    // Alice uses the Request ID that Bob's FETCH names, 2 being taken by
    // her FETCH, with the next one: nothing of Bob's is touched.
    let alices = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(11),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 4,
            track_namespace: TrackNamespace::new(vec![b"alice".to_vec()]).unwrap(),
            parameters: vec![],
        }),
    });
    assert!(
        alices.iter().all(
            |action| !matches!(action, Action::SendRequest { session, .. } if *session == BOB)
        ),
        "{alices:?}"
    );
    assert_eq!(relay.counts().fetches, 1);
    // Bob uses his: INVALID_JOINING_REQUEST_ID (0x32) for his FETCH.
    let bob_uses = relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(11),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 2,
            track_namespace: TrackNamespace::new(vec![b"bob".to_vec()]).unwrap(),
            parameters: vec![],
        }),
    });
    let refused = Action::SendRequest {
        session: BOB,
        stream: FETCH_STREAM,
        message: request_error(0x32),
    };
    assert!(bob_uses.contains(&refused), "{bob_uses:?}");
    assert_eq!(relay.counts().fetches, 0);
}

#[test]
fn a_request_id_used_on_the_stream_of_a_refused_request_settles_a_held_fetch_at_once() {
    let mut relay = relay_for_early_fetches();
    // Alice's SUBSCRIBE 0, for a track nobody has, is refused; her
    // direction of its stream stays open.
    let refused = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: FullTrackName::new(
                TrackNamespace::new(vec![b"other".to_vec()]).unwrap(),
                b"x".to_vec(),
            )
            .unwrap(),
            parameters: vec![],
        }),
    });
    assert!(
        refused.contains(&send(StreamKey(0), request_error(0x10))),
        "{refused:?}"
    );
    // Her FETCH 4 names the Request ID 2, which has not come: held.
    assert_eq!(relay.step(joining_fetch(4, 2)), [HOLD]);
    // The ID 2 comes as a REQUEST_UPDATE on the stream of the refused
    // SUBSCRIBE: INVALID_JOINING_REQUEST_ID (0x32), without waiting.
    let used = relay.step(Event::RequestMessage {
        session: ALICE,
        stream: StreamKey(0),
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id: 2,
            parameters: vec![],
        }),
    });
    assert_eq!(used, [send(FETCH_STREAM, request_error(0x32)), FINISH]);
    assert_eq!(relay.counts().fetches, 0);
}

#[test]
fn a_joining_fetch_that_was_read_first_waits_on_with_its_subscribe() {
    // The SUBSCRIBE comes and has to wait itself, for Bob's answer: the
    // FETCH waits with it, and the time for the SUBSCRIBE to come no longer
    // counts.
    let mut relay = relay_for_early_fetches();
    assert_eq!(relay.step(joining_fetch(2, 0)), [HOLD]);
    let asked = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: subscribe(0, b"c"),
    });
    assert!(
        matches!(
            asked.as_slice(),
            [Action::OpenRequest { session: BOB, .. }, _]
        ),
        "{asked:?}"
    );
    assert_eq!(
        relay.step(Event::JoiningFetchTimeout {
            session: ALICE,
            stream: FETCH_STREAM,
        }),
        []
    );
    assert_eq!(relay.counts().fetches, 1);
    // Bob accepts: Alice's SUBSCRIBE_OK has LARGEST_OBJECT {7, 1}, and the
    // FETCH is asked of Bob for {7, 0} to {7, 2}.
    let actions = bob_answers(
        &mut relay,
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(at(7, 1))],
            track_properties: vec![],
        }),
    );
    let fetch = Action::OpenRequest {
        session: BOB,
        request: OwnRequestKey(1),
        message: standalone(3, b"c", (at(7, 0), at(7, 2)), vec![]),
    };
    assert!(actions.contains(&fetch), "{actions:?}");
}

#[test]
fn a_joining_request_id_that_the_session_or_an_update_uses_up_refuses_the_fetch() {
    let refused = [send(FETCH_STREAM, request_error(0x32)), FINISH];
    // Alice subscribes to "b" with the Request ID 0, and her FETCH with the
    // ID 6 names the ID 2, which has not come.
    let held = || {
        let mut relay = relay_with_a_subscription(true);
        assert_eq!(relay.step(joining_fetch(6, 2)), [HOLD]);
        relay
    };
    // (what uses the Request ID 2)
    let cases = [
        // A second SUBSCRIBE for the track, which the session refuses by
        // itself: DUPLICATE_SUBSCRIPTION.
        Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(3),
            first: subscribe(2, b"b"),
        },
        // A REQUEST_UPDATE on the stream of her subscription.
        Event::RequestMessage {
            session: ALICE,
            stream: StreamKey(0),
            message: ControlMessage::RequestUpdate(RequestUpdate {
                request_id: 2,
                parameters: vec![],
            }),
        },
    ];
    for uses in cases {
        let mut relay = held();
        let actions = relay.step(uses.clone());
        assert!(actions.ends_with(&refused), "{uses:?}: {actions:?}");
        assert_eq!(relay.counts().fetches, 0, "{uses:?}");
    }
}

#[test]
fn a_joining_request_id_that_cannot_come_is_refused_at_once() {
    let refused = [send(FETCH_STREAM, request_error(0x32)), FINISH];
    // (the Request ID of the FETCH; the Joining Request ID)
    let cases = [
        // Not smaller than the FETCH's own.
        (2, 4),
        (2, 2),
        // Not one of a client's.
        (2, 1),
    ];
    for (request_id, joining_request_id) in cases {
        let mut relay = relay_for_early_fetches();
        assert_eq!(
            relay.step(joining_fetch(request_id, joining_request_id)),
            refused,
            "{request_id} {joining_request_id}"
        );
        assert_eq!(relay.counts().fetches, 0);
    }
    // Used, by a SUBSCRIBE that is over: Alice cancelled it.
    let mut relay = relay_with_a_subscription(true);
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: StreamKey(0),
        how: RequestEnd::Aborted,
    });
    assert_eq!(relay.step(joining_fetch(2, 0)), refused);
}

#[test]
fn a_joining_fetch_that_waits_counts_as_one_of_the_sessions_fetches() {
    let mut relay = relay_with_a_pending_subscription(true);
    for index in 0..16u64 {
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(index + 1),
            first: ControlMessage::Fetch(Fetch {
                request_id: (index + 1) * 2,
                range: FetchRange::RelativeJoining {
                    joining_request_id: 0,
                    joining_start: 0,
                },
                parameters: vec![],
            }),
        });
        assert_eq!(actions, [], "{index}");
    }
    assert_eq!(relay.counts().fetches, 16);
    // EXCESSIVE_LOAD (0x9) for the seventeenth.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(17),
            first: standalone(34, b"a", (at(0, 0), at(2, 0)), vec![]),
        }),
        [
            send(StreamKey(17), request_error(0x9)),
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(17),
            },
        ]
    );
}

/// What ends Alice's subscription to the track "b" (her request stream 0).
#[derive(Debug, Clone, Copy)]
enum Ending {
    /// Alice resets the SUBSCRIBE's stream.
    Cancelled,
    /// The publisher ends the track: `PUBLISH_DONE` on its PUBLISH.
    TrackEnded,
    /// A subgroup stream of the track that the relay cannot write to Alice
    /// any more: `TOO_FAR_BEHIND`.
    TooFarBehind,
    /// A `REQUEST_UPDATE` for the subscription, which the relay refuses and
    /// ends the subscription for: `UPDATE_FAILED`.
    UpdateRefused,
}

/// (the ending; whether Alice finished her direction of the SUBSCRIBE's
///  stream before, which ends nothing by itself)
///
/// WHY not after a FIN for the two that Alice's own stream brings about: a
/// stream that is reset was not finished, and one that is finished carries
/// no `REQUEST_UPDATE`.
const ENDINGS: [(Ending, bool); 6] = [
    (Ending::Cancelled, false),
    (Ending::TrackEnded, false),
    (Ending::TrackEnded, true),
    (Ending::TooFarBehind, false),
    (Ending::TooFarBehind, true),
    (Ending::UpdateRefused, false),
];

impl Ending {
    /// The events that bring it about.
    fn events(self, after_a_fin: bool) -> Vec<Event> {
        let alice_finishes = Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Fin,
        };
        let ending = match self {
            Self::Cancelled => vec![Event::RequestClosed {
                session: ALICE,
                stream: StreamKey(0),
                how: RequestEnd::Aborted,
            }],
            Self::TrackEnded => vec![Event::RequestMessage {
                session: PUBLISHER,
                stream: StreamKey(1),
                message: publish_done(0x2, 0),
            }],
            // The relay opens the stream to Alice as its out stream 0.
            Self::TooFarBehind => vec![
                Event::DataStreamOpened {
                    session: PUBLISHER,
                    stream: StreamKey(20),
                    header: header(41),
                },
                Event::SendQueueFull {
                    out_stream: OutStreamKey(0),
                },
            ],
            Self::UpdateRefused => vec![Event::RequestMessage {
                session: ALICE,
                stream: StreamKey(0),
                message: ControlMessage::RequestUpdate(RequestUpdate {
                    request_id: 4,
                    // One the relay does not apply.
                    parameters: vec![Parameter::Forward(false)],
                }),
            }],
        };
        after_a_fin
            .then_some(alice_finishes)
            .into_iter()
            .chain(ending)
            .collect()
    }
}

#[test]
fn a_joining_fetch_without_its_answer_ends_with_its_subscription() {
    let stream_opens = Event::FetchStreamOpened {
        session: PUBLISHER,
        stream: UPSTREAM_STREAM,
        request_id: 1,
    };
    // INVALID_JOINING_REQUEST_ID (0x32), and the relay's FETCH is cancelled.
    let refused = vec![
        CANCEL_UPSTREAM,
        send(FETCH_STREAM, request_error(0x32)),
        FINISH,
    ];
    // Where the publisher's fetch stream came ahead of its FETCH_OK and is
    // passed on, that is cut as well, with CANCELLED (0x1): without the
    // subscription the FETCH_OK cannot be made when it comes.
    let cut = vec![
        abort_pipe(0x1),
        CANCEL_UPSTREAM,
        send(FETCH_STREAM, request_error(0x32)),
        FINISH,
    ];
    // (whether the fetch stream has come; what the relay does about the
    //  FETCH)
    for (stream_came, expected) in [(false, refused), (true, cut)] {
        for (ending, after_a_fin) in ENDINGS {
            let name = format!("{ending:?}, after a FIN: {after_a_fin}, stream: {stream_came}");
            let mut relay = relay_with_a_subscription(true);
            relay.step(joining(true, 0, vec![]));
            if stream_came {
                relay.step(stream_opens.clone());
            }
            assert_eq!(relay.counts().subscriptions, 1, "{name}");
            let mut actions = Vec::new();
            for event in ending.events(after_a_fin) {
                actions.extend(relay.step(event));
            }
            let about_the_fetch: Vec<Action> = actions
                .iter()
                .filter(|action| expected.contains(action))
                .cloned()
                .collect();
            assert_eq!(about_the_fetch, expected, "{name}: {actions:?}");
            let counts = relay.counts();
            assert_eq!(
                (counts.subscriptions, counts.fetches, counts.own_requests),
                (0, 0, 0),
                "{name}"
            );
            // The publisher's answer to the FETCH that was cancelled does
            // nothing; Alice's session is not closed for it.
            let late = relay.step(Event::OwnRequestMessage {
                session: PUBLISHER,
                request: OWN,
                message: fetch_ok(at(5, 3)),
            });
            assert_eq!(late, [], "{name}");
        }
    }
}

#[test]
fn a_joining_fetch_that_has_its_answer_goes_on_when_its_subscription_ends() {
    for (ending, after_a_fin) in ENDINGS {
        let name = format!("{ending:?}, after a FIN: {after_a_fin}");
        let mut relay = relay_with_a_subscription(true);
        relay.step(joining(true, 0, vec![]));
        relay.step(Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OWN,
            message: fetch_ok(at(5, 3)),
        });
        relay.step(Event::FetchStreamOpened {
            session: PUBLISHER,
            stream: UPSTREAM_STREAM,
            request_id: 1,
        });
        let mut actions = Vec::new();
        for event in ending.events(after_a_fin) {
            actions.extend(relay.step(event));
        }
        // The subscription is over, and nothing is done about the FETCH or
        // its stream.
        let about_the_fetch = |action: &Action| match action {
            Action::AbortFetchPipe { .. } | Action::CancelOwnRequest { .. } => true,
            Action::SendRequest { stream, .. }
            | Action::FinishRequest { stream, .. }
            | Action::ResetRequest { stream, .. } => *stream == FETCH_STREAM,
            _ => false,
        };
        assert!(!actions.iter().any(about_the_fetch), "{name}: {actions:?}");
        let counts = relay.counts();
        assert_eq!((counts.subscriptions, counts.fetches), (0, 1), "{name}");
        // It comes to its end as any other.
        assert_eq!(
            relay.step(Event::FetchPipeEnded {
                session: ALICE,
                stream: FETCH_STREAM,
                how: FetchPipeEnd::Finished,
            }),
            [FINISH_UPSTREAM, AWAIT_PEER],
            "{name}"
        );
        assert_eq!(relay.counts().fetches, 0, "{name}");
    }
}

#[test]
fn a_standalone_fetch_goes_on_when_a_subscription_to_its_track_ends() {
    let mut relay = relay_with_a_subscription(true);
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: standalone(2, b"b", (at(0, 0), at(2, 0)), vec![]),
    });
    assert!(
        matches!(actions.as_slice(), [Action::OpenRequest { .. }, _]),
        "{actions:?}"
    );
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: StreamKey(0),
        how: RequestEnd::Aborted,
    });
    assert_eq!(relay.counts().fetches, 1);
}

#[test]
fn a_fetch_that_waits_for_nothing_is_answered_from_what_the_relay_knows() {
    let fill_timeout_0 = || vec![Parameter::FillTimeout(0)];
    let answered = |end: Location, marker: FetchPiece| {
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
                pieces: vec![marker],
                wait_ms: 10_000,
                send_order: FETCH_SEND_ORDER,
            },
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
            answered(at(5, 3), unknown_through(5, 2)),
        ),
        // The whole of Group 3: the last Location is {3, 2^64 - 1}, whose
        // Object ID is the nine bytes of §1.4.1: 0xff and eight more.
        (
            b"b",
            (at(0, 0), at(3, 0)),
            answered(at(3, 0), unknown_through(3, u64::MAX)),
        ),
        // Up to {4, 1}.
        (
            b"b",
            (at(1, 0), at(4, 2)),
            answered(at(4, 2), unknown_through(4, 1)),
        ),
        // The largest itself.
        (
            b"b",
            (at(5, 2), at(5, 3)),
            answered(at(5, 3), unknown_through(5, 2)),
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
        // The request stream is left to the subscriber when the stream is
        // written, and not before; where nothing was written, it was
        // finished with the answer.
        let expected = if writes { vec![AWAIT_PEER] } else { vec![] };
        assert_eq!(relay.step(written(None)), expected, "{name:?}");
        assert_eq!(relay.counts().fetches, 0, "{name:?}");
    }
}

/// The I/O side is done with the fetch stream the relay wrote for Alice.
fn written(given_up: Option<u32>) -> Event {
    Event::FetchStreamWritten {
        session: ALICE,
        stream: FETCH_STREAM,
        given_up,
    }
}

#[test]
fn a_fetch_stream_the_relay_could_not_write_resets_the_request_stream() {
    let mut relay = relay_with_a_subscription(true);
    let actions = relay.step(joining(true, 1, vec![Parameter::FillTimeout(0)]));
    assert!(
        matches!(actions.last(), Some(Action::WriteFetchStream { .. })),
        "{actions:?}"
    );
    // TOO_FAR_BEHIND (0x5), as the I/O side says for a subscriber that gave
    // it no stream to write on: FETCH_OK was sent, and no stream comes.
    assert_eq!(relay.step(written(Some(0x5))), [reset(0x5)]);
    assert_eq!(relay.counts().fetches, 0);
    // Said once.
    assert_eq!(relay.step(written(Some(0x5))), []);
}

#[test]
fn the_wait_for_the_stream_after_the_answer_is_the_fill_timeout() {
    // (FILL_TIMEOUT of the subscriber's FETCH; the wait in milliseconds)
    let cases = [
        (None, 5_000),
        // A second at least.
        (Some(1), 1_000),
        (Some(999), 1_000),
        (Some(1_001), 1_001),
        (Some(20_000), 20_000),
        // 30 seconds at most.
        (Some(60_000), 30_000),
    ];
    for (fill_timeout, wait_ms) in cases {
        let mut relay = relay_with_track();
        let parameters: Vec<Parameter> = fill_timeout
            .map(Parameter::FillTimeout)
            .into_iter()
            .collect();
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: FETCH_STREAM,
            first: standalone(0, b"a", (at(0, 0), at(2, 0)), parameters),
        });
        let actions = happen(&mut relay, &Happens::Ok);
        assert_eq!(
            actions.first(),
            Some(&Action::StartFetchStreamTimer {
                session: PUBLISHER,
                request: OWN,
                wait_ms,
            }),
            "{fill_timeout:?}"
        );
    }
}

#[test]
fn a_joining_fetch_that_waited_is_held_to_what_a_publisher_is_asked_for() {
    // Alice's Joining FETCH waits for her SUBSCRIBE, which Bob has not
    // answered. Meanwhile Bob is asked for 16 FETCH requests: 15 of Alice,
    // which with the one that waits are all she may have, and one of the
    // other session.
    let mut relay = relay_with_a_pending_subscription(true);
    // The limit this test is about is the one for all subscribers; one
    // session takes all of it here.
    relay.set_share_of_one_subscriber(64, 16);
    assert_eq!(relay.step(joining(true, 0, vec![])), []);
    let asked =
        |actions: &[Action]| matches!(actions, [Action::OpenRequest { session: BOB, .. }, _]);
    for index in 0..15u64 {
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(100 + index),
            first: standalone(4 + index * 2, b"c", (at(0, 0), at(2, 0)), vec![]),
        });
        assert!(asked(&actions), "{index}: {actions:?}");
    }
    let actions = relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(50),
        first: standalone(2, b"c", (at(0, 0), at(2, 0)), vec![]),
    });
    assert!(asked(&actions), "{actions:?}");
    // Bob accepts the SUBSCRIBE: the FETCH that waited would be the 17th,
    // and is refused with EXCESSIVE_LOAD (0x9).
    let actions = bob_answers(
        &mut relay,
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(at(7, 1))],
            track_properties: vec![],
        }),
    );
    let refused = [send(FETCH_STREAM, request_error(0x9)), FINISH];
    assert!(actions.ends_with(&refused), "{actions:?}");
    assert_eq!(relay.counts().fetches, 16);
}

#[test]
fn a_joining_fetch_that_waits_goes_with_its_subscribe_and_with_its_session() {
    let refused = [send(FETCH_STREAM, request_error(0x32)), FINISH];
    // (what happens; the end of what the relay does)
    let cases = [
        // Alice cancels the SUBSCRIBE that the publisher has not answered:
        // INVALID_JOINING_REQUEST_ID (0x32) for the FETCH that named it.
        (
            Event::RequestClosed {
                session: ALICE,
                stream: StreamKey(0),
                how: RequestEnd::Aborted,
            },
            refused.to_vec(),
        ),
        // Alice's session is gone: nobody is left to tell.
        (Event::SessionClosed { session: ALICE }, vec![]),
    ];
    for (happens, expected) in cases {
        let mut relay = relay_with_a_pending_subscription(true);
        assert_eq!(relay.step(joining(true, 0, vec![])), []);
        assert_eq!(relay.counts().fetches, 1);
        let actions = relay.step(happens.clone());
        // The FETCH hears of it where the slot of the SUBSCRIBE goes, before
        // what the relay does about the subscription itself.
        assert!(actions.starts_with(&expected), "{happens:?}: {actions:?}");
        assert!(
            !expected.is_empty() || !actions.contains(&FINISH),
            "{happens:?}: {actions:?}"
        );
        assert_eq!(relay.counts().fetches, 0, "{happens:?}");
    }
}

#[test]
fn the_updates_of_a_joining_fetch_that_waits_are_answered_after_its_fetch_ok() {
    let update = |request_id| Event::RequestMessage {
        session: ALICE,
        stream: FETCH_STREAM,
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters: vec![],
        }),
    };
    let subscribe_ok_from_bob = ControlMessage::SubscribeOk(SubscribeOk {
        track_alias: UPSTREAM_ALIAS,
        parameters: vec![Parameter::LargestObject(at(7, 1))],
        track_properties: vec![],
    });
    // Two updates while the FETCH waits for its SUBSCRIBE, and one while it
    // waits for the publisher: three REQUEST_OK after the FETCH_OK.
    let mut relay = relay_with_a_pending_subscription(true);
    assert_eq!(relay.step(joining(true, 0, vec![])), []);
    assert_eq!(relay.step(update(4)), []);
    assert_eq!(relay.step(update(6)), []);
    bob_answers(&mut relay, subscribe_ok_from_bob.clone());
    assert_eq!(relay.step(update(8)), []);
    let actions = relay.step(Event::OwnRequestMessage {
        session: BOB,
        request: OwnRequestKey(1),
        message: fetch_ok(at(7, 2)),
    });
    assert_eq!(
        actions,
        [
            Action::StartFetchStreamTimer {
                session: BOB,
                request: OwnRequestKey(1),
                wait_ms: 5_000,
            },
            send(FETCH_STREAM, fetch_ok(at(7, 2))),
            request_ok(),
            request_ok(),
            request_ok(),
        ]
    );

    // A FETCH that waits for its SUBSCRIBE takes 64 updates; the next one
    // costs it the FETCH, EXCESSIVE_LOAD (0x9), and none is answered.
    let mut relay = relay_with_a_pending_subscription(true);
    assert_eq!(relay.step(joining(true, 0, vec![])), []);
    for index in 0..64u64 {
        assert_eq!(relay.step(update(4 + index * 2)), [], "{index}");
    }
    assert_eq!(
        relay.step(update(132)),
        [send(FETCH_STREAM, request_error(0x9)), FINISH]
    );
    assert_eq!(relay.counts().fetches, 0);

    // The 64 are counted on when the FETCH is asked of the publisher: the
    // next update after that costs the FETCH as well, and the relay's FETCH
    // to Bob is cancelled.
    let mut relay = relay_with_a_pending_subscription(true);
    assert_eq!(relay.step(joining(true, 0, vec![])), []);
    for index in 0..64u64 {
        assert_eq!(relay.step(update(4 + index * 2)), [], "{index}");
    }
    bob_answers(&mut relay, subscribe_ok_from_bob.clone());
    assert_eq!(
        relay.step(update(132)),
        [
            Action::CancelOwnRequest {
                session: BOB,
                request: OwnRequestKey(1),
                code: 0x1,
            },
            send(FETCH_STREAM, request_error(0x9)),
            FINISH
        ]
    );
}

#[test]
fn a_joining_fetch_asks_the_first_source_for_a_range_counted_from_the_largest_of_all() {
    // Two sources of the track "b": the publisher, whose PUBLISH has the
    // largest Location {5, 2}, and Bob, whose has {9, 0}.
    let publish = |request_id, largest| {
        ControlMessage::Publish(Publish {
            request_id,
            full_track_name: track(b"b"),
            track_alias: 41,
            parameters: vec![Parameter::LargestObject(largest)],
            track_properties: vec![],
        })
    };
    let mut relay = relay_with_track();
    for (session, request_id, largest) in [(PUBLISHER, 2, at(5, 2)), (BOB, 0, at(9, 0))] {
        relay.step(Event::RequestOpened {
            session,
            stream: StreamKey(1),
            first: publish(request_id, largest),
        });
    }
    // Alice's Joining Location is the largest the relay knows of: {9, 0}.
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: subscribe(0, b"b"),
    });
    assert_eq!(
        actions,
        [send(
            StreamKey(0),
            ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 0,
                parameters: vec![Parameter::LargestObject(at(9, 0))],
                track_properties: vec![],
            })
        )]
    );
    // The FETCH for {9, 0} to {9, 1} goes to the publisher, which said that
    // it has nothing after {5, 2}. What it answers is what Alice gets
    // (known gaps).
    assert_eq!(
        relay.step(joining(true, 0, vec![])),
        asks_for(at(9, 0), at(9, 1))
    );
}

#[test]
fn a_joining_location_at_the_last_object_id_ends_the_range_with_the_whole_group() {
    // The track "b" with the largest Location {5, 2^64 - 1}: the Object after
    // it has no number, and the End Location {5, 0} asks for the whole Group
    // (§10.12.1), which ends with the same Object.
    let last = at(5, u64::MAX);
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"b"),
            track_alias: 41,
            parameters: vec![Parameter::LargestObject(last)],
            track_properties: vec![],
        }),
    });
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: subscribe(0, b"b"),
    });
    assert_eq!(
        relay.step(joining(true, 0, vec![])),
        asks_for(at(5, 0), at(5, 0))
    );
}

#[test]
fn a_fetch_that_is_refused_after_its_subscriber_finished_the_stream_leaves_nothing() {
    let mut relay = relay_with_track();
    let before = relay.counts();
    // One more than the 256 requests a session may have open in its session
    // state: Alice sends a FETCH, finishes
    // her direction of its request stream, and the publisher refuses the
    // relay's FETCH; the relay refuses hers.
    for index in 0..257u64 {
        let stream = StreamKey(index);
        let asked = relay.step(Event::RequestOpened {
            session: ALICE,
            stream,
            first: standalone(index * 2, b"a", (at(0, 0), at(2, 0)), vec![]),
        });
        assert!(
            matches!(asked.as_slice(), [Action::OpenRequest { .. }, _]),
            "{index}: {asked:?}"
        );
        relay.step(Event::RequestClosed {
            session: ALICE,
            stream,
            how: RequestEnd::Fin,
        });
        let refused = relay.step(Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OwnRequestKey(index),
            message: request_error(0x10),
        });
        assert!(
            refused.contains(&send(stream, request_error(0x10))),
            "{index}: {refused:?}"
        );
        relay.step(Event::OwnRequestClosed {
            session: PUBLISHER,
            request: OwnRequestKey(index),
            how: OwnRequestEnd::Fin,
        });
    }
    let after = relay.counts();
    assert_eq!(
        (
            after.requests,
            after.request_streams,
            after.fetches,
            after.own_requests
        ),
        (before.requests, before.request_streams, 0, 0)
    );
}

#[test]
fn a_fetch_that_is_served_after_its_subscriber_finished_the_stream_leaves_nothing() {
    let mut relay = relay_with_track();
    let before = relay.counts();
    // Alice sends the FETCH and finishes her direction of its request
    // stream; the FETCH goes on, is answered, and its stream is passed on to
    // the end.
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: FETCH_STREAM,
        first: standalone(0, b"a", (at(0, 0), at(2, 0)), vec![]),
    });
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: FETCH_STREAM,
        how: RequestEnd::Fin,
    });
    assert_eq!(relay.counts().fetches, 1);
    for happens in [
        Happens::Ok,
        Happens::Stream,
        Happens::PipeEnded(FetchPipeEnd::Finished),
    ] {
        happen(&mut relay, &happens);
    }
    let after = relay.counts();
    assert_eq!(
        (after.requests, after.request_streams, after.fetches),
        (before.requests, before.request_streams, 0)
    );
}

#[test]
fn a_fetch_the_relay_answers_itself_after_its_subscriber_finished_the_stream_leaves_nothing() {
    // A Joining FETCH with FILL_TIMEOUT 0 waits for Alice's SUBSCRIBE, and
    // Alice finishes her direction of the FETCH's request stream meanwhile.
    let mut relay = relay_with_a_pending_subscription(true);
    relay.step(joining(true, 0, vec![Parameter::FillTimeout(0)]));
    relay.step(Event::RequestClosed {
        session: ALICE,
        stream: FETCH_STREAM,
        how: RequestEnd::Fin,
    });
    let with_the_fetch = relay.counts();
    // Bob accepts the SUBSCRIBE; the relay answers the FETCH itself, and its
    // stream is written.
    let actions = bob_answers(
        &mut relay,
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(at(7, 1))],
            track_properties: vec![],
        }),
    );
    assert!(
        matches!(actions.last(), Some(Action::WriteFetchStream { .. })),
        "{actions:?}"
    );
    // The subscriber's direction is over: nothing is waited for.
    assert_eq!(relay.step(written(None)), [FINISH]);
    // The request and its stream are gone from the session; the SUBSCRIBE
    // is still there.
    let after = relay.counts();
    assert_eq!(
        (after.requests, after.request_streams, after.fetches),
        (
            with_the_fetch.requests - 1,
            with_the_fetch.request_streams - 1,
            0
        )
    );
}

#[test]
fn an_answer_to_a_subscribe_the_relay_cancelled_is_dropped() {
    // (what Bob answers to the SUBSCRIBE the relay has cancelled)
    let answers = [
        ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(at(7, 1))],
            track_properties: vec![],
        }),
        request_error(0x10),
    ];
    for answer in answers {
        // Alice's SUBSCRIBE waits for Bob's answer, and she gives it up: the
        // relay cancels what it asked of Bob.
        let mut relay = relay_with_a_pending_subscription(true);
        let actions = relay.step(Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        });
        let cancelled = Action::CancelOwnRequest {
            session: BOB,
            request: OwnRequestKey(0),
            code: 0x1,
        };
        assert!(actions.contains(&cancelled), "{actions:?}");
        // Bob sent his answer before he saw that: nothing follows from it,
        // and Bob's session is not closed for it.
        assert_eq!(bob_answers(&mut relay, answer.clone()), [], "{answer:?}");
        assert_eq!(relay.counts().sessions, 3, "{answer:?}");
    }
}
