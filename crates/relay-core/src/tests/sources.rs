//! Several sources of one track (§9.3, §9.5).

use super::*;

/// The alias Bob chose for the track he publishes.
const BOBS_ALIAS: u64 = 41;

fn publish_ok(session: SessionKey) -> Action {
    Action::SendRequest {
        session,
        stream: StreamKey(0),
        message: ControlMessage::RequestOk(RequestOk {
            parameters: vec![Parameter::Forward(true)],
            track_properties: vec![],
        }),
    }
}

/// A PUBLISH of the track "a" with Request ID 0.
fn publish_of_a(
    track_alias: u64,
    parameters: Vec<Parameter>,
    track_properties: Vec<KeyValuePair>,
) -> ControlMessage {
    ControlMessage::Publish(Publish {
        request_id: 0,
        full_track_name: track(b"a"),
        track_alias,
        parameters,
        track_properties,
    })
}

/// `session` publishes the track "a" on its stream 0, under `track_alias`.
fn publishes_a(relay: &mut Relay, session: SessionKey, track_alias: u64) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session,
        stream: StreamKey(0),
        first: publish_of_a(track_alias, vec![], vec![]),
    })
}

/// `relay_with_track`, where Bob publishes the track "a" as well.
fn relay_with_two_publishers() -> Relay {
    let mut relay = relay_with_track();
    assert_eq!(publishes_a(&mut relay, BOB, BOBS_ALIAS), [publish_ok(BOB)]);
    let counts = relay.counts();
    assert_eq!((counts.tracks, counts.sources, counts.asks), (1, 2, 0));
    relay
}

/// A source opens a stream of Subgroup 0 of `group`.
fn stream_from(relay: &mut Relay, source: SessionKey, stream: u64, group: u64) -> Vec<Action> {
    let track_alias = if source == BOB {
        BOBS_ALIAS
    } else {
        UPSTREAM_ALIAS
    };
    relay.step(Event::DataStreamOpened {
        session: source,
        stream: StreamKey(stream),
        header: header_of_group(track_alias, group),
    })
}

fn object_from(relay: &mut Relay, source: SessionKey, stream: u64, object_id: u64) -> Vec<Action> {
    relay.step(Event::Object {
        session: source,
        stream: StreamKey(stream),
        object: object(object_id),
    })
}

fn closed_from(relay: &mut Relay, source: SessionKey, stream: u64, end: StreamEnd) -> Vec<Action> {
    relay.step(Event::DataStreamClosed {
        session: source,
        stream: StreamKey(stream),
        end,
    })
}

/// The relay stops a stream of a source with `CANCELLED`.
fn stopped(source: SessionKey, stream: u64) -> Action {
    Action::StopDataStream {
        session: source,
        stream: StreamKey(stream),
        code: 0x1,
    }
}

fn reset(out_stream: u64, code: u32) -> Action {
    Action::ResetDataStream {
        out_stream: OutStreamKey(out_stream),
        code,
    }
}

#[test]
fn what_two_publishers_of_a_track_send_reaches_the_subscribers() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    // Each sends a Group of its own.
    assert_eq!(
        stream_from(&mut relay, PUBLISHER, 7, 1),
        [open_data_stream(ALICE, 0, header_of_group(0, 1))]
    );
    assert_eq!(
        stream_from(&mut relay, BOB, 7, 2),
        [open_data_stream(ALICE, 1, header_of_group(0, 2))]
    );
    assert_eq!(object_from(&mut relay, BOB, 7, 0), [send_object(1, 0)]);
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 7, 0),
        [send_object(0, 0)]
    );
}

#[test]
fn a_subgroup_is_taken_from_the_source_that_sends_it_first() {
    #[derive(Debug, Clone, Copy)]
    enum First {
        StillOpen,
        CutShort,
        Finished,
    }
    // (how the publisher's stream for the Subgroup stands when Bob's stream
    //  for the same Subgroup comes). Bob's stream is left in all three: the
    //  rule is the other way round from the one for the same source, where
    //  the second stream passes in the first two.
    for first in [First::StillOpen, First::CutShort, First::Finished] {
        let mut relay = relay_with_two_publishers();
        subscribe_to_track(&mut relay, ALICE);
        stream_from(&mut relay, PUBLISHER, 7, 1);
        object_from(&mut relay, PUBLISHER, 7, 0);
        match first {
            First::StillOpen => {}
            First::CutShort => {
                closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
            }
            First::Finished => {
                closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin);
            }
        }
        assert_eq!(
            stream_from(&mut relay, BOB, 8, 1),
            [stopped(BOB, 8)],
            "{first:?}"
        );
        // What Bob still sends on the stream is dropped.
        assert_eq!(object_from(&mut relay, BOB, 8, 0), [], "{first:?}");
        // The next Group comes from Bob, who sends it first, and the
        // publisher's stream for it is the one that is left.
        assert_eq!(
            stream_from(&mut relay, BOB, 9, 2),
            [open_data_stream(ALICE, 1, header_of_group(0, 2))],
            "{first:?}"
        );
        assert_eq!(
            stream_from(&mut relay, PUBLISHER, 9, 2),
            [stopped(PUBLISHER, 9)],
            "{first:?}"
        );
    }
}

/// The ways the publisher of [`relay_with_two_publishers`] goes while Bob
/// stays: a name, the events, and what the last of them leads to. When it
/// goes, Alice has stream 0 from the publisher's stream 7, which is open,
/// and stream 1 from Bob's stream 8.
fn ways_the_publisher_goes() -> [(&'static str, Vec<Event>, Vec<Action>); 4] {
    let drained_at_once = Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 1),
    };
    let done_with_a_stream_to_come = Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    };
    [
        (
            "it cancels its PUBLISH",
            vec![Event::RequestClosed {
                session: PUBLISHER,
                stream: StreamKey(0),
                how: RequestEnd::Aborted,
            }],
            // The relay finishes its direction of the request stream the
            // publisher ended (§3.3.2), as for any request.
            vec![
                Action::FinishRequest {
                    session: PUBLISHER,
                    stream: StreamKey(0),
                },
                stopped(PUBLISHER, 7),
                reset(0, 0x1),
            ],
        ),
        (
            "its session ends",
            vec![Event::SessionClosed { session: PUBLISHER }],
            vec![reset(0, 0x1)],
        ),
        (
            "PUBLISH_DONE, and the stream it counted ends",
            vec![
                drained_at_once,
                Event::DataStreamClosed {
                    session: PUBLISHER,
                    stream: StreamKey(7),
                    end: StreamEnd::Fin,
                },
            ],
            vec![
                Action::FinishDataStream {
                    out_stream: OutStreamKey(0),
                },
                Action::FinishRequest {
                    session: PUBLISHER,
                    stream: StreamKey(0),
                },
            ],
        ),
        (
            "PUBLISH_DONE, and a stream it counted does not come",
            vec![
                done_with_a_stream_to_come,
                Event::DrainTimeout {
                    session: PUBLISHER,
                    track_alias: UPSTREAM_ALIAS,
                    wait: 0,
                },
            ],
            vec![
                stopped(PUBLISHER, 7),
                Action::ResetRequest {
                    session: PUBLISHER,
                    stream: StreamKey(0),
                    code: 0x1,
                },
                reset(0, 0x1),
            ],
        ),
    ]
}

#[test]
fn a_source_that_goes_leaves_the_track_to_the_others() {
    for (name, events, expected) in ways_the_publisher_goes() {
        let mut relay = relay_with_two_publishers();
        subscribe_to_track(&mut relay, ALICE);
        stream_from(&mut relay, PUBLISHER, 7, 1);
        object_from(&mut relay, PUBLISHER, 7, 0);
        stream_from(&mut relay, BOB, 8, 2);
        let mut last = Vec::new();
        for event in events {
            last = relay.step(event);
        }
        assert_eq!(last, expected, "{name}");
        // Alice is still subscribed, to a track with one source.
        let counts = relay.counts();
        assert_eq!(
            (counts.tracks, counts.sources, counts.subscriptions),
            (1, 1, 1),
            "{name}"
        );
        // Bob does not go on with the Subgroup the publisher was sending,
        // however that ended.
        assert_eq!(
            stream_from(&mut relay, BOB, 9, 1),
            [stopped(BOB, 9)],
            "{name}"
        );
        // What Bob sends of his own goes on.
        assert_eq!(
            object_from(&mut relay, BOB, 8, 0),
            [send_object(1, 0)],
            "{name}"
        );
        // The track ends with its last source. Alice was sent two streams.
        let mut expected = vec![reset(1, 0x1)];
        expected.extend(done_actions(ALICE, 0x2, 2));
        assert_eq!(
            relay.step(Event::SessionClosed { session: BOB }),
            expected,
            "{name}"
        );
        // A stream that was finished stays in the books until the I/O side
        // has written it out; nothing else is left.
        let counts = relay.counts();
        assert_eq!(
            (counts.tracks, counts.subscriptions, counts.in_streams),
            (0, 0, 0),
            "{name}"
        );
    }
}

#[test]
fn a_track_ends_when_the_last_of_its_sources_has_drained() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    // The publisher is done and counted no stream: it is let go at once, and
    // the track goes on with Bob.
    assert_eq!(
        relay.step(Event::RequestMessage {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: publish_done(0x2, 0),
        }),
        [Action::FinishRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
        }]
    );
    assert_eq!(relay.counts().sources, 1);
    // A subscriber that comes now is accepted: Bob's subscription is
    // Established (§9.4).
    let carol = SessionKey(4);
    relay.step(Event::SessionOpened {
        session: carol,
        role: Role::Server,
    });
    // The Track Properties are Bob's, who gave none.
    assert_eq!(
        subscribe_to_track(&mut relay, carol),
        [Action::SendRequest {
            session: carol,
            stream: StreamKey(0),
            message: ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 0,
                parameters: vec![],
                track_properties: vec![],
            }),
        }]
    );
    let mut expected = vec![Action::FinishRequest {
        session: BOB,
        stream: StreamKey(0),
    }];
    expected.extend(done_actions(ALICE, 0x2, 0));
    expected.extend(done_actions(carol, 0x2, 0));
    assert_eq!(
        relay.step(Event::RequestMessage {
            session: BOB,
            stream: StreamKey(0),
            message: publish_done(0x2, 0),
        }),
        expected
    );
    assert_eq!(relay.counts().tracks, 0);
}

