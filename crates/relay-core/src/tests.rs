use std::sync::Arc;

use xmoq_moqt::data::{ObjectStatus, SubgroupHeader, SubgroupId, SubgroupObject};
use xmoq_moqt::kvp::KeyValuePair;
use xmoq_moqt::message::{
    ControlMessage, Publish, PublishDone, PublishNamespace, ReasonPhrase, RequestError, RequestOk,
    RequestUpdate, Setup, Subscribe, SubscribeOk,
};
use xmoq_moqt::name::{FullTrackName, Location, TrackNamespace};
use xmoq_moqt::params::Parameter;

mod routing;
mod subgroups;

use super::{
    Action, Event, OutStreamKey, OwnRequestEnd, OwnRequestKey, Relay, RequestEnd, Role, SessionKey,
    StreamEnd, StreamKey,
};

const PUBLISHER: SessionKey = SessionKey(1);
const ALICE: SessionKey = SessionKey(2);
const BOB: SessionKey = SessionKey(3);
/// The alias the publisher chose for the track.
const UPSTREAM_ALIAS: u64 = 40;

impl Relay {
    /// `handle`, with the I/O side reporting every stream it was asked to open
    /// as opened at once.
    fn step(&mut self, event: Event) -> Vec<Action> {
        let actions = self.handle(event);
        for action in &actions {
            if let Action::OpenDataStream { out_stream, .. } = action {
                let reported = self.handle(Event::OutStreamOpened {
                    out_stream: *out_stream,
                });
                assert_eq!(reported, []);
            }
        }
        actions
    }
}

/// `(sessions, tracks, subscriptions, incoming streams, outgoing streams)`.
fn held(relay: &Relay) -> (usize, usize, usize, usize, usize) {
    let counts = relay.counts();
    (
        counts.sessions,
        counts.tracks,
        counts.subscriptions,
        counts.in_streams,
        counts.out_streams,
    )
}

fn namespace() -> TrackNamespace {
    TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap()
}

fn track(name: &[u8]) -> FullTrackName {
    FullTrackName::new(namespace(), name.to_vec()).unwrap()
}

fn publish(request_id: u64, name: &[u8]) -> ControlMessage {
    ControlMessage::Publish(Publish {
        request_id,
        full_track_name: track(name),
        track_alias: UPSTREAM_ALIAS,
        parameters: vec![],
        track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
    })
}

fn subscribe(request_id: u64, name: &[u8]) -> ControlMessage {
    ControlMessage::Subscribe(Subscribe {
        request_id,
        full_track_name: track(name),
        parameters: vec![],
    })
}

fn request_error(error_code: u64) -> ControlMessage {
    ControlMessage::RequestError(RequestError {
        error_code,
        retry_interval: 0,
        error_reason: ReasonPhrase::default(),
        redirect: None,
    })
}

fn publish_done(status_code: u64, stream_count: u64) -> ControlMessage {
    ControlMessage::PublishDone(PublishDone {
        status_code,
        stream_count,
        error_reason: ReasonPhrase::default(),
    })
}

/// The header of a stream of Subgroup 0 of Group 3. The Subgroup ID is in
/// the header's type, so the relay knows the Subgroup when the stream opens.
fn header(track_alias: u64) -> SubgroupHeader {
    SubgroupHeader {
        track_alias,
        group_id: 3,
        subgroup_id: SubgroupId::Zero,
        publisher_priority: Some(9),
        properties: false,
        end_of_group: true,
        first_object: true,
    }
}

fn object(object_id: u64) -> SubgroupObject {
    SubgroupObject {
        object_id,
        properties: vec![],
        status: ObjectStatus::Normal,
        payload: vec![0x61],
    }
}

fn send_object(out_stream: u64, object_id: u64) -> Action {
    Action::SendObject {
        out_stream: OutStreamKey(out_stream),
        object: Arc::new(object(object_id)),
    }
}

fn open_data_stream(session: SessionKey, out_stream: u64, header: SubgroupHeader) -> Action {
    Action::OpenDataStream {
        session,
        out_stream: OutStreamKey(out_stream),
        header,
    }
}

/// A relay with one publisher that offers the track "a" on its stream 0.
fn relay_with_track() -> Relay {
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE, BOB] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    let actions = relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: publish(0, b"a"),
    });
    assert_eq!(
        actions,
        [Action::SendRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: ControlMessage::RequestOk(RequestOk {
                parameters: vec![Parameter::Forward(true)],
                track_properties: vec![],
            }),
        }]
    );
    relay
}

/// Subscribes `session` to the track "a" on its stream 0 and returns the
/// actions.
fn subscribe_to_track(relay: &mut Relay, session: SessionKey) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session,
        stream: StreamKey(0),
        first: subscribe(0, b"a"),
    })
}

fn subscribe_ok(session: SessionKey, largest: Option<Location>) -> Action {
    Action::SendRequest {
        session,
        stream: StreamKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 0,
            parameters: largest.map(Parameter::LargestObject).into_iter().collect(),
            // §9.6: the Track Properties of the PUBLISH are passed on.
            track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
        }),
    }
}

/// The same for another Group.
fn header_of_group(track_alias: u64, group_id: u64) -> SubgroupHeader {
    SubgroupHeader {
        group_id,
        ..header(track_alias)
    }
}

/// The Group of the stream `data_stream_opened` opens under a stream number.
/// Stream 7, the one most tests use, is of Group 3, the Group of `header`;
/// any other stream is of the Group with its own number, so that two streams
/// of a test are never of the same Subgroup unless the test says so.
fn group_of_stream(stream: u64) -> u64 {
    if stream == 7 { 3 } else { stream }
}

