//! The Subscription Filter of a SUBSCRIBE (§5.1.2): which Objects the relay
//! sends a subscriber.

use xmoq_moqt::message::{Fetch, FetchRange};
use xmoq_moqt::params::SubscriptionFilter;

use super::*;
use crate::routing::{filter_end_group, filter_start};

fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

const LARGEST: Option<SubscriptionFilter> = Some(SubscriptionFilter::LargestObject);
const NEXT_GROUP: Option<SubscriptionFilter> = Some(SubscriptionFilter::NextGroupStart);

pub(super) fn from(group: u64, object: u64) -> SubscriptionFilter {
    SubscriptionFilter::AbsoluteStart {
        start: at(group, object),
    }
}

#[test]
fn the_start_location_of_a_filter_is_counted_from_the_largest_object() {
    let last = u64::MAX;
    let range = Some(SubscriptionFilter::AbsoluteRange {
        start: at(4, 1),
        end_group_delta: 2,
    });
    // (the filter; the Largest Object when the SUBSCRIBE is accepted; the
    //  first Location that passes, `None` where nothing does)
    let cases = [
        // Unfiltered: everything.
        (None, None, Some(at(0, 0))),
        (None, Some(at(3, 1)), Some(at(0, 0))),
        // Largest Object: the Object after it.
        (LARGEST, Some(at(3, 1)), Some(at(3, 2))),
        (LARGEST, Some(at(0, 0)), Some(at(0, 1))),
        // Next Group Start: the Group after its Group.
        (NEXT_GROUP, Some(at(3, 1)), Some(at(4, 0))),
        // "If no content has been delivered yet, the filter Start Location
        // is {0, 0}", for both.
        (LARGEST, None, Some(at(0, 0))),
        (NEXT_GROUP, None, Some(at(0, 0))),
        // The Start Location that is given, also before the Largest Object.
        (Some(from(2, 0)), Some(at(3, 1)), Some(at(2, 0))),
        (Some(from(9, 4)), Some(at(3, 1)), Some(at(9, 4))),
        (Some(from(9, 4)), None, Some(at(9, 4))),
        (range, Some(at(3, 1)), Some(at(4, 1))),
        // No Object follows the last one a Group can have, and no Group the
        // last one.
        (LARGEST, Some(at(3, last)), Some(at(4, 0))),
        (LARGEST, Some(at(last, last)), None),
        (NEXT_GROUP, Some(at(last, 0)), None),
    ];
    for (filter, largest, start) in cases {
        assert_eq!(
            filter_start(filter, largest),
            start,
            "{filter:?} {largest:?}"
        );
    }
}

#[test]
fn the_end_group_of_a_filter_is_the_group_of_its_start_plus_the_delta() {
    let range = |group, end_group_delta| {
        Some(SubscriptionFilter::AbsoluteRange {
            start: at(group, 7),
            end_group_delta,
        })
    };
    // (the filter; the last Group that passes, `None` where the
    //  subscription is open ended)
    let cases = [
        (None, None),
        (LARGEST, None),
        (NEXT_GROUP, None),
        (Some(from(3, 2)), None),
        (range(3, 0), Some(3)),
        (range(3, 1), Some(4)),
        (range(0, 0), Some(0)),
        (range(10, 250), Some(260)),
        (range(u64::MAX, 0), Some(u64::MAX)),
    ];
    for (filter, end_group) in cases {
        assert_eq!(filter_end_group(filter), end_group, "{filter:?}");
    }
}

/// Bob's SUBSCRIBE for the track "a" with this filter.
pub(super) fn subscribe_with(relay: &mut Relay, filter: Option<SubscriptionFilter>) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"a"),
            parameters: filter
                .map(Parameter::SubscriptionFilter)
                .into_iter()
                .collect(),
        }),
    })
}

/// The streams of Bob among the actions, and the Objects sent on them.
fn to_bob(actions: &[Action], bobs: &mut Vec<OutStreamKey>) -> (usize, usize) {
    let mut opened = 0;
    for action in actions {
        if let Action::OpenDataStream {
            session: BOB,
            out_stream,
            ..
        } = action
        {
            bobs.push(*out_stream);
            opened += 1;
        }
    }
    let sent = actions
        .iter()
        .filter(|action| matches!(action, Action::SendObject { out_stream, .. } if bobs.contains(out_stream)))
        .count();
    (opened, sent)
}