#[test]
fn a_track_takes_only_so_many_sources() {
    // The expectations below are spelled out for 4 sources.
    assert_eq!(crate::MAX_SOURCES, 4);
    let mut relay = relay_with_two_publishers();
    for session in [SessionKey(11), SessionKey(12), SessionKey(13)] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    for session in [SessionKey(11), SessionKey(12)] {
        assert_eq!(
            publishes_a(&mut relay, session, UPSTREAM_ALIAS),
            [publish_ok(session)]
        );
    }
    assert_eq!(relay.counts().sources, 4);
    // The fifth is refused with EXCESSIVE_LOAD (0x9), and the stream it sent
    // ahead of its PUBLISH is stopped.
    let fifth = SessionKey(13);
    relay.step(Event::DataStreamOpened {
        session: fifth,
        stream: StreamKey(5),
        header: header(UPSTREAM_ALIAS),
    });
    assert_eq!(
        publishes_a(&mut relay, fifth, UPSTREAM_ALIAS),
        [
            stopped(fifth, 5),
            Action::SendRequest {
                session: fifth,
                stream: StreamKey(0),
                message: request_error(0x9),
            },
            Action::FinishRequest {
                session: fifth,
                stream: StreamKey(0),
            },
        ]
    );
    let counts = relay.counts();
    assert_eq!((counts.sources, counts.early_streams), (4, 0));
    // Once a source is gone, there is room again: the fifth asks anew.
    // (The relay's REQUEST_ERROR had Retry Interval 0, which reads as "do not
    // retry"; that it says so here is a known gap, and a publisher that
    // asks again all the same is what shows that the room is there.)
    relay.step(Event::SessionClosed { session: BOB });
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: fifth,
            stream: StreamKey(1),
            first: ControlMessage::Publish(Publish {
                request_id: 2,
                full_track_name: track(b"a"),
                track_alias: UPSTREAM_ALIAS,
                parameters: vec![],
                track_properties: vec![],
            }),
        }),
        [Action::SendRequest {
            session: fifth,
            stream: StreamKey(1),
            message: ControlMessage::RequestOk(RequestOk {
                parameters: vec![Parameter::Forward(true)],
                track_properties: vec![],
            }),
        }]
    );
    assert_eq!(relay.counts().sources, 4);
}

#[test]
fn a_malformed_track_is_cancelled_at_every_source() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, PUBLISHER, 7, 1);
    stream_from(&mut relay, BOB, 8, 2);
    // What Bob sends breaks the object model. Bob is told so, with
    // MALFORMED_TRACK (0x12); the publisher, who did nothing wrong, is
    // cancelled with CANCELLED (0x1). Alice's streams and her subscription
    // end with MALFORMED_TRACK, whichever source they came from (§2.4.2).
    let mut expected = vec![
        stopped(PUBLISHER, 7),
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x1,
        },
        Action::StopDataStream {
            session: BOB,
            stream: StreamKey(8),
            code: 0x12,
        },
        Action::ResetRequest {
            session: BOB,
            stream: StreamKey(0),
            code: 0x12,
        },
        reset(0, 0x12),
        reset(1, 0x12),
    ];
    expected.extend(done_actions(ALICE, 0x12, 2));
    assert_eq!(
        relay.step(Event::TrackMalformed {
            session: BOB,
            stream: StreamKey(8),
        }),
        expected
    );
    assert_eq!(held(&relay), (3, 0, 0, 0, 0));
}

#[test]
fn the_properties_and_the_largest_object_of_a_track_with_two_publishers() {
    let location = |group, object| Location { group, object };
    // (the Object the publisher sent before Bob came, as Group 3 and its
    //  Object ID; Bob's LARGEST_OBJECT; Bob's Track Properties; the
    //  LARGEST_OBJECT and the Track Properties a subscriber gets after).
    // The Track Properties are Bob's, also when he gave none (§2.5); the
    // largest Location is the larger one (§10.2.11).
    let cases = [
        (None, None, vec![], None, vec![]),
        (
            None,
            Some(location(9, 1)),
            vec![KeyValuePair::varint(2, 7).unwrap()],
            Some(location(9, 1)),
            vec![KeyValuePair::varint(2, 7).unwrap()],
        ),
        (
            Some(5),
            Some(location(2, 0)),
            vec![],
            Some(location(3, 5)),
            vec![],
        ),
        (Some(5), None, vec![], Some(location(3, 5)), vec![]),
    ];
    for (sent, bobs_largest, bobs_properties, largest, properties) in cases {
        let mut relay = relay_with_track();
        if let Some(object_id) = sent {
            data_stream_opened(&mut relay, 7);
            object_arrived(&mut relay, 7, object_id);
        }
        let parameters = bobs_largest
            .map(Parameter::LargestObject)
            .into_iter()
            .collect();
        relay.step(Event::RequestOpened {
            session: BOB,
            stream: StreamKey(0),
            first: publish_of_a(BOBS_ALIAS, parameters, bobs_properties),
        });
        assert_eq!(
            subscribe_to_track(&mut relay, ALICE),
            [Action::SendRequest {
                session: ALICE,
                stream: StreamKey(0),
                message: ControlMessage::SubscribeOk(SubscribeOk {
                    track_alias: 0,
                    parameters: largest.map(Parameter::LargestObject).into_iter().collect(),
                    track_properties: properties,
                }),
            }],
            "{sent:?} {bobs_largest:?}"
        );
    }
}

#[test]
fn a_session_that_goes_takes_its_tracks_and_leaves_the_ones_it_shared() {
    let mut relay = relay_with_two_publishers();
    // The publisher has a second track, "b", of its own, and Alice subscribes
    // to both.
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"b"),
            track_alias: 42,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    subscribe_to_track(&mut relay, ALICE);
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(1),
        first: subscribe(2, b"b"),
    });
    // And the publisher is a subscriber as well, of a track "c" that Bob
    // publishes.
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"c"),
            track_alias: 43,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(2),
        first: subscribe(4, b"c"),
    });
    assert_eq!(held(&relay), (3, 3, 3, 0, 0));
    // "b" ends for Alice; "a" goes on with Bob; "c" loses its subscriber.
    assert_eq!(
        relay.step(Event::SessionClosed { session: PUBLISHER }),
        [
            Action::SendRequest {
                session: ALICE,
                stream: StreamKey(1),
                message: publish_done(0x2, 0),
            },
            Action::FinishRequest {
                session: ALICE,
                stream: StreamKey(1),
            },
        ]
    );
    // Left are "a" with Bob as its source and Alice subscribed, and "c"
    // with Bob as its source and nobody subscribed.
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (2, 2, 1)
    );
}

/// The publisher announced `(xmoq)`, Alice subscribed to `(xmoq)/a`, and the
/// publisher accepted the relay's SUBSCRIBE, its request 0, under
/// `UPSTREAM_ALIAS`. Then Bob sent PUBLISH for the track. So the track "a"
/// has two sources of the two kinds, and Alice is subscribed to it.
fn relay_with_a_subscribed_and_a_published_source() -> Relay {
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE, BOB] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 0,
            track_namespace: namespace(),
            parameters: vec![],
        }),
    });
    subscribe_to_track(&mut relay, ALICE);
    relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    let counts = relay.counts();
    assert_eq!((counts.tracks, counts.sources, counts.asks), (1, 1, 0));
    // Bob's PUBLISH for the track is accepted: a second source, of the other
    // kind. Nothing is asked of anyone.
    assert_eq!(publishes_a(&mut relay, BOB, BOBS_ALIAS), [publish_ok(BOB)]);
    let counts = relay.counts();
    assert_eq!((counts.tracks, counts.sources, counts.asks), (1, 2, 0));
    relay
}

#[test]
fn a_publish_joins_a_track_the_relay_subscribed_to_upstream() {
    let mut relay = relay_with_a_subscribed_and_a_published_source();

    // Alice, the last subscriber, cancels. The relay cancels the SUBSCRIBE
    // it sent the publisher, since nobody gets what that brings; the track
    // stays for Bob, who sends unasked.
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
        (counts.tracks, counts.sources, counts.own_requests),
        (1, 1, 0)
    );
}

#[test]
fn a_stream_of_a_group_that_is_too_old_to_tell_is_left_once_there_were_two_sources() {
    // The expectations below are spelled out for 8 Groups.
    assert_eq!(crate::MAX_REMEMBERED_GROUPS, 8);
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    // The publisher sends Groups 1 to 9, each whole. Group 1 is 8 behind the
    // newest then, and the relay no longer knows that it had it.
    for group in 1..=9 {
        stream_from(&mut relay, PUBLISHER, 20 + group, group);
        closed_from(&mut relay, PUBLISHER, 20 + group, StreamEnd::Fin);
    }
    // Bob is 8 Groups behind and sends Group 1. Alice has it already, on a
    // stream that was not reset, so it must not come again (§2.2): Bob's
    // stream is stopped, and nothing is opened for Alice.
    assert_eq!(stream_from(&mut relay, BOB, 5, 1), [stopped(BOB, 5)]);
    // A Group the relay remembers is told apart as before: Group 2 is 7
    // behind.
    assert_eq!(stream_from(&mut relay, BOB, 6, 2), [stopped(BOB, 6)]);
    // The publisher's own stream for the old Group is left as well: the
    // relay cannot tell that it was the publisher who sent it.
    assert_eq!(
        stream_from(&mut relay, PUBLISHER, 39, 1),
        [stopped(PUBLISHER, 39)]
    );
    // Bob's stopped stream counts among the streams of his subscription
    // (§10.11): with a Stream Count of 2 the relay waits for nothing, and
    // finishes his PUBLISH stream at once.
    for stream in [5, 6] {
        closed_from(&mut relay, BOB, stream, StreamEnd::Aborted(None));
    }
    assert_eq!(
        relay.step(Event::RequestMessage {
            session: BOB,
            stream: StreamKey(0),
            message: publish_done(0x2, 2),
        }),
        [Action::FinishRequest {
            session: BOB,
            stream: StreamKey(0),
        }]
    );
    // One source is left. An old Group that comes late is still left: the
    // track has had two sources, and what the other one sent is not known
    // any more.
    assert_eq!(relay.counts().sources, 1);
    assert_eq!(
        stream_from(&mut relay, PUBLISHER, 40, 1),
        [stopped(PUBLISHER, 40)]
    );
}