/// The publisher opens a subgroup stream of Subgroup 0 of
/// [`group_of_stream`].
fn data_stream_opened(relay: &mut Relay, stream: u64) -> Vec<Action> {
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(stream),
        header: header_of_group(UPSTREAM_ALIAS, group_of_stream(stream)),
    })
}

fn object_arrived(relay: &mut Relay, stream: u64, object_id: u64) -> Vec<Action> {
    relay.step(Event::Object {
        session: PUBLISHER,
        stream: StreamKey(stream),
        object: object(object_id),
    })
}

fn data_stream_closed(relay: &mut Relay, stream: u64, end: StreamEnd) -> Vec<Action> {
    relay.step(Event::DataStreamClosed {
        session: PUBLISHER,
        stream: StreamKey(stream),
        end,
    })
}

/// What `PUBLISH_DONE` and the FIN after it look like for a subscriber.
fn done_actions(session: SessionKey, status_code: u64, stream_count: u64) -> [Action; 3] {
    [
        Action::SendRequest {
            session,
            stream: StreamKey(0),
            message: publish_done(status_code, stream_count),
        },
        Action::FinishRequest {
            session,
            stream: StreamKey(0),
        },
        Action::StartRequestTimer {
            session,
            stream: StreamKey(0),
        },
    ]
}

#[test]
fn a_session_starts_with_setup() {
    let mut relay = Relay::new("test/0");
    assert_eq!(
        relay.step(Event::SessionOpened {
            session: ALICE,
            role: Role::Server,
        }),
        [Action::SendControl {
            session: ALICE,
            message: ControlMessage::Setup(Setup {
                setup_options: vec![KeyValuePair::bytes(0x07, b"test/0".to_vec()).unwrap()],
            }),
        }]
    );
    assert_eq!(
        relay.step(Event::ControlMessage {
            session: ALICE,
            message: ControlMessage::Setup(Setup::default()),
        }),
        []
    );
}

#[test]
fn an_object_goes_to_every_subscriber_under_its_own_alias() {
    let mut relay = relay_with_track();
    assert_eq!(
        subscribe_to_track(&mut relay, ALICE),
        [subscribe_ok(ALICE, None)]
    );
    assert_eq!(
        subscribe_to_track(&mut relay, BOB),
        [subscribe_ok(BOB, None)]
    );

    // Each subscriber session numbers its aliases from 0; the publisher's 40
    // never leaves its session (§11.1).
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
    assert_eq!(
        data_stream_closed(&mut relay, 7, StreamEnd::Fin),
        [
            Action::FinishDataStream {
                out_stream: OutStreamKey(0)
            },
            Action::FinishDataStream {
                out_stream: OutStreamKey(1)
            },
        ]
    );
    // The relay keeps a finished stream until the I/O side has written it out.
    assert_eq!(held(&relay), (3, 1, 2, 0, 2));
    out_stream_closed(&mut relay, 0);
    out_stream_closed(&mut relay, 1);
    assert_eq!(held(&relay), (3, 1, 2, 0, 0));
}

#[test]
fn publish_done_reaches_the_subscribers_once_the_streams_ended() {
    // (streams the publisher counts, what ends the wait, actions of that step)
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);
    object_arrived(&mut relay, 7, 0);

    // PUBLISH_DONE overtakes the end of the data stream: the relay waits.
    assert_eq!(
        relay.step(Event::RequestMessage {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: publish_done(0x2, 1),
        }),
        [Action::StartDrainTimer {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
        }]
    );
    // The publisher closes its request stream right after; nothing ends yet.
    assert_eq!(
        relay.step(Event::RequestClosed {
            session: PUBLISHER,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        }),
        []
    );
    // A subscriber that comes now is too late: no upstream is left (§9.4).
    assert_eq!(
        subscribe_to_track(&mut relay, BOB),
        [
            Action::SendRequest {
                session: BOB,
                stream: StreamKey(0),
                message: request_error(0x10),
            },
            Action::FinishRequest {
                session: BOB,
                stream: StreamKey(0),
            },
        ]
    );

    let mut expected = vec![
        Action::FinishDataStream {
            out_stream: OutStreamKey(0),
        },
        // The one stream PUBLISH_DONE counted has ended: the relay says so to
        // the publisher by finishing its direction of the request stream.
        Action::FinishRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
        },
    ];
    // TRACK_ENDED (0x2), and the one stream the relay opened for Alice.
    expected.extend(done_actions(ALICE, 0x2, 1));
    assert_eq!(data_stream_closed(&mut relay, 7, StreamEnd::Fin), expected);
    assert_eq!(held(&relay), (3, 0, 0, 0, 1));
    out_stream_closed(&mut relay, 0);
    assert_eq!(held(&relay), (3, 0, 0, 0, 0));

    // The timer that was started fires into nothing.
    assert_eq!(
        relay.step(Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
        }),
        []
    );
}

#[test]
fn the_drain_timer_ends_a_track_whose_streams_never_came() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);
    // The publisher counts two streams; the second one never arrives.
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });

    let mut expected = vec![
        Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(7),
            code: 0x1,
        },
        // The relay stopped waiting: a reset, CANCELLED (0x1), where a FIN
        // would have said that everything arrived.
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x1,
        },
        Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x1,
        },
    ];
    expected.extend(done_actions(ALICE, 0x2, 1));
    assert_eq!(
        relay.step(Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
        }),
        expected
    );
    assert_eq!(held(&relay), (3, 0, 0, 0, 0));
}