#[test]
fn a_subscriber_gets_the_objects_from_the_start_location_of_its_filter_on() {
    let range = |start, end_group_delta| {
        Some(SubscriptionFilter::AbsoluteRange {
            start,
            end_group_delta,
        })
    };
    // Bob subscribes when the Largest Object is {3, 1}. Then come: the
    // Object {3, 2} on the stream that brought the first two; a stream of
    // Group 2, which is late, with its Object {2, 0}; and a stream of Group
    // 4 with {4, 0}.
    //
    // (Bob's filter; whether he gets {3, 2}, {2, 0}, {4, 0})
    let cases = [
        (None, [true, true, true]),
        (LARGEST, [true, false, true]),
        (NEXT_GROUP, [false, false, true]),
        (Some(from(0, 0)), [true, true, true]),
        (Some(from(2, 0)), [true, true, true]),
        (Some(from(2, 1)), [true, false, true]),
        (Some(from(3, 2)), [true, false, true]),
        (Some(from(3, 3)), [false, false, true]),
        (Some(from(4, 0)), [false, false, true]),
        (Some(from(4, 1)), [false, false, false]),
        // `AbsoluteRange`: to the End Group, the Group of the Start
        // Location plus the End Group Delta, which passes whole. Delta 0:
        // "the remainder of that Group".
        (range(at(3, 2), 0), [true, false, false]),
        (range(at(3, 2), 1), [true, false, true]),
        (range(at(3, 3), 1), [false, false, true]),
        (range(at(2, 1), 1), [true, false, false]),
        (range(at(2, 1), 2), [true, false, true]),
        // An End Group that {3, 2} is past already: the Group 2 that comes
        // late is still in the range, and is sent.
        (range(at(2, 0), 0), [false, true, false]),
        (range(at(0, 0), 3), [true, true, false]),
        (range(at(0, 0), 4), [true, true, true]),
        // The whole range is before what comes, or after it.
        (range(at(0, 0), 1), [false, false, false]),
        (range(at(5, 0), 9), [false, false, false]),
    ];
    for (filter, expected) in cases {
        let mut relay = relay_with_track();
        // Alice has no filter, and gets every Object.
        subscribe_to_track(&mut relay, ALICE);
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        object_arrived(&mut relay, 7, 1);
        // The LARGEST_OBJECT he is told is the one his filter counts from.
        assert_eq!(
            subscribe_with(&mut relay, filter),
            [subscribe_ok(BOB, Some(at(3, 1)))],
            "{filter:?}"
        );
        let mut bobs = Vec::new();
        let mut got = [false; 3];
        // {3, 2}: a stream of his own, from that Object on.
        let same_stream = object_arrived(&mut relay, 7, 2);
        got[0] = to_bob(&same_stream, &mut bobs) == (1, 1);
        // {2, 0}: no stream is opened for a Group he gets nothing of.
        let mut late = data_stream_opened(&mut relay, 2);
        late.extend(object_arrived(&mut relay, 2, 0));
        got[1] = to_bob(&late, &mut bobs) == (1, 1);
        let mut next = data_stream_opened(&mut relay, 4);
        next.extend(object_arrived(&mut relay, 4, 0));
        got[2] = to_bob(&next, &mut bobs) == (1, 1);
        assert_eq!(got, expected, "{filter:?}");
        // Nothing was opened where nothing was sent.
        let streams = expected.iter().filter(|got| **got).count();
        assert_eq!(bobs.len(), streams, "{filter:?}");
    }
}

#[test]
fn a_subscription_that_starts_inside_a_subgroup_gets_its_stream_with_the_first_object_that_passes()
{
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    // Bob's subscription starts at {3, 2}. The stream of Group 3 comes
    // after it, with the Objects 0, 2 and 5.
    assert_eq!(
        subscribe_with(&mut relay, Some(from(3, 2))),
        [subscribe_ok(BOB, None)]
    );
    // The header opens a stream for Alice alone: which Objects the stream
    // has is not known yet.
    assert_eq!(
        data_stream_opened(&mut relay, 7),
        [open_data_stream(ALICE, 0, header(0))]
    );
    assert_eq!(object_arrived(&mut relay, 7, 0), [send_object(0, 0)]);
    // Bob's stream begins with Object 2: its header names the Subgroup ID
    // and does not claim FIRST_OBJECT (§11.4.2), as for a subscriber that
    // came while the stream was under way.
    assert_eq!(
        object_arrived(&mut relay, 7, 2),
        [
            open_data_stream(
                BOB,
                1,
                SubgroupHeader {
                    subgroup_id: SubgroupId::Explicit(0),
                    first_object: false,
                    ..header(0)
                }
            ),
            send_object(0, 2),
            send_object(1, 2),
        ]
    );
    assert_eq!(
        object_arrived(&mut relay, 7, 5),
        [send_object(0, 5), send_object(1, 5)]
    );
}