#[test]
fn the_source_that_is_left_does_not_bring_back_what_the_one_that_went_had_sent() {
    // The expectations below are spelled out for 8 Groups.
    assert_eq!(crate::MAX_REMEMBERED_GROUPS, 8);
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    // The publisher is ahead: it sends Groups 1 to 20, each whole, and then
    // its session ends. Bob, who is behind, is the one source left.
    for group in 1..=20 {
        stream_from(&mut relay, PUBLISHER, 20 + group, group);
        closed_from(&mut relay, PUBLISHER, 20 + group, StreamEnd::Fin);
    }
    relay.step(Event::SessionClosed { session: PUBLISHER });
    assert_eq!(relay.counts().sources, 1);
    // (the Group Bob sends next). Alice has all of them from the publisher,
    // on streams that ended with a FIN. Group 12 is 8 behind the newest and
    // forgotten, Group 13 is remembered; neither comes again.
    for (stream, group) in [(1, 6), (2, 12), (3, 13)] {
        assert_eq!(
            stream_from(&mut relay, BOB, stream, group),
            [stopped(BOB, stream)],
            "{group}"
        );
    }
    // What nobody has sent yet is Bob's to send: Alice's 21st stream.
    assert_eq!(
        stream_from(&mut relay, BOB, 4, 21),
        [open_data_stream(ALICE, 20, header_of_group(0, 21))]
    );
}

#[test]
fn a_publisher_publishes_anew_once_its_source_has_drained() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, BOB, 8, 2);
    // The publisher says PUBLISH_DONE and counted no stream: nothing is
    // awaited, and its source is let go at once.
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 0),
    });
    assert_eq!(relay.counts().sources, 1);
    // Its new PUBLISH, under another alias, is a source again.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: ControlMessage::Publish(Publish {
                request_id: 2,
                full_track_name: track(b"a"),
                track_alias: 42,
                parameters: vec![],
                track_properties: vec![],
            }),
        }),
        [Action::SendRequest {
            session: PUBLISHER,
            stream: StreamKey(1),
            message: ControlMessage::RequestOk(RequestOk {
                parameters: vec![Parameter::Forward(true)],
                track_properties: vec![],
            }),
        }]
    );
    assert_eq!(relay.counts().sources, 2);
    // What it sends now reaches Alice, and nothing of the source that is
    // gone is in its way.
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(9),
            header: header_of_group(42, 3),
        }),
        [open_data_stream(ALICE, 1, header_of_group(0, 3))]
    );
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 9, 0),
        [send_object(1, 0)]
    );
    assert_eq!(object_from(&mut relay, BOB, 8, 0), [send_object(0, 0)]);
}

#[test]
fn two_sources_that_chose_the_same_track_alias_are_told_apart() {
    let mut relay = relay_with_track();
    // Bob chose the alias the publisher chose; an alias means something in
    // its session only (§11.1).
    assert_eq!(
        publishes_a(&mut relay, BOB, UPSTREAM_ALIAS),
        [publish_ok(BOB)]
    );
    subscribe_to_track(&mut relay, ALICE);
    let from = |relay: &mut Relay, source, group| {
        relay.step(Event::DataStreamOpened {
            session: source,
            stream: StreamKey(7),
            header: header_of_group(UPSTREAM_ALIAS, group),
        })
    };
    assert_eq!(
        from(&mut relay, PUBLISHER, 1),
        [open_data_stream(ALICE, 0, header_of_group(0, 1))]
    );
    assert_eq!(
        from(&mut relay, BOB, 2),
        [open_data_stream(ALICE, 1, header_of_group(0, 2))]
    );
    // The publisher's timer for its alias ends the publisher's source, and
    // of Alice's streams the one that came from it.
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });
    assert_eq!(
        relay.step(Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
            wait: 0,
        }),
        [
            stopped(PUBLISHER, 7),
            Action::ResetRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
                code: 0x1,
            },
            reset(0, 0x1),
        ]
    );
    // Bob's stream 7 goes on.
    assert_eq!(object_from(&mut relay, BOB, 7, 0), [send_object(1, 0)]);
}

/// The ways the publisher of [`relay_with_a_subscribed_and_a_published_source`]
/// goes while Bob stays: a name, the events, and what the last of them leads
/// to. Its source is the relay's SUBSCRIBE, request 0. When it goes, Alice
/// has stream 0 from the publisher's stream 7, which is open, and stream 1
/// from Bob's stream 8.
fn ways_the_subscribed_source_goes() -> [(&'static str, Vec<Event>, Vec<Action>); 4] {
    let done = |stream_count| Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: publish_done(0x2, stream_count),
    };
    let cancelled = Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x1,
    };
    [
        (
            "it resets the relay's SUBSCRIBE",
            vec![Event::OwnRequestClosed {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                how: OwnRequestEnd::Reset,
            }],
            vec![stopped(PUBLISHER, 7), cancelled.clone(), reset(0, 0x1)],
        ),
        (
            "its session ends",
            vec![Event::SessionClosed { session: PUBLISHER }],
            vec![reset(0, 0x1)],
        ),
        (
            "PUBLISH_DONE, and the stream it counted ends",
            vec![
                done(1),
                Event::DataStreamClosed {
                    session: PUBLISHER,
                    stream: StreamKey(7),
                    end: StreamEnd::Fin,
                },
            ],
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
            "PUBLISH_DONE, and a stream it counted does not come",
            vec![
                done(2),
                Event::DrainTimeout {
                    session: PUBLISHER,
                    track_alias: UPSTREAM_ALIAS,
                    wait: 0,
                },
            ],
            vec![stopped(PUBLISHER, 7), cancelled, reset(0, 0x1)],
        ),
    ]
}

#[test]
fn a_source_the_relay_subscribed_to_that_goes_leaves_the_track_to_the_others() {
    for (name, events, expected) in ways_the_subscribed_source_goes() {
        let mut relay = relay_with_a_subscribed_and_a_published_source();
        stream_from(&mut relay, PUBLISHER, 7, 1);
        object_from(&mut relay, PUBLISHER, 7, 0);
        stream_from(&mut relay, BOB, 8, 2);
        let mut last = Vec::new();
        for event in events {
            last = relay.step(event);
        }
        assert_eq!(last, expected, "{name}");
        let counts = relay.counts();
        assert_eq!(
            (
                counts.tracks,
                counts.sources,
                counts.subscriptions,
                counts.own_requests
            ),
            (1, 1, 1, 0),
            "{name}"
        );
        assert_eq!(
            object_from(&mut relay, BOB, 8, 0),
            [send_object(1, 0)],
            "{name}"
        );
    }
}

#[test]
fn a_malformed_track_cancels_the_relays_subscribe_as_well() {
    let mut relay = relay_with_a_subscribed_and_a_published_source();
    stream_from(&mut relay, PUBLISHER, 7, 1);
    stream_from(&mut relay, BOB, 8, 2);
    // Bob broke the object model. The relay's SUBSCRIBE to the publisher is
    // cancelled with CANCELLED (0x1); what goes to Bob and to Alice carries
    // MALFORMED_TRACK (0x12).
    let mut expected = vec![
        stopped(PUBLISHER, 7),
        Action::CancelOwnRequest {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            code: 0x1,
        },
        Action::StopDataStream {
            session: BOB,
            stream: StreamKey(8),
            code: 0x12,
        },
        Action::ResetRequest {
            session: BOB,
            stream: StreamKey(0),
            code: 0x12,
        },
        reset(0, 0x12),
        reset(1, 0x12),
    ];
    expected.extend(done_actions(ALICE, 0x12, 2));
    assert_eq!(
        relay.step(Event::TrackMalformed {
            session: BOB,
            stream: StreamKey(8),
        }),
        expected
    );
}

#[test]
fn a_stream_of_another_source_is_left_when_its_first_object_names_the_subgroup() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, PUBLISHER, 7, 1);
    object_from(&mut relay, PUBLISHER, 7, 0);
    // Bob's header leaves the Subgroup ID to the first Object (§11.4.2), so
    // the relay knows only then that it is the publisher's Subgroup 0.
    let unnamed = SubgroupHeader {
        subgroup_id: SubgroupId::FirstObjectId,
        ..header_of_group(BOBS_ALIAS, 1)
    };
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: BOB,
            stream: StreamKey(8),
            header: unnamed,
        }),
        []
    );
    assert_eq!(object_from(&mut relay, BOB, 8, 0), [stopped(BOB, 8)]);
    // An Object 3 first would have named another Subgroup, which is Bob's.
    relay.step(Event::DataStreamOpened {
        session: BOB,
        stream: StreamKey(9),
        header: unnamed,
    });
    assert_eq!(
        object_from(&mut relay, BOB, 9, 3),
        [
            open_data_stream(ALICE, 1, unnamed_for_alice()),
            send_object(1, 3)
        ]
    );
}

/// What Alice gets for Bob's stream of Group 1 whose header leaves the
/// Subgroup ID to the first Object: the header as it came, under her alias.
fn unnamed_for_alice() -> SubgroupHeader {
    SubgroupHeader {
        subgroup_id: SubgroupId::FirstObjectId,
        ..header_of_group(0, 1)
    }
}