#[test]
fn a_subscriber_that_joins_inside_a_subgroup_gets_a_stream_of_its_own() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // The header leaves the Subgroup ID to the first Object (§11.4.2, mode
    // 0b01), so Alice's stream is opened when that Object comes.
    let first_object_names_the_subgroup = SubgroupHeader {
        subgroup_id: SubgroupId::FirstObjectId,
        ..header(UPSTREAM_ALIAS)
    };
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(7),
            header: first_object_names_the_subgroup,
        }),
        []
    );
    assert_eq!(
        object_arrived(&mut relay, 7, 4),
        [
            open_data_stream(
                ALICE,
                0,
                SubgroupHeader {
                    subgroup_id: SubgroupId::FirstObjectId,
                    ..header(0)
                }
            ),
            send_object(0, 4),
        ]
    );

    // LARGEST_OBJECT is there once Objects were published (§10.2.11).
    assert_eq!(
        subscribe_to_track(&mut relay, BOB),
        [subscribe_ok(
            BOB,
            Some(Location {
                group: 3,
                object: 4
            })
        )]
    );
    // Bob's stream starts at Object 6. Its header names the Subgroup ID (4,
    // the first Object of the Subgroup) and does not claim FIRST_OBJECT.
    assert_eq!(
        object_arrived(&mut relay, 7, 6),
        [
            open_data_stream(
                BOB,
                1,
                SubgroupHeader {
                    subgroup_id: SubgroupId::Explicit(4),
                    first_object: false,
                    ..header(0)
                }
            ),
            send_object(0, 6),
            send_object(1, 6),
        ]
    );
}

#[test]
fn a_subscription_to_a_track_nobody_published_is_rejected() {
    let mut relay = relay_with_track();
    // DOES_NOT_EXIST (0x10), then a FIN (§3.3.2).
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(0),
            first: subscribe(0, b"other"),
        }),
        [
            Action::SendRequest {
                session: ALICE,
                stream: StreamKey(0),
                message: request_error(0x10),
            },
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(0),
            },
        ]
    );
    assert_eq!(held(&relay), (3, 1, 0, 0, 0));
}

#[test]
fn requests_the_relay_does_not_serve_are_refused_and_the_session_goes_on() {
    let mut relay = relay_with_track();
    // (first message, REQUEST_ERROR code)
    let cases = [
        // FETCH: NOT_SUPPORTED.
        (
            ControlMessage::UnsupportedRequest {
                message_type: 0x16,
                request_id: 0,
            },
            0x3,
        ),
        // A second publisher of the track: UNINTERESTED.
        (publish(2, b"a"), 0x20),
    ];
    for (index, (first, error_code)) in cases.into_iter().enumerate() {
        let stream = StreamKey(u64::try_from(index).unwrap());
        assert_eq!(
            relay.step(Event::RequestOpened {
                session: ALICE,
                stream,
                first: first.clone(),
            }),
            [
                Action::SendRequest {
                    session: ALICE,
                    stream,
                    message: request_error(error_code),
                },
                Action::FinishRequest {
                    session: ALICE,
                    stream,
                },
            ],
            "{first:?}"
        );
        // What the peer still sends on the stream is ignored.
        assert_eq!(
            relay.step(Event::RequestMessage {
                session: ALICE,
                stream,
                message: publish_done(0x2, 0),
            }),
            []
        );
    }
    // A namespace is accepted; see the FIXME on routing in the crate docs.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(9),
            first: ControlMessage::PublishNamespace(PublishNamespace {
                request_id: 4,
                track_namespace: namespace(),
                parameters: vec![],
            }),
        }),
        [Action::SendRequest {
            session: ALICE,
            stream: StreamKey(9),
            message: ControlMessage::RequestOk(RequestOk::default()),
        }]
    );
    assert_eq!(held(&relay), (3, 1, 0, 0, 0));
}

#[test]
fn a_subscriber_that_leaves_does_not_disturb_the_others() {
    // (how Alice leaves, the reset code her open stream gets)
    let leavings: [(&str, Event, Vec<Action>); 2] = [
        (
            "cancels the request",
            Event::RequestClosed {
                session: ALICE,
                stream: StreamKey(0),
                how: RequestEnd::Aborted,
            },
            vec![
                Action::FinishRequest {
                    session: ALICE,
                    stream: StreamKey(0),
                },
                Action::ResetDataStream {
                    out_stream: OutStreamKey(0),
                    code: 0x1,
                },
            ],
        ),
        // Nothing can be done on a session that is gone.
        (
            "disconnects",
            Event::SessionClosed { session: ALICE },
            vec![],
        ),
    ];
    for (name, leaving, expected) in leavings {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        subscribe_to_track(&mut relay, BOB);
        data_stream_opened(&mut relay, 7);

        assert_eq!(relay.step(leaving), expected, "{name}");
        assert_eq!(
            object_arrived(&mut relay, 7, 0),
            [send_object(1, 0)],
            "{name}"
        );
        // A stream opened later goes to Bob alone.
        assert_eq!(
            data_stream_opened(&mut relay, 8),
            [open_data_stream(BOB, 2, header_of_group(0, 8))],
            "{name}"
        );
    }
}