#[test]
fn a_subscription_whose_first_object_is_the_first_of_its_subgroup_gets_the_header_as_it_came() {
    let mut relay = relay_with_track();
    // Bob's subscription starts at {3, 1}, and the stream of Group 3 begins
    // with Object 1: the stream is his from its first Object, though it was
    // not opened with the header.
    subscribe_with(&mut relay, Some(from(3, 1)));
    assert_eq!(data_stream_opened(&mut relay, 7), []);
    assert_eq!(
        object_arrived(&mut relay, 7, 1),
        [open_data_stream(BOB, 0, header(0)), send_object(0, 1)]
    );
}

#[test]
fn a_subscriber_that_waited_for_the_publisher_is_counted_from_the_subscribe_ok_it_gets() {
    let mut relay = relay_with_track();
    // Bob announces the namespace; Alice subscribes to the track "c" with
    // the Largest Object filter and waits for Bob's answer.
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 0,
            track_namespace: track(b"c").namespace().clone(),
            parameters: vec![],
        }),
    });
    let asked = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"c"),
            parameters: vec![Parameter::SubscriptionFilter(
                SubscriptionFilter::LargestObject,
            )],
        }),
    });
    assert!(
        matches!(
            asked.first(),
            Some(Action::OpenRequest { session: BOB, .. })
        ),
        "{asked:?}"
    );
    // Bob accepts, and says that the Largest Object is {7, 1}: that is what
    // Alice's SUBSCRIBE_OK has, and what her filter counts from.
    let accepted = relay.step(Event::OwnRequestMessage {
        session: BOB,
        request: OwnRequestKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 60,
            parameters: vec![Parameter::LargestObject(at(7, 1))],
            track_properties: vec![],
        }),
    });
    let told = accepted.iter().any(|action| {
        matches!(
            action,
            Action::SendRequest {
                session: ALICE,
                message: ControlMessage::SubscribeOk(ok),
                ..
            } if ok.parameters == [Parameter::LargestObject(at(7, 1))]
        )
    });
    assert!(told, "{accepted:?}");
    // Bob's stream of Group 7 brings Object 1, which is not after the
    // Largest Object, and then Object 2, which is.
    let stream = StreamKey(9);
    let opened = relay.step(Event::DataStreamOpened {
        session: BOB,
        stream,
        header: header_of_group(60, 7),
    });
    assert_eq!(opened, []);
    let object = |object_id| Event::Object {
        session: BOB,
        stream,
        object: object(object_id),
    };
    assert_eq!(relay.step(object(1)), []);
    let passed = relay.step(object(2));
    assert!(
        matches!(
            passed.as_slice(),
            [
                Action::OpenDataStream { session: ALICE, .. },
                Action::SendObject { .. }
            ]
        ),
        "{passed:?}"
    );
}