#[test]
fn nothing_is_left_when_everyone_has_gone() {
    let mut relay = relay_with_a_subscribed_and_a_published_source();
    stream_from(&mut relay, PUBLISHER, 7, 1);
    stream_from(&mut relay, BOB, 8, 2);
    for session in [PUBLISHER, BOB, ALICE] {
        relay.step(Event::SessionClosed { session });
    }
    assert_eq!(
        relay.counts(),
        crate::Counts {
            sessions: 0,
            tracks: 0,
            subscriptions: 0,
            requests: 0,
            request_streams: 0,
            half_closed_streams: 0,
            in_streams: 0,
            out_streams: 0,
            sources: 0,
            asks: 0,
            declined: 0,
            early_streams: 0,
            remembered_subgroups: 0,
            namespaces: 0,
            pending_tracks: 0,
            own_requests: 0,
            fetches: 0,
        }
    );
}

#[test]
fn a_track_whose_sources_drain_goes_on_with_a_publisher_that_comes() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, PUBLISHER, 7, 1);
    // The publisher says PUBLISH_DONE and counts two streams, of which one
    // has come: the track drains.
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });
    // Bob's PUBLISH is accepted, and the track is one a subscriber can come
    // to again (§9.4): Carol gets SUBSCRIBE_OK, with Bob's Track Properties.
    assert_eq!(publishes_a(&mut relay, BOB, BOBS_ALIAS), [publish_ok(BOB)]);
    let carol = SessionKey(4);
    relay.step(Event::SessionOpened {
        session: carol,
        role: Role::Server,
    });
    assert_eq!(
        subscribe_to_track(&mut relay, carol),
        [Action::SendRequest {
            session: carol,
            stream: StreamKey(0),
            message: ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 0,
                parameters: vec![],
                track_properties: vec![],
            }),
        }]
    );
    // Alice, who was there, gets what Bob sends, and so does Carol.
    assert_eq!(
        stream_from(&mut relay, BOB, 8, 2),
        [
            open_data_stream(ALICE, 1, header_of_group(0, 2)),
            open_data_stream(carol, 2, header_of_group(0, 2)),
        ]
    );
    // The publisher's second stream comes and ends, and then its first: the
    // publisher is let go, and nothing is said to the subscribers.
    stream_from(&mut relay, PUBLISHER, 9, 3);
    closed_from(&mut relay, PUBLISHER, 9, StreamEnd::Fin);
    let last = closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin);
    assert_eq!(
        last,
        [
            Action::FinishDataStream {
                out_stream: OutStreamKey(0),
            },
            Action::FinishRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
            },
        ]
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (1, 1, 2)
    );
}

#[test]
fn a_track_drains_again_when_the_publisher_that_came_goes_first() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, PUBLISHER, 7, 1);
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });
    // Bob comes to the draining track, and goes before the publisher's last
    // stream has come. Alice stays subscribed: the track still has the
    // publisher's source, which drains.
    assert_eq!(publishes_a(&mut relay, BOB, BOBS_ALIAS), [publish_ok(BOB)]);
    assert_eq!(relay.step(Event::SessionClosed { session: BOB }), []);
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (1, 1, 1)
    );
    // No Established source is left, so a subscriber that comes now is turned
    // away with DOES_NOT_EXIST (0x10), as before Bob came.
    let carol = SessionKey(4);
    relay.step(Event::SessionOpened {
        session: carol,
        role: Role::Server,
    });
    assert_eq!(
        subscribe_to_track(&mut relay, carol),
        [
            Action::SendRequest {
                session: carol,
                stream: StreamKey(0),
                message: request_error(0x10),
            },
            Action::FinishRequest {
                session: carol,
                stream: StreamKey(0),
            },
        ]
    );
    // The track ends when the wait for the publisher's streams is over.
    let mut expected = vec![
        stopped(PUBLISHER, 7),
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x1,
        },
        reset(0, 0x1),
    ];
    expected.extend(done_actions(ALICE, 0x2, 1));
    assert_eq!(
        relay.step(Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
            wait: 0,
        }),
        expected
    );
}

#[test]
fn a_publisher_that_takes_over_a_draining_track_has_to_go_on_with_its_groups() {
    // The expectations below are spelled out for 8 Groups.
    assert_eq!(crate::MAX_REMEMBERED_GROUPS, 8);
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // The publisher is at Group 50 when it says PUBLISH_DONE; one stream it
    // counted has not come, so the track drains.
    stream_from(&mut relay, PUBLISHER, 7, 50);
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });
    assert_eq!(publishes_a(&mut relay, BOB, BOBS_ALIAS), [publish_ok(BOB)]);
    // (the Group Bob sends). Bob counts from 0. Alice is subscribed to the
    // track as it was and has had Groups up to 50, so what is 8 or more
    // behind that is left; Bob is told nothing but the stop of each stream.
    for (stream, group) in [(1, 0), (2, 1), (3, 41), (4, 42)] {
        assert_eq!(
            stream_from(&mut relay, BOB, stream, group),
            [stopped(BOB, stream)],
            "{group}"
        );
    }
    // From Group 43 on Bob is heard: Alice's second stream.
    assert_eq!(
        stream_from(&mut relay, BOB, 5, 43),
        [open_data_stream(ALICE, 1, header_of_group(0, 43))]
    );
}

/// The publisher publishes the track "a" anew, on its stream 1 with Request
/// ID 2, under `track_alias`.
fn publishes_a_anew(relay: &mut Relay, track_alias: u64) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"a"),
            track_alias,
            parameters: vec![],
            track_properties: vec![],
        }),
    })
}

fn publish_anew_ok() -> Action {
    Action::SendRequest {
        session: PUBLISHER,
        stream: StreamKey(1),
        message: ControlMessage::RequestOk(RequestOk {
            parameters: vec![Parameter::Forward(true)],
            track_properties: vec![],
        }),
    }
}

/// A stream of the publisher under an alias of its own choosing.
fn stream_under(relay: &mut Relay, stream: u64, track_alias: u64, group: u64) -> Vec<Action> {
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream: StreamKey(stream),
        header: header_of_group(track_alias, group),
    })
}

/// `relay_with_track` with Alice subscribed, where the publisher sent one
/// stream (stream 7, Group 1, Alice's stream 0), said `PUBLISH_DONE` with a
/// Stream Count of 2, and so drains.
fn relay_with_a_draining_publisher() -> Relay {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, PUBLISHER, 7, 1);
    object_from(&mut relay, PUBLISHER, 7, 0);
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 2),
    });
    relay
}

#[test]
fn a_publisher_publishes_anew_under_another_alias_while_its_source_drains() {
    let mut relay = relay_with_a_draining_publisher();
    // For the publisher its subscription is over (§5.1), so it may publish
    // the track anew. Alice stays subscribed; the track has two sources of
    // the one session, told apart by their alias.
    assert_eq!(publishes_a_anew(&mut relay, 42), [publish_anew_ok()]);
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (1, 2, 1)
    );
    // A stream of the new subscription, and the late one of the old.
    assert_eq!(
        stream_under(&mut relay, 9, 42, 2),
        [open_data_stream(ALICE, 1, header_of_group(0, 2))]
    );
    assert_eq!(
        stream_under(&mut relay, 8, UPSTREAM_ALIAS, 3),
        [open_data_stream(ALICE, 2, header_of_group(0, 3))]
    );
    // The two streams the old subscription counted end: it has drained, and
    // its PUBLISH stream is finished. Nothing is said to Alice, and the
    // stream of the new subscription is not touched.
    closed_from(&mut relay, PUBLISHER, 8, StreamEnd::Fin);
    assert_eq!(
        closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin),
        [
            Action::FinishDataStream {
                out_stream: OutStreamKey(0),
            },
            Action::FinishRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
            },
        ]
    );
    assert_eq!(relay.counts().sources, 1);
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 9, 0),
        [send_object(1, 0)]
    );
    // The track never had a source of another session: an old Group that
    // comes late still passes (Group 2 is 18 behind Group 20).
    stream_under(&mut relay, 10, 42, 20);
    assert_eq!(
        stream_under(&mut relay, 11, 42, 1),
        [open_data_stream(ALICE, 4, header_of_group(0, 1))]
    );
}

#[test]
fn a_publisher_publishes_anew_under_the_same_alias_while_its_source_drains() {
    let mut relay = relay_with_a_draining_publisher();
    // The alias is the new subscription's from here on, and the old one is
    // over: the relay stops waiting for the stream it counted, which it
    // says with a reset (CANCELLED, 0x1) where a FIN would have said that
    // all of them came.
    assert_eq!(
        publishes_a_anew(&mut relay, UPSTREAM_ALIAS),
        [
            Action::ResetRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
                code: 0x1,
            },
            publish_anew_ok(),
        ]
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (1, 1, 1)
    );
    // The stream that was open goes on, as one of the new subscription.
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 7, 1),
        [send_object(0, 1)]
    );
    // It is cut short, and the publisher sends the Subgroup again: the same
    // source's second stream after a first that ended without a FIN passes.
    assert_eq!(
        closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2))),
        [reset(0, 0x2)]
    );
    assert_eq!(
        stream_under(&mut relay, 8, UPSTREAM_ALIAS, 1),
        [open_data_stream(ALICE, 1, header_of_group(0, 1))]
    );
    // The timer of the old subscription's wait finds nothing to end.
    assert_eq!(
        relay.step(Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
            wait: 0,
        }),
        []
    );
    assert_eq!(relay.counts().sources, 1);
}

#[test]
fn what_ends_one_of_two_sources_of_a_session_leaves_the_other() {
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, 42);
    stream_under(&mut relay, 9, 42, 2);
    // The wait for the old subscription's second stream runs out: its open
    // stream is stopped and Alice's stream for it reset. The new one stays.
    assert_eq!(
        relay.step(Event::DrainTimeout {
            session: PUBLISHER,
            track_alias: UPSTREAM_ALIAS,
            wait: 0,
        }),
        [
            stopped(PUBLISHER, 7),
            Action::ResetRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
                code: 0x1,
            },
            reset(0, 0x1),
        ]
    );
    assert_eq!(relay.counts().sources, 1);
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 9, 0),
        [send_object(1, 0)]
    );
    // Of the two streams, the one of the source that is left is held.
    let counts = relay.counts();
    assert_eq!((counts.sources, counts.in_streams), (1, 1));
}

