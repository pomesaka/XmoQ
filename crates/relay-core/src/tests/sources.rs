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
            Action::StartRequestTimer {
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
fn a_publisher_whose_source_drains_cannot_become_a_second_source() {
    let mut relay = relay_with_two_publishers();
    // The publisher says PUBLISH_DONE and counts a stream that has not come.
    // The track goes on with Bob; the publisher's source drains.
    relay.step(Event::RequestMessage {
        session: PUBLISHER,
        stream: StreamKey(0),
        message: publish_done(0x2, 1),
    });
    assert_eq!(relay.counts().sources, 2);
    // For the publisher its subscription is over (§5.1), so it may publish
    // the track anew, under another alias. The relay still waits for the
    // stream of the first one, and takes no second source of the session.
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
        [
            Action::SendRequest {
                session: PUBLISHER,
                stream: StreamKey(1),
                message: request_error(0x20),
            },
            Action::FinishRequest {
                session: PUBLISHER,
                stream: StreamKey(1),
            },
        ]
    );
    assert_eq!(relay.counts().sources, 2);
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
            early_streams: 0,
            remembered_subgroups: 0,
            namespaces: 0,
            pending_tracks: 0,
            own_requests: 0,
        }
    );
}