#[test]
fn a_joining_fetch_ends_where_the_largest_object_filter_starts() {
    let mut relay = relay_with_track();
    // The publisher offers the track "b" with the Largest Object {5, 2}.
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
        session: ALICE,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"b"),
            parameters: vec![Parameter::SubscriptionFilter(
                SubscriptionFilter::LargestObject,
            )],
        }),
    });
    // The Joining FETCH: up to and with {5, 2}, End Location {5, 3}
    // (§10.12.2.1).
    let fetched = relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(1),
        first: ControlMessage::Fetch(Fetch {
            request_id: 2,
            range: FetchRange::RelativeJoining {
                joining_request_id: 0,
                joining_start: 0,
            },
            parameters: vec![],
        }),
    });
    let end = fetched.iter().find_map(|action| match action {
        Action::OpenRequest {
            message: ControlMessage::Fetch(fetch),
            ..
        } => match fetch.range {
            FetchRange::Standalone { end, .. } => Some(end),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(end, Some(at(5, 3)), "{fetched:?}");
    // The subscription: {5, 2}, which reaches the relay late, is the
    // FETCH's; {5, 3} is the first that is the subscription's.
    let stream = StreamKey(9);
    relay.step(Event::DataStreamOpened {
        session: PUBLISHER,
        stream,
        header: header_of_group(41, 5),
    });
    let object = |object_id| Event::Object {
        session: PUBLISHER,
        stream,
        object: object(object_id),
    };
    assert_eq!(relay.step(object(2)), []);
    let first = relay.step(object(3));
    assert!(
        matches!(
            first.as_slice(),
            [
                Action::OpenDataStream { session: ALICE, .. },
                Action::SendObject { .. }
            ]
        ),
        "{first:?}"
    );
}

#[test]
fn a_stream_the_subscriber_gets_nothing_of_leaves_nothing_of_the_subscriber_to_end() {
    // (how the publisher's stream ends; what Alice's stream 0 and then her
    //  stream 1 are ended with; the outgoing streams the relay still holds
    //  after both)
    let cases = [
        (StreamEnd::Fin, [finish(0), finish(1)], 2),
        (StreamEnd::Aborted(Some(5)), [reset(0, 5), reset(1, 5)], 0),
        // CANCELLED (0x1).
        (StreamEnd::Lost, [reset(0, 1), reset(1, 1)], 0),
    ];
    for (end, [first, second], out_streams) in cases {
        let mut relay = relay_with_track();
        // Alice gets everything. Bob's subscription starts at {3, 2}.
        subscribe_to_track(&mut relay, ALICE);
        subscribe_with(&mut relay, Some(from(3, 2)));
        // A stream of Group 3 that ends after the Objects 0 and 1, before
        // Bob's first: he has no stream to end.
        data_stream_opened(&mut relay, 7);
        object_arrived(&mut relay, 7, 0);
        object_arrived(&mut relay, 7, 1);
        assert_eq!(data_stream_closed(&mut relay, 7, end), [first], "{end:?}");
        // A stream of Group 2 that ends after its header.
        assert_eq!(
            data_stream_opened(&mut relay, 2),
            [open_data_stream(ALICE, 1, header_of_group(0, 2))]
        );
        assert_eq!(data_stream_closed(&mut relay, 2, end), [second], "{end:?}");
        // The two are Alice's; none is held for Bob, and none of the
        // publisher's.
        assert_eq!(held(&relay), (3, 1, 2, 0, out_streams), "{end:?}");
        // The publisher goes: Bob is told of no stream, Alice of her two.
        let gone = relay.step(Event::SessionClosed { session: PUBLISHER });
        let mut expected = done_actions(ALICE, 0x2, 2).to_vec();
        expected.extend(done_actions(BOB, 0x2, 0));
        assert_eq!(gone, expected, "{end:?}");
    }
}

#[test]
fn a_stream_of_the_end_group_is_opened_with_its_header() {
    let range = |start, end_group_delta| {
        Some(SubscriptionFilter::AbsoluteRange {
            start,
            end_group_delta,
        })
    };
    // (Bob's filter; whether the header of a stream of Group 2 opens a
    //  stream for him)
    let cases = [
        // The End Group itself: a stream of it that ends without an Object
        // still says that there is a Subgroup (§11.4.3).
        (range(at(2, 0), 0), true),
        (range(at(1, 0), 1), true),
        (range(at(0, 0), 5), true),
        // After the End Group, and a range that starts inside the Group.
        (range(at(0, 0), 1), false),
        (range(at(2, 1), 0), false),
    ];
    for (filter, opened) in cases {
        let mut relay = relay_with_track();
        subscribe_with(&mut relay, filter);
        let expected = if opened {
            vec![open_data_stream(BOB, 0, header_of_group(0, 2))]
        } else {
            vec![]
        };
        assert_eq!(data_stream_opened(&mut relay, 2), expected, "{filter:?}");
    }
}

#[test]
fn a_subscriber_with_forward_state_0_and_a_filter_gets_no_objects() {
    let mut relay = relay_with_track();
    relay.step(Event::RequestOpened {
        session: BOB,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"a"),
            parameters: vec![
                Parameter::Forward(false),
                Parameter::SubscriptionFilter(from(3, 1)),
            ],
        }),
    });
    // Neither with the header nor with the first Object that passes his
    // filter.
    assert_eq!(data_stream_opened(&mut relay, 7), []);
    assert_eq!(object_arrived(&mut relay, 7, 1), []);
    assert_eq!(object_arrived(&mut relay, 7, 2), []);
    let gone = relay.step(Event::SessionClosed { session: PUBLISHER });
    assert_eq!(gone, done_actions(BOB, 0x2, 0));
}

fn finish(out_stream: u64) -> Action {
    Action::FinishDataStream {
        out_stream: OutStreamKey(out_stream),
    }
}

fn reset(out_stream: u64, code: u32) -> Action {
    Action::ResetDataStream {
        out_stream: OutStreamKey(out_stream),
        code,
    }
}