#[test]
fn a_session_that_goes_takes_both_its_sources_of_a_track() {
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, 42);
    stream_under(&mut relay, 9, 42, 2);
    let mut expected = vec![reset(0, 0x1), reset(1, 0x1)];
    expected.extend(done_actions(ALICE, 0x2, 2));
    assert_eq!(
        relay.step(Event::SessionClosed { session: PUBLISHER }),
        expected
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.in_streams),
        (0, 0, 0)
    );
}

#[test]
fn the_two_sources_of_a_session_are_two_sources_for_a_subgroup() {
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, 42);
    // The new subscription sends Group 5 first; the late stream of the old
    // one for the same Subgroup is left, as a stream of another source is.
    assert_eq!(
        stream_under(&mut relay, 9, 42, 5),
        [open_data_stream(ALICE, 1, header_of_group(0, 5))]
    );
    assert_eq!(
        stream_under(&mut relay, 8, UPSTREAM_ALIAS, 5),
        [stopped(PUBLISHER, 8)]
    );
}

#[test]
fn a_source_that_drains_counts_against_the_limit() {
    // The expectations below are spelled out for 4 sources.
    assert_eq!(crate::MAX_SOURCES, 4);
    let mut relay = relay_with_a_draining_publisher();
    for session in [BOB, SessionKey(11), SessionKey(12), SessionKey(13)] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    for session in [BOB, SessionKey(11), SessionKey(12)] {
        publishes_a(&mut relay, session, BOBS_ALIAS);
    }
    assert_eq!(relay.counts().sources, 4);
    let overloaded = |session, stream| {
        [
            Action::SendRequest {
                session,
                stream: StreamKey(stream),
                message: request_error(0x9),
            },
            Action::FinishRequest {
                session,
                stream: StreamKey(stream),
            },
        ]
    };
    // A fifth publisher is refused, and so is the publisher that drains when
    // it publishes anew under another alias: EXCESSIVE_LOAD (0x9).
    assert_eq!(
        publishes_a(&mut relay, SessionKey(13), BOBS_ALIAS),
        overloaded(SessionKey(13), 0)
    );
    assert_eq!(publishes_a_anew(&mut relay, 42), overloaded(PUBLISHER, 1));
    assert_eq!(relay.counts().sources, 4);
    // Under the same alias it takes the place of its own source, which needs
    // no room.
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(2),
            first: ControlMessage::Publish(Publish {
                request_id: 4,
                full_track_name: track(b"a"),
                track_alias: UPSTREAM_ALIAS,
                parameters: vec![],
                track_properties: vec![],
            }),
        }),
        [
            Action::ResetRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
                code: 0x1,
            },
            Action::SendRequest {
                session: PUBLISHER,
                stream: StreamKey(2),
                message: ControlMessage::RequestOk(RequestOk {
                    parameters: vec![Parameter::Forward(true)],
                    track_properties: vec![],
                }),
            },
        ]
    );
    assert_eq!(relay.counts().sources, 4);
}

#[test]
fn an_alias_that_is_used_again_for_another_track_ends_the_source_that_had_it() {
    let mut relay = relay_with_a_draining_publisher();
    // The publisher uses the alias of its draining subscription to "a" for a
    // PUBLISH of "b". The alias means "b" from here on, so nothing more can
    // be told to be of "a": its source ends as when the wait ran out, and
    // with it the track, which had no other.
    let mut expected = vec![
        stopped(PUBLISHER, 7),
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x1,
        },
        reset(0, 0x1),
    ];
    expected.extend(done_actions(ALICE, 0x2, 1));
    expected.push(publish_anew_ok());
    assert_eq!(
        relay.step(Event::RequestOpened {
            session: PUBLISHER,
            stream: StreamKey(1),
            first: publish(2, b"b"),
        }),
        expected
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (1, 1, 0)
    );
}

#[test]
fn a_subscribe_ok_that_uses_the_alias_of_a_draining_source_ends_that_source() {
    let mut relay = relay_with_a_draining_publisher();
    // The publisher announces a namespace, Bob subscribes to "b" in it, and
    // the relay asks the publisher for "b".
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 2,
            track_namespace: namespace(),
            parameters: vec![],
        }),
    });
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: subscribe(0, b"b"),
    });
    // The publisher accepts, under the alias of its draining subscription to
    // "a". The alias means "b" from here on: the source of "a" ends as when
    // the wait ran out, and with it "a", before Bob is answered.
    let mut expected = vec![
        Action::StopUpstreamTimer {
            session: PUBLISHER,
            request: OwnRequestKey(0),
        },
        stopped(PUBLISHER, 7),
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x1,
        },
        reset(0, 0x1),
    ];
    expected.extend(done_actions(ALICE, 0x2, 1));
    expected.push(Action::SendRequest {
        session: BOB,
        stream: StreamKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 0,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    assert_eq!(
        relay.step(Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            message: ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: UPSTREAM_ALIAS,
                parameters: vec![],
                track_properties: vec![],
            }),
        }),
        expected
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.subscriptions),
        (1, 1, 1)
    );
    // The stream of "a" that was stopped ends. It is not counted for "b",
    // whose one stream is open when the publisher says PUBLISH_DONE.
    stream_under(&mut relay, 9, UPSTREAM_ALIAS, 3);
    closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x1)));
    assert_eq!(
        relay.step(Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            message: publish_done(0x2, 1),
        }),
        [drain_timer(1)]
    );
}

/// `PUBLISH_DONE` on the PUBLISH stream of `publishes_a_anew`.
fn done_anew(relay: &mut Relay, stream_count: u64) -> Vec<Action> {
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(1),
        message: publish_done(0x2, stream_count),
    })
}

fn drain_timeout(relay: &mut Relay, wait: u64) -> Vec<Action> {
    relay.step(Event::DrainTimeout {
        session: PUBLISHER,
        track_alias: UPSTREAM_ALIAS,
        wait,
    })
}

fn drain_timer(wait: u64) -> Action {
    Action::StartDrainTimer {
        session: PUBLISHER,
        track_alias: UPSTREAM_ALIAS,
        wait,
    }
}

#[test]
fn a_stream_of_the_subscription_before_is_not_counted_for_the_new_one() {
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, UPSTREAM_ALIAS);
    assert_eq!(
        stream_under(&mut relay, 9, UPSTREAM_ALIAS, 3),
        [open_data_stream(ALICE, 1, header_of_group(0, 3))]
    );
    // The stream that was open before the PUBLISH ends. The publisher does
    // not count it among the streams of the new subscription (§10.11), so
    // the one stream its PUBLISH_DONE counts is still to end.
    assert_eq!(
        closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin),
        [Action::FinishDataStream {
            out_stream: OutStreamKey(0),
        }]
    );
    assert_eq!(done_anew(&mut relay, 1), [drain_timer(1)]);
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 9, 0),
        [send_object(1, 0)]
    );
    let mut expected = vec![
        Action::FinishDataStream {
            out_stream: OutStreamKey(1),
        },
        Action::FinishRequest {
            session: PUBLISHER,
            stream: StreamKey(1),
        },
    ];
    expected.extend(done_actions(ALICE, 0x2, 2));
    assert_eq!(
        closed_from(&mut relay, PUBLISHER, 9, StreamEnd::Fin),
        expected
    );
}

#[test]
fn known_gap_a_late_stream_of_the_subscription_before_is_counted_for_the_new_one() {
    // A known gap, pinned. The first subscription counted two streams and
    // one had come. The second comes after the PUBLISH under the same alias,
    // and nothing tells it from a stream of the new subscription.
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, UPSTREAM_ALIAS);
    stream_under(&mut relay, 8, UPSTREAM_ALIAS, 2);
    stream_under(&mut relay, 9, UPSTREAM_ALIAS, 3);
    assert_eq!(done_anew(&mut relay, 1), [drain_timer(1)]);
    // Its end is taken for the one stream the new PUBLISH_DONE counts, and
    // the source is over while stream 9, which that was, is open.
    closed_from(&mut relay, PUBLISHER, 8, StreamEnd::Fin);
    let counts = relay.counts();
    assert_eq!((counts.tracks, counts.sources), (0, 0));
}

#[test]
fn the_timer_of_an_earlier_wait_does_not_end_a_later_one() {
    #[derive(Debug, Clone, Copy)]
    enum First {
        /// The first subscription still drained when the alias was used
        /// again, and the new one took its place.
        Replaced,
        /// It had drained, and its track was over.
        Drained,
    }
    for first in [First::Replaced, First::Drained] {
        let mut relay = match first {
            First::Replaced => relay_with_a_draining_publisher(),
            First::Drained => {
                let mut relay = relay_with_a_draining_publisher();
                closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin);
                stream_under(&mut relay, 8, UPSTREAM_ALIAS, 2);
                closed_from(&mut relay, PUBLISHER, 8, StreamEnd::Fin);
                assert_eq!(relay.counts().tracks, 0, "{first:?}");
                relay
            }
        };
        publishes_a_anew(&mut relay, UPSTREAM_ALIAS);
        stream_under(&mut relay, 9, UPSTREAM_ALIAS, 3);
        assert_eq!(done_anew(&mut relay, 5), [drain_timer(1)], "{first:?}");
        // The timer of the first wait cannot be stopped and runs out.
        assert_eq!(drain_timeout(&mut relay, 0), [], "{first:?}");
        assert_eq!(relay.counts().sources, 1, "{first:?}");
        drain_timeout(&mut relay, 1);
        assert_eq!(relay.counts().sources, 0, "{first:?}");
    }
}

