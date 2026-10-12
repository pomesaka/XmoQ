//! A subscriber's `NEW_GROUP_REQUEST` and what the relay passes on of it
//! (§10.2.13).

use std::collections::{HashMap, HashSet};

use xmoq_moqt::message::RequestUpdate;

use super::*;

/// `DYNAMIC_GROUPS` (§12.6).
const DYNAMIC_GROUPS: u64 = 0x30;

/// How a subscriber asks.
#[derive(Debug, Clone, Copy)]
enum By {
    /// In its SUBSCRIBE.
    Subscribe,
    /// In a `REQUEST_UPDATE` after a SUBSCRIBE without it.
    Update,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// A subscriber asks with this value.
    Asks(SessionKey, By, u64),
    /// The publisher sends the first Object of this Group.
    Group(u64),
}

/// A relay whose publisher offers "a", with `DYNAMIC_GROUPS` of this value
/// or without the property.
fn relay_with_a_track_of(dynamic_groups: Option<u64>) -> Relay {
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
        first: ControlMessage::Publish(Publish {
            request_id: 0,
            full_track_name: track(b"a"),
            track_alias: UPSTREAM_ALIAS,
            parameters: vec![],
            track_properties: dynamic_groups
                .map(|value| KeyValuePair::varint(DYNAMIC_GROUPS, value).unwrap())
                .into_iter()
                .collect(),
        }),
    });
    relay
}

/// The `REQUEST_UPDATE`s the relay sends on the publisher's PUBLISH stream
/// among these actions: (Request ID, `NEW_GROUP_REQUEST`).
fn passed_on(actions: &[Action]) -> Vec<(u64, u64)> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::SendRequest {
                session: PUBLISHER,
                stream: StreamKey(0),
                message:
                    ControlMessage::RequestUpdate(RequestUpdate {
                        request_id,
                        parameters,
                    }),
            } => match parameters[..] {
                [Parameter::NewGroupRequest(group)] => Some((*request_id, group)),
                _ => panic!("{parameters:?}"),
            },
            _ => None,
        })
        .collect()
}

fn subscribes(relay: &mut Relay, session: SessionKey, parameters: Vec<Parameter>) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session,
        stream: StreamKey(0),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: 0,
            full_track_name: track(b"a"),
            parameters,
        }),
    })
}

/// `DYNAMIC_GROUPS` of the track; what happens; the `REQUEST_UPDATE`s the
/// relay sends the publisher.
type Case = (Option<u64>, Vec<Step>, Vec<(u64, u64)>);

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a table with the draft's sentences beside its rows, and one loop"
)]
fn a_new_group_request_is_passed_on_to_the_publisher_as_the_draft_has_it() {
    use By::{Subscribe as InSubscribe, Update};
    use Step::{Asks, Group};
    // (DYNAMIC_GROUPS of the track; what happens; the REQUEST_UPDATEs the
    // relay sends the publisher: its Request ID, which is odd and goes up
    // by 2, and the value)
    let cases: Vec<Case> = vec![
        // 0, "no Group information": passed on.
        (Some(1), vec![Asks(ALICE, InSubscribe, 0)], vec![(1, 0)]),
        (Some(1), vec![Asks(ALICE, Update, 0)], vec![(1, 0)]),
        // "unless: 1. The Track does not support dynamic Groups".
        (None, vec![Asks(ALICE, InSubscribe, 0)], vec![]),
        (Some(0), vec![Asks(ALICE, Update, 0)], vec![]),
        // "a value larger than the Largest Group".
        (
            Some(1),
            vec![Group(5), Asks(ALICE, InSubscribe, 6)],
            vec![(1, 6)],
        ),
        (
            Some(1),
            vec![Group(5), Asks(ALICE, Update, 6)],
            vec![(1, 6)],
        ),
        // "a non-zero value less than or equal to the Largest Group, it
        // does not send a NEW_GROUP_REQUEST upstream".
        (Some(1), vec![Group(5), Asks(ALICE, InSubscribe, 5)], vec![]),
        (Some(1), vec![Group(5), Asks(ALICE, Update, 3)], vec![]),
        // "unless: 2. There is already an outstanding NEW_GROUP_REQUEST
        // from this Relay with a greater or equal value".
        (
            Some(1),
            vec![Asks(ALICE, InSubscribe, 0), Asks(BOB, InSubscribe, 0)],
            vec![(1, 0)],
        ),
        (
            Some(1),
            vec![
                Group(5),
                Asks(ALICE, InSubscribe, 6),
                Asks(BOB, InSubscribe, 0),
                Asks(BOB, Update, 6),
            ],
            vec![(1, 6)],
        ),
        (
            Some(1),
            vec![
                Group(5),
                Asks(ALICE, InSubscribe, 6),
                Asks(BOB, InSubscribe, 7),
            ],
            vec![(1, 6), (3, 7)],
        ),
        // "the request is considered outstanding until the Largest Group
        // increases": an Object of the same Group does not end it, one of
        // the next does.
        (
            Some(1),
            vec![
                Group(5),
                Asks(ALICE, InSubscribe, 0),
                Group(5),
                Asks(BOB, InSubscribe, 0),
            ],
            vec![(1, 0)],
        ),
        (
            Some(1),
            vec![
                Group(5),
                Asks(ALICE, InSubscribe, 0),
                Group(6),
                Asks(BOB, InSubscribe, 0),
            ],
            vec![(1, 0), (3, 0)],
        ),
    ];
    for (dynamic_groups, steps, expected) in cases {
        let case = format!("{dynamic_groups:?} {steps:?}");
        let mut relay = relay_with_a_track_of(dynamic_groups);
        let mut actions = Vec::new();
        // Each Group on a stream of its own; a Group that comes again, as
        // the next Object of its stream.
        let mut objects_of: HashMap<u64, u64> = HashMap::new();
        let mut subscribed: HashSet<SessionKey> = HashSet::new();
        for step in steps {
            match step {
                Group(group) => {
                    let stream = StreamKey(100 + group);
                    let next = objects_of.entry(group).or_insert(0);
                    if *next == 0 {
                        relay.step(Event::DataStreamOpened {
                            session: PUBLISHER,
                            stream,
                            header: header_of_group(UPSTREAM_ALIAS, group),
                        });
                    }
                    relay.step(Event::Object {
                        session: PUBLISHER,
                        stream,
                        object: object(*next),
                    });
                    *next += 1;
                }
                Asks(session, InSubscribe, group) => {
                    let parameters = vec![Parameter::NewGroupRequest(group)];
                    subscribed.insert(session);
                    actions.extend(subscribes(&mut relay, session, parameters));
                }
                Asks(session, Update, group) => {
                    // A subscriber that has not subscribed yet does first.
                    if subscribed.insert(session) {
                        subscribes(&mut relay, session, vec![]);
                    }
                    let update = relay.step(Event::RequestMessage {
                        session,
                        stream: StreamKey(0),
                        message: ControlMessage::RequestUpdate(RequestUpdate {
                            request_id: 2,
                            parameters: vec![Parameter::NewGroupRequest(group)],
                        }),
                    });
                    // §10.9: the subscriber's update has its REQUEST_OK,
                    // whatever is passed on.
                    assert!(
                        update.contains(&Action::SendRequest {
                            session,
                            stream: StreamKey(0),
                            message: ControlMessage::RequestOk(RequestOk::default()),
                        }),
                        "{case}: {update:?}"
                    );
                    actions.extend(update);
                }
            }
        }
        assert_eq!(passed_on(&actions), expected, "{case}");
    }
}
