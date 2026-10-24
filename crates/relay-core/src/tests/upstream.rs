//! A relay with an upstream relay (`upstream.rs`): which requests go to it,
//! and the session with it, which is opened when a request needs it.

use super::*;

/// The relay's session with the upstream relay, of which it is the client.
const UPSTREAM: SessionKey = SessionKey(9);

/// The Error Codes of §10.6.2 these tests see.
const INTERNAL_ERROR: u64 = 0x0;
const DOES_NOT_EXIST: u64 = 0x10;

fn namespace_of(fields: &[&[u8]]) -> TrackNamespace {
    TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
}

fn track_in(fields: &[&[u8]]) -> FullTrackName {
    FullTrackName::new(namespace_of(fields), b"a".to_vec()).unwrap()
}

/// A relay whose upstream takes `upstream`, with the publisher and Alice.
fn edge(upstream: &[&[u8]]) -> Relay {
    let mut relay = Relay::new("test/0");
    relay.set_upstream(namespace_of(upstream));
    for session in [PUBLISHER, ALICE] {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
    relay
}

fn announce(relay: &mut Relay, fields: &[&[u8]]) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session: PUBLISHER,
        stream: StreamKey(0),
        first: ControlMessage::PublishNamespace(PublishNamespace {
            request_id: 0,
            track_namespace: namespace_of(fields),
            parameters: vec![],
        }),
    })
}

/// Alice's SUBSCRIBE on her stream `stream`, held for a publisher to appear
/// for that many milliseconds where `rendezvous_ms` says.
fn subscribe_to(
    relay: &mut Relay,
    stream: u64,
    fields: &[&[u8]],
    rendezvous_ms: Option<u64>,
) -> Vec<Action> {
    relay.step(Event::RequestOpened {
        session: ALICE,
        stream: StreamKey(stream),
        first: ControlMessage::Subscribe(Subscribe {
            request_id: stream * 2,
            full_track_name: track_in(fields),
            parameters: rendezvous_ms
                .map(Parameter::RendezvousTimeout)
                .into_iter()
                .collect(),
        }),
    })
}

fn upstream_opened(relay: &mut Relay) -> Vec<Action> {
    relay.step(Event::SessionOpened {
        session: UPSTREAM,
        role: Role::Client,
    })
}

/// The sessions the relay sent a SUBSCRIBE to.
fn asked_of(actions: &[Action]) -> Vec<SessionKey> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::OpenRequest {
                session,
                message: ControlMessage::Subscribe(_),
                ..
            } => Some(*session),
            _ => None,
        })
        .collect()
}

/// The Error Codes of the refusals Alice was sent.
fn refusals(actions: &[Action]) -> Vec<u64> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::SendRequest {
                session: ALICE,
                message: ControlMessage::RequestError(error),
                ..
            } => Some(error.error_code),
            _ => None,
        })
        .collect()
}

fn connects(actions: &[Action]) -> usize {
    actions
        .iter()
        .filter(|action| matches!(action, Action::ConnectUpstream))
        .count()
}

#[derive(Debug, PartialEq, Eq)]
enum Routed {
    /// The session with the upstream is asked for.
    Connects,
    /// The SUBSCRIBE goes to these sessions.
    Asks(Vec<SessionKey>),
    /// Refused with this code.
    Refused(u64),
}

fn routed(actions: &[Action]) -> Routed {
    match (connects(actions), asked_of(actions), refusals(actions)) {
        (1, asked, refused) if asked.is_empty() && refused.is_empty() => Routed::Connects,
        (0, asked, refused) if !asked.is_empty() && refused.is_empty() => Routed::Asks(asked),
        (0, asked, refused) if asked.is_empty() && refused.len() == 1 => {
            Routed::Refused(refused[0])
        }
        other => panic!("not one of the outcomes: {other:?}"),
    }
}

#[test]
fn a_subscribe_goes_to_the_upstream_only_for_its_namespace_and_where_nobody_announced() {
    struct Row {
        name: &'static str,
        upstream: &'static [&'static [u8]],
        announced: Option<&'static [&'static [u8]]>,
        opened: bool,
        track: &'static [&'static [u8]],
        routed: Routed,
    }
    let rows = [
        Row {
            name: "every namespace, no session yet",
            upstream: &[],
            announced: None,
            opened: false,
            track: &[b"xmoq", b"live"],
            routed: Routed::Connects,
        },
        Row {
            name: "within the prefix, no session yet",
            upstream: &[b"xmoq"],
            announced: None,
            opened: false,
            track: &[b"xmoq", b"live"],
            routed: Routed::Connects,
        },
        Row {
            name: "outside the prefix",
            upstream: &[b"xmoq"],
            announced: None,
            opened: false,
            track: &[b"other"],
            routed: Routed::Refused(DOES_NOT_EXIST),
        },
        Row {
            // Field by field, as Namespace Prefix Matching (§9.5).
            name: "a field that begins as the prefix does",
            upstream: &[b"xmoq"],
            announced: None,
            opened: false,
            track: &[b"xmoqq"],
            routed: Routed::Refused(DOES_NOT_EXIST),
        },
        Row {
            name: "announced here, no session yet",
            upstream: &[],
            announced: Some(&[b"xmoq"]),
            opened: false,
            track: &[b"xmoq", b"live"],
            routed: Routed::Asks(vec![PUBLISHER]),
        },
        Row {
            name: "the session is open",
            upstream: &[],
            announced: None,
            opened: true,
            track: &[b"xmoq", b"live"],
            routed: Routed::Asks(vec![UPSTREAM]),
        },
        Row {
            name: "the session is open and the namespace announced here",
            upstream: &[],
            announced: Some(&[b"xmoq", b"live"]),
            opened: true,
            track: &[b"xmoq", b"live"],
            routed: Routed::Asks(vec![PUBLISHER]),
        },
        Row {
            name: "the session is open and another namespace announced here",
            upstream: &[],
            announced: Some(&[b"xmoq", b"chat"]),
            opened: true,
            track: &[b"xmoq", b"live"],
            routed: Routed::Asks(vec![UPSTREAM]),
        },
    ];
    for row in rows {
        let mut relay = edge(row.upstream);
        if let Some(announced) = row.announced {
            announce(&mut relay, announced);
        }
        if row.opened {
            upstream_opened(&mut relay);
        }
        let actions = subscribe_to(&mut relay, 0, row.track, None);
        assert_eq!(routed(&actions), row.routed, "{}", row.name);
    }
}