#[test]
fn a_subgroup_that_was_cut_short_is_sent_again_under_the_alias_it_came_under() {
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, 42);
    assert_eq!(
        closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2))),
        [reset(0, 0x2)]
    );
    // Under the new alias it is the Subgroup of another source, which the
    // relay took from the first.
    assert_eq!(stream_under(&mut relay, 9, 42, 1), [stopped(PUBLISHER, 9)]);
    assert_eq!(
        stream_under(&mut relay, 10, UPSTREAM_ALIAS, 1),
        [open_data_stream(ALICE, 1, header_of_group(0, 1))]
    );
    // That lasts as long as the first source does. Once the wait for it is
    // over, the Subgroup is stopped under either alias, and the subscribers
    // do not get the rest of it.
    closed_from(&mut relay, PUBLISHER, 10, StreamEnd::Aborted(Some(0x2)));
    drain_timeout(&mut relay, 0);
    assert_eq!(relay.counts().sources, 1);
    assert_eq!(
        stream_under(&mut relay, 11, UPSTREAM_ALIAS, 1),
        [stopped(PUBLISHER, 11)]
    );
    assert_eq!(
        stream_under(&mut relay, 12, 42, 1),
        [stopped(PUBLISHER, 12)]
    );
}

#[test]
fn a_stream_of_the_subscription_before_ends_with_the_source_that_took_its_alias() {
    let mut relay = relay_with_a_draining_publisher();
    publishes_a_anew(&mut relay, UPSTREAM_ALIAS);
    stream_under(&mut relay, 9, UPSTREAM_ALIAS, 3);
    assert_eq!(done_anew(&mut relay, 1), [drain_timer(1)]);
    // The one stream the new subscription counted ends, and the source is
    // over: its PUBLISH stream is finished, since all it counted came.
    // Stream 7, which the subscription before had open and the new one took
    // over, is not waited for, as it was not counted: it is stopped, and
    // Alice's stream for it reset.
    let mut expected = vec![
        Action::FinishDataStream {
            out_stream: OutStreamKey(1),
        },
        stopped(PUBLISHER, 7),
        Action::FinishRequest {
            session: PUBLISHER,
            stream: StreamKey(1),
        },
        reset(0, 0x1),
    ];
    expected.extend(done_actions(ALICE, 0x2, 2));
    assert_eq!(
        closed_from(&mut relay, PUBLISHER, 9, StreamEnd::Fin),
        expected
    );
}

#[test]
fn a_stream_of_a_track_that_lost_its_alias_is_not_counted_for_the_track_that_has_it() {
    let mut relay = relay_with_a_draining_publisher();
    // "b" under the alias of the draining "a": "a" ends, and its open stream
    // 7 is stopped.
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: publish(2, b"b"),
    });
    stream_under(&mut relay, 9, UPSTREAM_ALIAS, 3);
    closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x1)));
    // "b" counts one stream, which is open: the end of 7 was not it.
    assert_eq!(done_anew(&mut relay, 1), [drain_timer(1)]);
    assert_eq!(relay.counts().sources, 1);
}

#[test]
fn a_malformed_track_is_held_against_the_source_whose_alias_the_stream_carries() {
    // (the stream that is malformed; the code for the PUBLISH stream of the
    //  subscription that drains, stream 0; the one for the new one, stream
    //  1). MALFORMED_TRACK is 0x12, CANCELLED 0x1.
    for (malformed, to_the_first, to_the_second) in [(7, 0x12, 0x1), (9, 0x1, 0x12)] {
        let mut relay = relay_with_a_draining_publisher();
        publishes_a_anew(&mut relay, 42);
        stream_under(&mut relay, 9, 42, 2);
        let actions = relay.step(Event::TrackMalformed {
            session: PUBLISHER,
            stream: StreamKey(malformed),
        });
        let to_the_publisher: Vec<&Action> = actions
            .iter()
            .filter(|action| matches!(action, Action::ResetRequest { .. }))
            .collect();
        assert_eq!(
            to_the_publisher,
            [
                &Action::ResetRequest {
                    session: PUBLISHER,
                    stream: StreamKey(0),
                    code: to_the_first,
                },
                &Action::ResetRequest {
                    session: PUBLISHER,
                    stream: StreamKey(1),
                    code: to_the_second,
                },
            ],
            "stream {malformed}"
        );
        assert_eq!(relay.counts().tracks, 0, "stream {malformed}");
    }
}

#[test]
fn a_publish_takes_the_place_of_a_draining_subscription_of_the_relay() {
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 0,
            track_namespace: namespace(),
            parameters: vec![],
        }),
    });
    // The relay asks the publisher for Alice, and the publisher accepts,
    // sends a stream and says PUBLISH_DONE for two.
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: subscribe(0, b"a"),
    });
    relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    stream_from(&mut relay, PUBLISHER, 7, 1);
    assert_eq!(
        relay.step(Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            message: publish_done(0x2, 2),
        }),
        [drain_timer(0)]
    );
    // It publishes the track under the same alias: the relay's SUBSCRIBE is
    // cancelled where the PUBLISH stream of a publisher is reset.
    assert_eq!(
        publishes_a_anew(&mut relay, UPSTREAM_ALIAS),
        [
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code: 0x1,
            },
            publish_anew_ok(),
        ]
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.own_requests),
        (1, 1, 0)
    );
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 7, 0),
        [send_object(0, 0)]
    );
}

#[test]
fn a_subscribe_ok_the_track_cannot_take_still_ends_the_source_that_had_its_alias() {
    assert_eq!(crate::MAX_SOURCES, 4);
    let mut relay = relay_with_a_draining_publisher();
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 2,
            track_namespace: namespace(),
            parameters: vec![],
        }),
    });
    // The relay asks the publisher for "b", and "b" gets all the sources it
    // may have from four others before the answer comes.
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: subscribe(0, b"b"),
    });
    for session in [11, 12, 13, 14].map(SessionKey) {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
        relay.step(Event::RequestOpened {
            session,
            stream: StreamKey(0),
            first: publish(0, b"b"),
        });
    }
    // The answer uses the alias of the draining subscription to "a". The
    // session has given the alias to the new subscription, which the relay
    // cancels; "a" can get nothing more under it and ends.
    let mut expected = vec![
        Action::StopUpstreamTimer {
            session: PUBLISHER,
            request: OwnRequestKey(0),
        },
        stopped(PUBLISHER, 7),
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(0),
            code: 0x1,
        },
        reset(0, 0x1),
    ];
    expected.extend(done_actions(ALICE, 0x2, 1));
    expected.push(Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x1,
    });
    assert_eq!(
        relay.step(Event::OwnRequestMessage {
            session: PUBLISHER,
            request: OwnRequestKey(0),
            message: ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: UPSTREAM_ALIAS,
                parameters: vec![],
                track_properties: vec![],
            }),
        }),
        expected
    );
    let counts = relay.counts();
    assert_eq!((counts.tracks, counts.sources), (1, 4));
}

#[test]
fn a_stream_of_a_session_that_is_no_source_of_the_track_is_stopped() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    stream_from(&mut relay, BOB, 8, 2);
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 3),
    });
    // Bob's stream is malformed and the track ends, while the publisher's
    // alias still drains in its session. Another publisher makes a track of
    // the name.
    relay.step(Event::TrackMalformed {
        session: BOB,
        stream: StreamKey(8),
    });
    relay.step(Event::SessionOpened {
        session: SessionKey(11),
        role: Role::Server,
    });
    publishes_a(&mut relay, SessionKey(11), BOBS_ALIAS);
    // A late stream of the publisher is of the track that ended.
    assert_eq!(
        stream_under(&mut relay, 9, UPSTREAM_ALIAS, 5),
        [stopped(PUBLISHER, 9)]
    );
    let counts = relay.counts();
    assert_eq!((counts.in_streams, counts.remembered_subgroups), (0, 0));
}

/// DEFAULT PUBLISHER PRIORITY (§12.4), a Track Property.
const DEFAULT_PUBLISHER_PRIORITY: u64 = 0x0e;

/// A relay where the publisher and Bob publish the track "a", each with a
/// DEFAULT PUBLISHER PRIORITY of its own, or with none.
fn relay_with_default_priorities(publishers: Option<u64>, bobs: Option<u64>) -> Relay {
    let property = |priority: Option<u64>| -> Vec<KeyValuePair> {
        priority
            .map(|priority| KeyValuePair::varint(DEFAULT_PUBLISHER_PRIORITY, priority).unwrap())
            .into_iter()
            .collect()
    };
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE, BOB] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    for (session, alias, priority) in [
        (PUBLISHER, UPSTREAM_ALIAS, publishers),
        (BOB, BOBS_ALIAS, bobs),
    ] {
        relay.step(Event::RequestOpened {
            session,
            stream: StreamKey(0),
            first: publish_of_a(alias, vec![], property(priority)),
        });
    }
    relay
}

#[test]
fn known_gap_a_subgroup_without_a_priority_reaches_the_subscriber_under_another_default() {
    // A known gap, pinned. The publisher's default is 10 and Bob's 200.
    let mut relay = relay_with_default_priorities(Some(10), Some(200));
    // The subscriber is told the Track Properties of the source that came
    // last (§2.5), so a default of 200.
    assert_eq!(
        subscribe_to_track(&mut relay, ALICE),
        [Action::SendRequest {
            session: ALICE,
            stream: StreamKey(0),
            message: ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: 0,
                parameters: vec![],
                track_properties: vec![
                    KeyValuePair::varint(DEFAULT_PUBLISHER_PRIORITY, 200).unwrap()
                ],
            }),
        }]
    );
    // A Subgroup of the publisher that leaves its priority to the default
    // (§11.4.2) has priority 10. Its header goes on as it came, and to the
    // subscriber it now says 200.
    let inherits = |track_alias| SubgroupHeader {
        publisher_priority: None,
        ..header_of_group(track_alias, 1)
    };
    assert_eq!(
        relay.step(Event::DataStreamOpened {
            session: PUBLISHER,
            stream: StreamKey(7),
            header: inherits(UPSTREAM_ALIAS),
        }),
        [open_data_stream(ALICE, 0, inherits(0))]
    );
}

/// A header of Subgroup 0 of `group` with that Publisher Priority; `None`
/// leaves it to the default of the subscription (§11.4.2).
fn with_priority(track_alias: u64, group: u64, priority: Option<u8>) -> SubgroupHeader {
    SubgroupHeader {
        publisher_priority: priority,
        ..header_of_group(track_alias, group)
    }
}