#[test]
fn a_publisher_that_leaves_ends_the_track_for_every_subscriber() {
    // (how the publisher leaves)
    let leavings = [
        (
            "cancels the request",
            Event::RequestClosed {
                session: PUBLISHER,
                stream: StreamKey(0),
                how: RequestEnd::Aborted,
            },
            // The publisher is still there: the relay ends its direction of
            // the request stream, and the data stream is stopped (§10.11).
            vec![
                Action::FinishRequest {
                    session: PUBLISHER,
                    stream: StreamKey(0),
                },
                Action::StopDataStream {
                    session: PUBLISHER,
                    stream: StreamKey(7),
                    code: 0x1,
                },
            ],
            (3, 0, 0, 0, 0),
        ),
        (
            "disconnects",
            Event::SessionClosed { session: PUBLISHER },
            vec![],
            (2, 0, 0, 0, 0),
        ),
    ];
    for (name, leaving, towards_publisher, counts) in leavings {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        subscribe_to_track(&mut relay, BOB);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);

        // The open streams are reset first, then PUBLISH_DONE goes out with
        // TRACK_ENDED (0x2) and the count of streams each subscriber got.
        let mut expected = towards_publisher;
        expected.extend([
            Action::ResetDataStream {
                out_stream: OutStreamKey(0),
                code: 0x1,
            },
            Action::ResetDataStream {
                out_stream: OutStreamKey(1),
                code: 0x1,
            },
        ]);
        expected.extend(done_actions(ALICE, 0x2, 1));
        expected.extend(done_actions(BOB, 0x2, 1));
        assert_eq!(relay.step(leaving), expected, "{name}");
        assert_eq!(held(&relay), counts, "{name}");

        // The track can be published again.
        assert_eq!(
            relay
                .handle(Event::RequestOpened {
                    session: ALICE,
                    stream: StreamKey(5),
                    first: publish(2, b"a"),
                })
                .len(),
            1,
            "{name}"
        );
    }
}

#[test]
fn a_subscriber_that_falls_behind_loses_its_subscription_alone() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    subscribe_to_track(&mut relay, BOB);
    data_stream_opened(&mut relay, 7);
    data_stream_opened(&mut relay, 8);

    // Alice's stream 0 is full. Both of her streams are reset with
    // TOO_FAR_BEHIND (0x5), and PUBLISH_DONE says the same and counts them.
    let mut expected = vec![
        Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x5,
        },
        Action::ResetDataStream {
            out_stream: OutStreamKey(2),
            code: 0x5,
        },
    ];
    expected.extend(done_actions(ALICE, 0x5, 2));
    // The resets come in the order her streams were started.
    assert_eq!(
        relay.step(Event::SendQueueFull {
            out_stream: OutStreamKey(0),
        }),
        expected
    );

    // Reported again for a stream that is already gone: nothing.
    assert_eq!(
        relay.step(Event::SendQueueFull {
            out_stream: OutStreamKey(2),
        }),
        []
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), [send_object(1, 0)]);
    assert_eq!(held(&relay), (3, 1, 1, 2, 2));
}

#[test]
fn a_stream_that_is_cut_short_is_reset_downstream() {
    // (how the publisher's stream ended, the code the subscriber's stream gets)
    let cases = [
        // The publisher's own code is passed on: DELIVERY_TIMEOUT (0x2).
        (StreamEnd::Aborted(Some(0x2)), 0x2),
        // Reset without a code the relay can read, or stopped by the relay:
        // INTERNAL_ERROR (0x0).
        (StreamEnd::Aborted(None), 0x0),
        // Cut off with the publisher's connection: CANCELLED (0x1), the code
        // the stream gets when the end of the session is seen first.
        (StreamEnd::Lost, 0x1),
    ];
    for (end, code) in cases {
        let mut relay = relay_with_track();
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        assert_eq!(
            data_stream_closed(&mut relay, 7, end),
            [Action::ResetDataStream {
                out_stream: OutStreamKey(0),
                code,
            }],
            "{end:?}"
        );
        // The subscription itself goes on.
        assert_eq!(held(&relay), (3, 1, 1, 0, 0), "{end:?}");
    }
}

#[test]
fn a_malformed_track_is_cancelled_upstream_and_ended_downstream() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);

    // Everything carries MALFORMED_TRACK (0x12).
    let mut expected = vec![
        Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(7),
            code: 0x12,
        },
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x12,
        },
        Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x12,
        },
    ];
    expected.extend(done_actions(ALICE, 0x12, 1));
    assert_eq!(
        relay.step(Event::TrackMalformed {
            session: PUBLISHER,
            stream: StreamKey(7),
        }),
        expected
    );
    // The sessions stay (§2.4.2); the stream's own end changes nothing more.
    assert_eq!(
        data_stream_closed(&mut relay, 7, StreamEnd::Aborted(None)),
        []
    );
    assert_eq!(held(&relay), (3, 0, 0, 0, 0));
}

#[test]
fn a_stream_that_arrives_before_its_publish_is_held() {
    let mut relay = Relay::new("test/0");
    relay.step(Event::SessionOpened {
        session: PUBLISHER,
        role: Role::Server,
    });
    relay.step(Event::SessionOpened {
        session: ALICE,
        role: Role::Server,
    });

    // §11.4.2: held "for a brief period", which the I/O side times.
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [Action::StartEarlyTimer {
            session: PUBLISHER,
            stream: StreamKey(7),
        }]
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), []);
    assert_eq!(data_stream_closed(&mut relay, 7, StreamEnd::Fin), []);

    // The PUBLISH explains the alias; nobody subscribes yet, so the held
    // stream is consumed without a trace, but it counts for LARGEST_OBJECT.
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: publish(0, b"a"),
    });
    assert_eq!(
        subscribe_to_track(&mut relay, ALICE),
        [subscribe_ok(
            ALICE,
            Some(Location {
                group: 3,
                object: 0
            })
        )]
    );
    assert_eq!(held(&relay), (2, 1, 1, 0, 0));
}