#[test]
fn the_subscribes_that_came_while_the_relay_connected_go_upstream_once_it_has() {
    let mut relay = edge(&[]);
    assert_eq!(
        connects(&subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], None)),
        1
    );
    // Asked for once, whatever comes meanwhile.
    assert_eq!(
        connects(&subscribe_to(&mut relay, 1, &[b"xmoq", b"a"], None)),
        0
    );
    assert_eq!(
        connects(&subscribe_to(&mut relay, 2, &[b"xmoq", b"b"], None)),
        0
    );
    let opened = upstream_opened(&mut relay);
    // §10.1: the client's Request IDs are even.
    let ids: Vec<u64> = opened
        .iter()
        .filter_map(|action| match action {
            Action::OpenRequest {
                session: UPSTREAM,
                message: ControlMessage::Subscribe(subscribe),
                ..
            } => Some(subscribe.request_id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, [0, 2], "one SUBSCRIBE a track");
    assert!(matches!(
        opened.first(),
        Some(Action::SendControl {
            session: UPSTREAM,
            message: ControlMessage::Setup(_),
        })
    ));
    assert_eq!(refusals(&opened), [0u64; 0]);
}

#[test]
fn what_waits_for_an_upstream_that_cannot_be_reached_is_refused_but_a_held_subscribe() {
    let mut relay = edge(&[]);
    subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], None);
    subscribe_to(&mut relay, 1, &[b"xmoq", b"b"], Some(5_000));
    let failed = relay.step(Event::UpstreamFailed);
    let errors: Vec<&RequestError> = failed
        .iter()
        .filter_map(|action| match action {
            Action::SendRequest {
                message: ControlMessage::RequestError(error),
                ..
            } => Some(error),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1, "{failed:?}");
    assert_eq!(errors[0].error_code, INTERNAL_ERROR);
    assert_eq!(errors[0].retry_interval, 1_001);
    // The next request asks for the session again.
    assert_eq!(
        connects(&subscribe_to(&mut relay, 2, &[b"xmoq", b"c"], None)),
        1
    );
    // The held one is asked for with the others.
    assert_eq!(asked_of(&upstream_opened(&mut relay)), [UPSTREAM, UPSTREAM]);
}

#[test]
fn a_session_with_the_upstream_that_went_is_opened_again_for_the_next_request() {
    let mut relay = edge(&[]);
    subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], None);
    upstream_opened(&mut relay);
    let gone = relay.step(Event::SessionClosed { session: UPSTREAM });
    // What was asked of it will not be answered.
    assert_eq!(refusals(&gone), [DOES_NOT_EXIST]);
    assert_eq!(
        routed(&subscribe_to(&mut relay, 1, &[b"xmoq", b"a"], None)),
        Routed::Connects
    );
}

#[test]
fn a_fetch_or_track_status_nobody_can_answer_has_the_session_with_the_upstream_opened() {
    let rows: [(&str, ControlMessage); 2] = [
        (
            "FETCH",
            ControlMessage::Fetch(Fetch {
                request_id: 0,
                range: FetchRange::Standalone {
                    full_track_name: track_in(&[b"xmoq", b"a"]),
                    start: Location {
                        group: 0,
                        object: 0,
                    },
                    end: Location {
                        group: 1,
                        object: 0,
                    },
                },
                parameters: vec![],
            }),
        ),
        (
            "TRACK_STATUS",
            ControlMessage::TrackStatus(Subscribe {
                request_id: 0,
                full_track_name: track_in(&[b"xmoq", b"a"]),
                parameters: vec![],
            }),
        ),
    ];
    for (name, first) in rows {
        let mut relay = edge(&[]);
        let actions = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(0),
            first,
        });
        // Refused all the same (known gap).
        assert_eq!(connects(&actions), 1, "{name}");
        assert_eq!(refusals(&actions).len(), 1, "{name}");
    }
}