fn opens(
    relay: &mut Relay,
    session: SessionKey,
    stream: u64,
    header: SubgroupHeader,
) -> Vec<Action> {
    relay.step(Event::DataStreamOpened {
        session,
        stream: StreamKey(stream),
        header,
    })
}

fn malformed_stop(session: SessionKey, stream: u64) -> Action {
    Action::StopDataStream {
        session,
        stream: StreamKey(stream),
        code: 0x12,
    }
}

fn publish_reset(session: SessionKey, code: u32) -> Action {
    Action::ResetRequest {
        session,
        stream: StreamKey(0),
        code,
    }
}

#[test]
fn a_subgroup_that_another_source_sends_with_another_priority_makes_the_track_malformed() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, Some(9)),
    );
    // §2.4.2, the first example. The stream that brought the other priority
    // is stopped with MALFORMED_TRACK (0x12), and its source is cancelled
    // with that code; the source that sent the Subgroup first did nothing
    // the relay can hold against it, and is cancelled with CANCELLED (0x1).
    // Alice's stream is reset and her subscription ends, MALFORMED_TRACK.
    let mut expected = vec![
        malformed_stop(BOB, 8),
        stopped(PUBLISHER, 7),
        publish_reset(PUBLISHER, 0x1),
        publish_reset(BOB, 0x12),
        reset(0, 0x12),
    ];
    expected.extend(done_actions(ALICE, 0x12, 1));
    assert_eq!(
        opens(&mut relay, BOB, 8, with_priority(BOBS_ALIAS, 1, Some(8))),
        expected
    );
    let counts = relay.counts();
    assert_eq!(
        (counts.tracks, counts.sources, counts.in_streams),
        (0, 0, 0)
    );
}

#[test]
fn a_subgroup_that_comes_again_with_another_priority_makes_the_track_malformed() {
    #[derive(Debug, Clone, Copy)]
    enum First {
        StillOpen,
        CutShort,
        Finished,
    }
    // (how the publisher's first stream for the Subgroup stands; who sends
    //  the second, with priority 8 where the first had 9). The track ends in
    //  all six, also where the second stream would have been left unread or
    //  would have taken the place of the first. The source of the second
    //  stream gets MALFORMED_TRACK (0x12) on its PUBLISH stream and the
    //  other CANCELLED (0x1).
    let cases = [
        (First::StillOpen, PUBLISHER, (0x12, 0x1)),
        (First::CutShort, PUBLISHER, (0x12, 0x1)),
        (First::Finished, PUBLISHER, (0x12, 0x1)),
        (First::StillOpen, BOB, (0x1, 0x12)),
        (First::CutShort, BOB, (0x1, 0x12)),
        (First::Finished, BOB, (0x1, 0x12)),
    ];
    for (first, second, (to_the_publisher, to_bob)) in cases {
        let mut relay = relay_with_two_publishers();
        subscribe_to_track(&mut relay, ALICE);
        opens(
            &mut relay,
            PUBLISHER,
            7,
            with_priority(UPSTREAM_ALIAS, 1, Some(9)),
        );
        match first {
            First::StillOpen => {}
            First::CutShort => {
                closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
            }
            First::Finished => {
                closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin);
            }
        }
        let alias = if second == BOB {
            BOBS_ALIAS
        } else {
            UPSTREAM_ALIAS
        };
        let actions = opens(&mut relay, second, 8, with_priority(alias, 1, Some(8)));
        let resets: Vec<&Action> = actions
            .iter()
            .filter(|action| matches!(action, Action::ResetRequest { .. }))
            .collect();
        assert_eq!(
            resets,
            [
                &publish_reset(PUBLISHER, to_the_publisher),
                &publish_reset(BOB, to_bob)
            ],
            "{first:?} {second:?}"
        );
        assert_eq!(
            actions.first(),
            Some(&malformed_stop(second, 8)),
            "{first:?} {second:?}"
        );
        assert_eq!(relay.counts().tracks, 0, "{first:?} {second:?}");
    }
}

#[test]
fn the_priority_of_a_subgroup_is_the_one_in_its_header_or_the_default_of_its_source() {
    // (the DEFAULT PUBLISHER PRIORITY of the publisher's PUBLISH and the
    //  priority in its header; the same for Bob, who sends the Subgroup
    //  second; whether the track is malformed). §12.4: without the property
    //  the default is 128.
    let cases = [
        // The same priority, written two ways: no difference.
        (None, None, None, Some(128), false),
        (Some(10), None, None, Some(10), false),
        (Some(10), None, Some(10), None, false),
        (None, Some(9), None, Some(9), false),
        // Both leave it to their default, and the defaults differ.
        (Some(10), None, Some(200), None, true),
        (None, None, Some(200), None, true),
        (None, Some(9), None, Some(8), true),
        // A header overrides the default (§12.4).
        (Some(10), Some(9), Some(200), Some(9), false),
        // 300 is no priority (§12.4), and counts as no default: 128.
        (Some(300), None, None, Some(128), false),
        (Some(300), None, None, Some(44), true),
    ];
    for (publishers_default, publishers, bobs_default, bobs, malformed) in cases {
        let case = format!("{publishers_default:?} {publishers:?} {bobs_default:?} {bobs:?}");
        let mut relay = relay_with_default_priorities(publishers_default, bobs_default);
        subscribe_to_track(&mut relay, ALICE);
        opens(
            &mut relay,
            PUBLISHER,
            7,
            with_priority(UPSTREAM_ALIAS, 1, publishers),
        );
        let actions = opens(&mut relay, BOB, 8, with_priority(BOBS_ALIAS, 1, bobs));
        if malformed {
            assert_eq!(actions.first(), Some(&malformed_stop(BOB, 8)), "{case}");
            assert_eq!(relay.counts().tracks, 0, "{case}");
        } else {
            // Left like any Subgroup another source sent first; the header
            // the subscriber got is the publisher's, as it came.
            assert_eq!(actions, [stopped(BOB, 8)], "{case}");
            assert_eq!(relay.counts().tracks, 1, "{case}");
        }
    }
}

#[test]
fn another_priority_is_found_when_the_first_object_names_the_subgroup() {
    let mut relay = relay_with_two_publishers();
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, Some(9)),
    );
    // Bob's header leaves the Subgroup ID to the first Object (§11.4.2), so
    // which Subgroup the priority 8 is for is known only then.
    let unnamed = SubgroupHeader {
        subgroup_id: SubgroupId::FirstObjectId,
        ..with_priority(BOBS_ALIAS, 1, Some(8))
    };
    assert_eq!(opens(&mut relay, BOB, 8, unnamed), []);
    let mut expected = vec![
        stopped(PUBLISHER, 7),
        publish_reset(PUBLISHER, 0x1),
        malformed_stop(BOB, 8),
        publish_reset(BOB, 0x12),
        reset(0, 0x12),
    ];
    expected.extend(done_actions(ALICE, 0x12, 1));
    assert_eq!(object_from(&mut relay, BOB, 8, 0), expected);
}

#[test]
fn one_publisher_that_sends_a_subgroup_again_with_another_priority_makes_its_track_malformed() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, Some(9)),
    );
    closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
    // With the priority it had, the Subgroup that was cut short passes.
    assert_eq!(
        opens(
            &mut relay,
            PUBLISHER,
            8,
            with_priority(UPSTREAM_ALIAS, 1, Some(9))
        ),
        [open_data_stream(ALICE, 1, with_priority(0, 1, Some(9)))]
    );
    closed_from(&mut relay, PUBLISHER, 8, StreamEnd::Aborted(Some(0x2)));
    let mut expected = vec![malformed_stop(PUBLISHER, 9), publish_reset(PUBLISHER, 0x12)];
    expected.extend(done_actions(ALICE, 0x12, 2));
    assert_eq!(
        opens(
            &mut relay,
            PUBLISHER,
            9,
            with_priority(UPSTREAM_ALIAS, 1, Some(8))
        ),
        expected
    );
}

#[test]
fn known_gap_a_priority_is_not_compared_for_a_subgroup_the_relay_has_forgotten() {
    // A known gap, pinned: the relay remembers the 8 newest Groups.
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, Some(9)),
    );
    closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin);
    opens(
        &mut relay,
        PUBLISHER,
        8,
        with_priority(UPSTREAM_ALIAS, 20, Some(9)),
    );
    assert_eq!(
        opens(
            &mut relay,
            PUBLISHER,
            9,
            with_priority(UPSTREAM_ALIAS, 1, Some(8))
        ),
        [open_data_stream(ALICE, 2, with_priority(0, 1, Some(8)))]
    );
}

/// `relay_with_default_priorities` with a default of 10 for the publisher,
/// where Alice subscribed, the publisher sent `first` and whatever `then`
/// does, and said `PUBLISH_DONE` for three streams, so that it drains.
fn relay_with_a_draining_default_of_10(then: impl Fn(&mut Relay)) -> Relay {
    let mut relay = relay_with_default_priorities(Some(10), None);
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, None),
    );
    then(&mut relay);
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 3),
    });
    relay
}

/// The publisher publishes "a" anew under the same alias, with a DEFAULT
/// PUBLISHER PRIORITY of 200.
fn publishes_anew_with_a_default_of_200(relay: &mut Relay) {
    publishes_anew_under(relay, UPSTREAM_ALIAS);
}