#[test]
fn early_streams_are_held_up_to_a_limit() {
    // The expectations below are spelled out for limits of 8 and 32.
    assert_eq!(super::MAX_EARLY_STREAMS, 8);
    assert_eq!(super::MAX_EARLY_OBJECTS, 32);
    let stopped = |stream| {
        vec![Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(stream),
            code: 0x0,
        }]
    };

    let mut relay = Relay::new("test/0");
    relay.step(Event::SessionOpened {
        session: PUBLISHER,
        role: Role::Server,
    });
    for stream in 0..8 {
        assert_eq!(
            data_stream_opened(&mut relay, stream),
            [Action::StartEarlyTimer {
                session: PUBLISHER,
                stream: StreamKey(stream),
            }]
        );
    }
    // The ninth stream is not held.
    assert_eq!(data_stream_opened(&mut relay, 8), stopped(8));

    for object_id in 0..32 {
        assert_eq!(object_arrived(&mut relay, 0, object_id), []);
    }
    // The 33rd Object costs its stream.
    assert_eq!(object_arrived(&mut relay, 1, 0), stopped(1));
}

#[test]
fn a_late_stream_of_an_ended_track_is_stopped() {
    let mut relay = relay_with_track();
    relay.step(Event::RequestClosed {
        session: PUBLISHER,
        stream: StreamKey(0),
        how: RequestEnd::Aborted,
    });
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(7),
            code: 0x1,
        }]
    );
}

#[test]
fn a_protocol_error_closes_one_session_and_cleans_up_after_it() {
    // (what the publisher does wrong, the Session Termination Error Code)
    let faults = [
        // A second SETUP on the control stream: PROTOCOL_VIOLATION.
        (
            Event::ControlMessage {
                session: PUBLISHER,
                message: ControlMessage::Setup(Setup::default()),
            },
            0x3,
        ),
        // A request stream that begins with a response: PROTOCOL_VIOLATION.
        (
            Event::RequestOpened {
                session: PUBLISHER,
                stream: StreamKey(1),
                first: ControlMessage::RequestOk(RequestOk::default()),
            },
            0x3,
        ),
        // A Request ID of the server's parity: INVALID_REQUEST_ID.
        (
            Event::RequestOpened {
                session: PUBLISHER,
                stream: StreamKey(1),
                first: subscribe(1, b"b"),
            },
            0x4,
        ),
        // The Track Alias of "a" for another track: DUPLICATE_TRACK_ALIAS.
        (
            Event::RequestOpened {
                session: PUBLISHER,
                stream: StreamKey(1),
                first: publish(2, b"b"),
            },
            0x5,
        ),
        // Found by the I/O side: an unknown stream type, PROTOCOL_VIOLATION.
        (
            Event::SessionFailed {
                session: PUBLISHER,
                error: xmoq_moqt::Error::UnknownStreamType(0x20),
            },
            0x3,
        ),
    ];
    for (fault, code) in faults {
        let mut relay = relay_with_track();
        relay.step(Event::ControlMessage {
            session: PUBLISHER,
            message: ControlMessage::Setup(Setup::default()),
        });
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);

        let actions = relay.step(fault.clone());
        let Some(Action::CloseSession {
            session,
            code: closed_with,
            ..
        }) = actions.first()
        else {
            panic!("{fault:?} led to {actions:?}");
        };
        assert_eq!((*session, *closed_with), (PUBLISHER, code), "{fault:?}");
        // §9.5: Alice sees her subscription end, not what the publisher did.
        let mut rest = vec![Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x1,
        }];
        rest.extend(done_actions(ALICE, 0x2, 1));
        assert_eq!(actions[1..], rest, "{fault:?}");
        assert_eq!(held(&relay), (2, 0, 0, 0, 0), "{fault:?}");

        // The I/O side reports the session gone afterwards.
        assert_eq!(
            relay.step(Event::SessionClosed { session: PUBLISHER }),
            [],
            "{fault:?}"
        );
    }
}

#[test]
fn a_request_update_is_refused_and_ends_the_subscription() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    data_stream_opened(&mut relay, 7);

    // NOT_SUPPORTED (0x3), the stream reset with CANCELLED (0x1), then
    // PUBLISH_DONE with UPDATE_FAILED (0x8) (§10.9.1).
    let mut expected = vec![
        Action::SendRequest {
            session: ALICE,
            stream: StreamKey(0),
            message: request_error(0x3),
        },
        Action::ResetDataStream {
            out_stream: OutStreamKey(0),
            code: 0x1,
        },
    ];
    expected.extend(done_actions(ALICE, 0x8, 1));
    assert_eq!(
        relay.step(Event::RequestMessage {
            session: ALICE,
            stream: StreamKey(0),
            message: ControlMessage::RequestUpdate(RequestUpdate {
                request_id: 2,
                parameters: vec![],
            }),
        }),
        expected
    );
    assert_eq!(held(&relay), (3, 1, 0, 1, 0));
}

#[test]
fn events_for_what_the_relay_let_go_are_ignored() {
    let mut relay = relay_with_track();
    relay.step(Event::SessionClosed { session: PUBLISHER });
    let stale = [
        Event::ControlMessage {
            session: PUBLISHER,
            message: ControlMessage::Setup(Setup::default()),
        },
        Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: subscribe(2, b"a"),
        },
        Event::RequestMessage {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: publish_done(0x2, 0),
        },
        Event::RequestClosed {
            session: PUBLISHER,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        },
        Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(7),
            header: header(UPSTREAM_ALIAS),
        },
        Event::Object {
            session: PUBLISHER,
            stream: StreamKey(7),
            object: object(0),
        },
        Event::DataStreamClosed {
            session: PUBLISHER,
            stream: StreamKey(7),
            end: StreamEnd::Fin,
        },
        Event::TrackMalformed {
            session: PUBLISHER,
            stream: StreamKey(7),
        },
        Event::SendQueueFull {
            out_stream: OutStreamKey(9),
        },
        Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
        },
        Event::SessionClosed { session: PUBLISHER },
    ];
    for event in stale {
        assert_eq!(relay.step(event.clone()), [], "{event:?}");
    }
    assert_eq!(held(&relay), (2, 0, 0, 0, 0));
}

fn out_stream_closed(relay: &mut Relay, out_stream: u64) {
    assert_eq!(
        relay.handle(Event::OutStreamClosed {
            out_stream: OutStreamKey(out_stream),
        }),
        []
    );
}

#[test]
fn a_relay_that_ended_subscriptions_holds_nothing_of_them() {
    let mut relay = relay_with_track();
    let baseline = relay.counts().requests;
    // Alice subscribes and the publisher cancels, over and over. Each round
    // the relay ends her subscription with PUBLISH_DONE, and she closes her
    // stream. Nothing may be left of a round.
    for round in 0..300u64 {
        let stream = StreamKey(round);
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream,
            first: subscribe(round * 2, b"a"),
        });
        assert_eq!(actions.len(), 1, "round {round}: {actions:?}");
        relay.step(Event::RequestClosed {
            session: PUBLISHER,
            stream: StreamKey(round),
            how: RequestEnd::Aborted,
        });
        relay.step(Event::RequestClosed {
            session: ALICE,
            stream,
            how: RequestEnd::Aborted,
        });
        // The publisher offers the track again, on a new stream.
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(round + 1),
            first: publish(round * 2 + 2, b"a"),
        });
        assert_eq!(relay.counts().requests, baseline, "round {round}");
        assert_eq!(held(&relay), (3, 1, 0, 0, 0), "round {round}");
        assert_eq!(relay.counts().early_streams, 0, "round {round}");
    }
}

#[test]
fn what_a_subscriber_sends_after_publish_done_is_ignored_but_counted() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    relay.step(Event::RequestClosed {
        session: PUBLISHER,
        stream: StreamKey(0),
        how: RequestEnd::Aborted,
    });
    let update = |request_id| Event::RequestMessage {
        session: ALICE,
        stream: StreamKey(0),
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters: vec![],
        }),
    };
    // A REQUEST_UPDATE that crossed the PUBLISH_DONE: nothing is answered.
    assert_eq!(relay.step(update(2)), []);
    // Its Request ID was used up all the same: INVALID_REQUEST_ID (0x4).
    let actions = relay.step(update(2));
    assert!(
        matches!(
            actions.first(),
            Some(Action::CloseSession {
                session: ALICE,
                code: 0x4,
                ..
            })
        ),
        "{actions:?}"
    );
}

#[test]
fn the_largest_object_of_a_publish_is_passed_on() {
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    let largest = Location {
        group: 5,
        object: 3,
    };
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: ControlMessage::Publish(Publish {
            request_id: 0,
            full_track_name: track(b"a"),
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![Parameter::LargestObject(largest)],
            track_properties: vec![KeyValuePair::varint(2, 5).unwrap()],
        }),
    });
    // §10.2.11: no Object has passed through the relay yet.
    assert_eq!(
        subscribe_to_track(&mut relay, ALICE),
        [subscribe_ok(ALICE, Some(largest))]
    );
}

#[test]
fn a_subscriber_with_forward_state_0_gets_no_objects() {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"a"),
            parameters: vec![Parameter::Forward(false)],
        }),
    });
    subscribe_to_track(&mut relay, BOB);

    // Only Bob gets a stream and the Object.
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [open_data_stream(BOB, 0, header(0))]
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), [send_object(0, 0)]);
    // Alice is still told when the track ends, with no streams to count.
    out_stream_closed(&mut relay, 0);
    let actions = relay.step(Event::SessionClosed { session: PUBLISHER });
    assert_eq!(actions[..3], done_actions(ALICE, 0x2, 0));
}

#[test]
fn a_stream_that_may_never_have_been_opened_makes_the_count_unknown() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // The I/O side has not reported the stream opened when the subscription
    // is cut off: the subscriber's stream limit may have kept it from opening.
    let opened = relay.handle(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(7),
        header: header(UPSTREAM_ALIAS),
    });
    assert_eq!(opened, [open_data_stream(ALICE, 0, header(0))]);

    // §10.11: 2^62 - 1 when the exact number cannot be given.
    let mut expected = vec![Action::ResetDataStream {
        out_stream: OutStreamKey(0),
        code: 0x5,
    }];
    expected.extend(done_actions(ALICE, 0x5, 4_611_686_018_427_387_903));
    assert_eq!(
        relay.step(Event::SendQueueFull {
            out_stream: OutStreamKey(0),
        }),
        expected
    );
}

#[test]
fn a_finished_stream_that_is_not_written_out_is_reset_with_its_subscription() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // Three short Subgroups, each finished by the publisher. Alice reads
    // nothing, so none of her streams is reported done.
    for stream in 0..3 {
        data_stream_opened(&mut relay, 10 + stream);
        object_arrived(&mut relay, 10 + stream, 0);
        data_stream_closed(&mut relay, 10 + stream, StreamEnd::Fin);
    }
    assert_eq!(held(&relay), (3, 1, 1, 0, 3));

    // Her budget runs out on the next stream. All three finished streams are
    // reset with TOO_FAR_BEHIND (0x5) along with the new one.
    data_stream_opened(&mut relay, 13);
    let mut expected: Vec<Action> = [0, 1, 2, 3]
        .into_iter()
        .map(|out_stream| Action::ResetDataStream {
            out_stream: OutStreamKey(out_stream),
            code: 0x5,
        })
        .collect();
    expected.extend(done_actions(ALICE, 0x5, 4));
    assert_eq!(
        relay.step(Event::SendQueueFull {
            out_stream: OutStreamKey(3),
        }),
        expected
    );
    assert_eq!(held(&relay), (3, 1, 0, 1, 0));
}