#[test]
fn a_stream_has_the_default_priority_of_the_subscription_it_came_on() {
    // (the priority in the header of the stream that delivered Subgroup 0
    //  of Group 1 whole, on a subscription with a default of 10; whether
    //  the track is malformed). The second stream for the Subgroup has no
    //  priority, and opens while that subscription goes on: its priority is
    //  10, and stays 10 when the alias goes to a subscription with a default
    //  of 200 before the Object comes that names its Subgroup (§11.4.2).
    //  10 and 10 are the same; 200 and 10 are not, though 200 is the
    //  default of the subscription that has the alias by then.
    for (first, malformed) in [(None, false), (Some(200), true)] {
        let mut relay = relay_with_default_priorities(Some(10), None);
        subscribe_to_track(&mut relay, ALICE);
        opens(
            &mut relay,
            PUBLISHER,
            7,
            with_priority(UPSTREAM_ALIAS, 1, first),
        );
        closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Fin);
        let unnamed = SubgroupHeader {
            subgroup_id: SubgroupId::FirstObjectId,
            ..with_priority(UPSTREAM_ALIAS, 1, None)
        };
        assert_eq!(opens(&mut relay, PUBLISHER, 8, unnamed), [], "{first:?}");
        relay.step(Event::RequestMessage {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: publish_done(0x2, 3),
        });
        publishes_anew_with_a_default_of_200(&mut relay);
        let actions = object_from(&mut relay, PUBLISHER, 8, 0);
        if malformed {
            assert_eq!(relay.counts().tracks, 0, "{first:?}");
        } else {
            // Left like any second stream for a Subgroup that was delivered
            // whole.
            assert_eq!(actions, [stopped(PUBLISHER, 8)], "{first:?}");
            assert_eq!(relay.counts().tracks, 1, "{first:?}");
        }
    }
}

#[test]
fn a_subgroup_sent_again_under_a_new_default_priority_makes_the_track_malformed() {
    // The stream of Subgroup 0 of Group 1, with the default of 10, is open
    // when the publisher publishes anew under the same alias with a default
    // of 200, and goes on as a stream of the new subscription.
    let mut relay = relay_with_a_draining_default_of_10(|_| {});
    publishes_anew_with_a_default_of_200(&mut relay);
    // The publisher sends the Subgroup again and leaves the priority to the
    // default, now 200: the Subgroup has two priorities (§2.4.2), and the
    // track ends. Bob, the other source, is cancelled.
    let mut expected = vec![
        malformed_stop(PUBLISHER, 9),
        malformed_stop(PUBLISHER, 7),
        Action::ResetRequest {
            session: PUBLISHER,
            stream: StreamKey(1),
            code: 0x12,
        },
        publish_reset(BOB, 0x1),
        reset(0, 0x12),
    ];
    expected.extend(done_actions(ALICE, 0x12, 1));
    assert_eq!(
        opens(
            &mut relay,
            PUBLISHER,
            9,
            with_priority(UPSTREAM_ALIAS, 1, None)
        ),
        expected
    );
}

/// The publisher publishes "a" anew on its stream 1, with a DEFAULT
/// PUBLISHER PRIORITY of 200.
fn publishes_anew_under(relay: &mut Relay, track_alias: u64) {
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(1),
        first: ControlMessage::Publish(Publish {
            request_id: 2,
            full_track_name: track(b"a"),
            track_alias,
            parameters: vec![],
            track_properties: vec![KeyValuePair::varint(DEFAULT_PUBLISHER_PRIORITY, 200).unwrap()],
        }),
    });
}

#[test]
fn a_subgroup_that_was_cut_short_is_sent_again_across_a_new_default_priority() {
    // Subgroup 0 of Group 1 came with the default of 10 and was cut short.
    // The publisher sends it again on a stream that has no priority either
    // and whose first Object will name the Subgroup (§11.4.2), says
    // PUBLISH_DONE, and publishes anew under the alias with a default of
    // 200. Then the Object comes.
    let unnamed = |track_alias| SubgroupHeader {
        subgroup_id: SubgroupId::FirstObjectId,
        ..with_priority(track_alias, 1, None)
    };
    let mut relay = relay_with_a_draining_default_of_10(|relay| {
        closed_from(relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
        opens(relay, PUBLISHER, 8, unnamed(UPSTREAM_ALIAS));
    });
    publishes_anew_with_a_default_of_200(&mut relay);
    // The stream is of the first subscription: priority 10, as before, and
    // the Subgroup goes on (§2.2).
    assert_eq!(
        object_from(&mut relay, PUBLISHER, 8, 0),
        [open_data_stream(ALICE, 1, unnamed(0)), send_object(1, 0)]
    );
}

#[test]
fn a_stream_without_a_priority_under_an_alias_that_drains_is_not_compared() {
    // Subgroup 0 of Group 1 came with 200 in its header, on a subscription
    // with a default of 10, and was cut short.
    let mut relay = relay_with_default_priorities(Some(10), None);
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, Some(200)),
    );
    closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 3),
    });
    // The publisher publishes anew with a default of 200 and sends the
    // Subgroup again without a priority; the stream is read before the
    // PUBLISH (§5.1). Under the subscription that drains it would have
    // priority 10, which it has not. The relay does not know, and passes it
    // as the same source's second stream for a Subgroup that was cut short.
    assert_eq!(
        opens(
            &mut relay,
            PUBLISHER,
            9,
            with_priority(UPSTREAM_ALIAS, 1, None)
        ),
        [open_data_stream(ALICE, 1, with_priority(0, 1, None))]
    );
    publishes_anew_with_a_default_of_200(&mut relay);
    assert_eq!(relay.counts().tracks, 1);
}

#[test]
fn known_gap_a_late_stream_is_read_with_the_default_of_the_subscription_that_took_its_alias() {
    // Subgroup 0 of Group 1 came with the default of 10 and was cut short;
    // the alias went to a subscription with a default of 200.
    let mut relay = relay_with_a_draining_default_of_10(|relay| {
        closed_from(relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
    });
    publishes_anew_with_a_default_of_200(&mut relay);
    // A stream of the first subscription comes late and sends the Subgroup
    // again, with priority 10 as before. Nothing tells it from a stream of
    // the new subscription, so the relay reads 200, and ends a track that
    // nothing is wrong with.
    let actions = opens(
        &mut relay,
        PUBLISHER,
        9,
        with_priority(UPSTREAM_ALIAS, 1, None),
    );
    assert_eq!(actions.first(), Some(&malformed_stop(PUBLISHER, 9)));
    assert_eq!(relay.counts().tracks, 0);
}

#[test]
fn each_of_the_two_sources_of_a_session_has_its_own_default_priority() {
    // (the priority in the header of the stream of the first subscription,
    //  whose default is 10; whether a stream without a priority under the
    //  alias of the second, whose default is 200, makes the track malformed).
    for (first, malformed) in [(Some(200), false), (None, true)] {
        let mut relay = relay_with_default_priorities(Some(10), None);
        subscribe_to_track(&mut relay, ALICE);
        opens(
            &mut relay,
            PUBLISHER,
            7,
            with_priority(UPSTREAM_ALIAS, 1, first),
        );
        relay.step(Event::RequestMessage {
            session: PUBLISHER,
            stream: StreamKey(0),
            message: publish_done(0x2, 3),
        });
        publishes_anew_under(&mut relay, 42);
        let actions = opens(&mut relay, PUBLISHER, 9, with_priority(42, 1, None));
        if malformed {
            assert_eq!(
                actions.first(),
                Some(&malformed_stop(PUBLISHER, 9)),
                "{first:?}"
            );
        } else {
            // Another source's Subgroup, with the same priority: left.
            assert_eq!(actions, [stopped(PUBLISHER, 9)], "{first:?}");
        }
    }
}

#[test]
fn a_publisher_that_accepted_the_relays_subscribe_has_the_default_priority_it_gave_then() {
    let mut relay = Relay::new("test/0");
    for session in [PUBLISHER, ALICE, BOB] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 0,
            track_namespace: namespace(),
            parameters: vec![],
        }),
    });
    subscribe_to_track(&mut relay, ALICE);
    // The publisher accepts with a default of 10 (§12.4); Bob publishes the
    // track without one, which is 128.
    relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![],
            track_properties: vec![KeyValuePair::varint(DEFAULT_PUBLISHER_PRIORITY, 10).unwrap()],
        }),
    });
    publishes_a(&mut relay, BOB, BOBS_ALIAS);
    // 10 by the default, and 10 in Bob's header: the same.
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, None),
    );
    assert_eq!(
        opens(&mut relay, BOB, 8, with_priority(BOBS_ALIAS, 1, Some(10))),
        [stopped(BOB, 8)]
    );
    // 128 by Bob's default, and 10 by the publisher's: the track is
    // malformed, and the relay's SUBSCRIBE is cancelled with that code.
    opens(&mut relay, BOB, 9, with_priority(BOBS_ALIAS, 2, None));
    let actions = opens(
        &mut relay,
        PUBLISHER,
        10,
        with_priority(UPSTREAM_ALIAS, 2, None),
    );
    assert_eq!(actions.first(), Some(&malformed_stop(PUBLISHER, 10)));
    assert!(actions.contains(&Action::CancelOwnRequest {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        code: 0x12,
    }));
    assert_eq!(relay.counts().tracks, 0);
}

#[test]
fn known_gap_a_stream_sent_ahead_of_a_publish_is_read_before_the_publish_done() {
    // Subgroup 0 of Group 1 came with 200 in its header, on a subscription
    // with a default of 10, and was cut short. The publisher says
    // PUBLISH_DONE, publishes anew under the alias with a default of 200,
    // and sends the Subgroup again without a priority, which is 200 as
    // before. The three travel on three streams, and the relay reads the
    // last of them first: to it the subscription goes on, the stream has
    // its default of 10, and the track is malformed, though the publisher
    // sent the Subgroup with 200 both times. Nothing tells this from one
    // subscription that sends a Subgroup again with another priority.
    let mut relay = relay_with_default_priorities(Some(10), None);
    subscribe_to_track(&mut relay, ALICE);
    opens(
        &mut relay,
        PUBLISHER,
        7,
        with_priority(UPSTREAM_ALIAS, 1, Some(200)),
    );
    closed_from(&mut relay, PUBLISHER, 7, StreamEnd::Aborted(Some(0x2)));
    let actions = opens(
        &mut relay,
        PUBLISHER,
        9,
        with_priority(UPSTREAM_ALIAS, 1, None),
    );
    assert_eq!(actions.first(), Some(&malformed_stop(PUBLISHER, 9)));
    assert_eq!(relay.counts().tracks, 0);
}