#[test]
fn early_streams_of_a_refused_publish_are_let_go() {
    let mut relay = relay_with_track();
    // Alice sends eight streams ahead of a PUBLISH for the track that already
    // has a publisher.
    for stream in 0..8 {
        relay.step(Event::DataStreamOpened {
            session: ALICE,
            stream: StreamKey(10 + stream),
            header: header(UPSTREAM_ALIAS),
        });
    }
    assert_eq!(relay.counts().early_streams, 8);
    let actions = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: publish(0, b"a"),
    });
    // The eight streams are stopped with CANCELLED (0x1), then the PUBLISH is
    // refused.
    let stopped = actions
        .iter()
        .filter(|action| matches!(action, Action::StopDataStream { code: 0x1, .. }))
        .count();
    assert_eq!(stopped, 8);
    assert_eq!(relay.counts().early_streams, 0);

    // Her next track can run ahead of its PUBLISH again.
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: ALICE,
            stream: StreamKey(20),
            header: header(41),
        }),
        [Action::StartEarlyTimer {
            session: ALICE,
            stream: StreamKey(20),
        }]
    );
}

#[test]
fn an_early_stream_is_held_for_a_while_only() {
    let mut relay = Relay::new("test/0");
    relay.step(Event::SessionOpened {
        session: PUBLISHER,
        role: Role::Server,
    });
    // (whether the stream ended before the time was up, what the timeout does)
    let cases = [
        (
            false,
            vec![Action::StopDataStream {
                session: PUBLISHER,
                stream: StreamKey(7),
                code: 0x1,
            }],
        ),
        // Nothing is left to stop on a stream that already ended.
        (true, vec![]),
    ];
    for (ended, expected) in cases {
        data_stream_opened(&mut relay, 7);
        if ended {
            data_stream_closed(&mut relay, 7, StreamEnd::Fin);
        }
        assert_eq!(
            relay.step(Event::EarlyStreamTimeout {
                session: PUBLISHER,
                stream: StreamKey(7),
            }),
            expected,
            "ended {ended}"
        );
        assert_eq!(relay.counts().early_streams, 0, "ended {ended}");
        // The timer of a stream that is no longer held does nothing.
        assert_eq!(
            relay.step(Event::EarlyStreamTimeout {
                session: PUBLISHER,
                stream: StreamKey(7),
            }),
            [],
            "ended {ended}"
        );
    }
}

#[test]
fn early_objects_are_held_up_to_a_number_of_bytes() {
    // The expectation below is spelled out for a limit of 1 MiB.
    assert_eq!(super::MAX_EARLY_BYTES, 1_048_576);
    let mut relay = Relay::new("test/0");
    relay.step(Event::SessionOpened {
        session: PUBLISHER,
        role: Role::Server,
    });
    data_stream_opened(&mut relay, 7);
    let large = |object_id| Event::Object {
        session: PUBLISHER,
        stream: StreamKey(7),
        object: SubgroupObject {
            payload: vec![0; 600_000],
            ..object(object_id)
        },
    };
    assert_eq!(relay.step(large(0)), []);
    // The second one takes the session over 1 MiB.
    assert_eq!(
        relay.step(large(1)),
        [Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(7),
            code: 0x0,
        }]
    );
}

#[test]
fn the_relay_takes_the_role_it_is_given() {
    // (the relay's role, a Request ID the peer may use, one it may not)
    let cases = [(Role::Server, 0, 1), (Role::Client, 1, 0)];
    for (role, valid, invalid) in cases {
        let mut relay = Relay::new("test/0");
        relay.step(Event::SessionOpened {
            session: PUBLISHER,
            role,
        });
        let opened = |request_id, stream| Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(stream),
            first: publish(request_id, b"a"),
        };
        assert!(
            matches!(
                relay.step(opened(valid, 0)).first(),
                Some(Action::SendRequest { .. })
            ),
            "{role:?}"
        );
        assert!(
            matches!(
                relay.step(opened(invalid, 1)).first(),
                Some(Action::CloseSession { code: 0x4, .. })
            ),
            "{role:?}"
        );
    }
}

#[test]
fn a_request_the_peer_does_not_close_after_publish_done_is_let_go() {
    let mut relay = relay_with_track();
    let baseline = relay.counts().requests;
    subscribe_to_track(&mut relay, ALICE);
    relay.step(Event::RequestClosed {
        session: PUBLISHER,
        stream: StreamKey(0),
        how: RequestEnd::Aborted,
    });
    // Alice got PUBLISH_DONE and keeps her direction of the stream open: the
    // request still counts.
    assert_eq!(relay.counts().requests, baseline);

    // CANCELLED (0x1) towards her, and the request is gone.
    let timeout = || Event::RequestTimeout {
        session: ALICE,
        stream: StreamKey(0),
    };
    assert_eq!(
        relay.step(timeout()),
        [Action::StopRequest {
            session: ALICE,
            stream: StreamKey(0),
            code: 0x1,
        }]
    );
    assert_eq!(relay.counts().requests, baseline - 1);
    // A second timer, or one for a stream that closed in time, does nothing.
    assert_eq!(relay.step(timeout()), []);
    assert_eq!(
        relay.step(Event::RequestClosed {
            session: ALICE,
            stream: StreamKey(0),
            how: RequestEnd::Aborted,
        }),
        []
    );
    assert_eq!(relay.step(timeout()), []);
}

#[test]
fn the_timer_of_a_request_that_is_still_live_does_nothing() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    assert_eq!(
        relay.step(Event::RequestTimeout {
            session: ALICE,
            stream: StreamKey(0),
        }),
        []
    );
    assert_eq!(held(&relay), (3, 1, 1, 0, 0));
}

#[test]
fn early_objects_count_with_their_properties() {
    // The expectation below is spelled out for a limit of 1 MiB.
    assert_eq!(super::MAX_EARLY_BYTES, 1_048_576);
    let mut relay = Relay::new("test/0");
    relay.step(Event::SessionOpened {
        session: PUBLISHER,
        role: Role::Server,
    });
    data_stream_opened(&mut relay, 7);
    // No payload, and ten properties of 60,000 bytes: 600,160 bytes held.
    let heavy = |object_id| Event::Object {
        session: PUBLISHER,
        stream: StreamKey(7),
        object: SubgroupObject {
            properties: (0..10)
                .map(|_| KeyValuePair::bytes(0x21, vec![0; 60_000]).unwrap())
                .collect(),
            payload: vec![],
            ..object(object_id)
        },
    };
    assert_eq!(relay.step(heavy(0)), []);
    // The second one takes the session over 1 MiB.
    assert_eq!(
        relay.step(heavy(1)),
        [Action::StopDataStream {
            session: PUBLISHER,
            stream: StreamKey(7),
            code: 0x0,
        }]
    );
}

#[test]
fn publish_done_counts_the_streams_of_a_subscriber_that_joined_late() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // Two Subgroups pass before Bob subscribes, and he joins inside a third.
    for stream in [10, 11] {
        data_stream_opened(&mut relay, stream);
        object_arrived(&mut relay, stream, 0);
        data_stream_closed(&mut relay, stream, StreamEnd::Fin);
    }
    data_stream_opened(&mut relay, 12);
    object_arrived(&mut relay, 12, 0);
    subscribe_to_track(&mut relay, BOB);
    object_arrived(&mut relay, 12, 1);
    data_stream_closed(&mut relay, 12, StreamEnd::Fin);

    // The publisher leaves: Alice was sent three streams, Bob one.
    let actions = relay.step(Event::SessionClosed { session: PUBLISHER });
    let mut expected = done_actions(ALICE, 0x2, 3).to_vec();
    expected.extend(done_actions(BOB, 0x2, 1));
    assert_eq!(actions, expected);
}

#[test]
fn the_open_streams_of_a_track_that_ends_are_stopped_in_the_order_of_their_numbers() {
    // Six streams, opened in an order that is neither rising nor falling: an
    // order left to chance would come out as the one below once in 720 times.
    let mut relay = relay_with_track();
    for stream in [9, 4, 11, 7, 5, 8] {
        data_stream_opened(&mut relay, stream);
    }
    let actions = relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(7),
    });
    let stopped: Vec<u64> = actions
        .iter()
        .filter_map(|action| match action {
            Action::StopDataStream { stream, .. } => Some(stream.0),
            _ => None,
        })
        .collect();
    assert_eq!(stopped, [4, 5, 7, 8, 9, 11]);
}

#[test]
fn the_tracks_of_a_publisher_that_goes_end_in_the_order_of_their_names() {
    // The publisher offers five tracks in an order that is not that of their
    // names, and Alice subscribes to them in another: an order left to chance
    // would come out as the one below once in 120 times.
    let published = [b"d", b"a", b"e", b"c", b"b"];
    let subscribed = [b"c", b"e", b"b", b"a", b"d"];
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    for (stream, name) in (0u64..).zip(published) {
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(stream),
            first: ControlMessage::Publish(Publish {
                request_id: stream * 2,
                full_track_name: track(name),
                track_alias: 40 + stream,
                parameters: vec![],
                track_properties: vec![],
            }),
        });
    }
    // Alice's request stream for a track has the number its name has in the
    // alphabet: "a" is on stream 0, "e" on stream 4.
    for name in subscribed {
        let stream = u64::from(name[0] - b'a');
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(stream),
            first: subscribe(stream * 2, name),
        });
    }
    let actions = relay.step(Event::SessionClosed { session: PUBLISHER });
    // Alice is told track by track that it is over, with PUBLISH_DONE.
    let done_on: Vec<u64> = actions
        .iter()
        .filter_map(|action| match action {
            Action::SendRequest {
                session: ALICE,
                stream,
                message: ControlMessage::PublishDone(_),
            } => Some(stream.0),
            _ => None,
        })
        .collect();
    assert_eq!(done_on, [0, 1, 2, 3, 4]);
}

#[test]
fn a_drain_timer_of_a_track_that_is_gone_does_not_end_the_next_one_of_its_name() {
    let mut relay = relay_with_track();
    data_stream_opened(&mut relay, 7);
    // The publisher counts two streams, and then its track turns out to be
    // malformed: the track ends while the alias still waits for a stream.
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });
    relay.step(Event::TrackMalformed {
        session: PUBLISHER,
        stream: StreamKey(7),
    });
    assert_eq!(relay.counts().tracks, 0);
    // Alice publishes a track of the same name.
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: publish(0, b"a"),
    });
    assert_eq!(relay.counts().tracks, 1);
    // The timer of the first track is for the publisher's alias, which is no
    // source of Alice's track.
    relay.step(Event::DrainTimeout {
        session: PUBLISHER,
        track_alias: UPSTREAM_ALIAS,
    });
    assert_eq!(relay.counts().tracks, 1);
}
