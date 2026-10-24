//! A relay with an upstream relay (`upstream.rs`): which requests go to it,
//! and the session with it, which is opened when a request needs it.

use super::*;
use xmoq_moqt::message::GoAway;

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
        .filter(|action| matches!(action, Action::ConnectUpstream { .. }))
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

/// A FETCH and a `TRACK_STATUS` of Alice for a track of `(xmoq)`.
fn fetch_and_track_status() -> [(&'static str, ControlMessage); 2] {
    [
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
    ]
}

/// What the relay opened on the session with the upstream, by the kind of
/// its first message.
fn opened_upstream(actions: &[Action]) -> Vec<&'static str> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::OpenRequest {
                session: UPSTREAM,
                message,
                ..
            } => Some(match message {
                ControlMessage::Fetch(_) => "FETCH",
                ControlMessage::TrackStatus(_) => "TRACK_STATUS",
                ControlMessage::Subscribe(_) => "SUBSCRIBE",
                _ => "other",
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn a_fetch_or_track_status_waits_for_the_session_with_the_upstream_like_a_subscribe() {
    for (name, first) in fetch_and_track_status() {
        let mut relay = edge(&[]);
        let came = relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(0),
            first,
        });
        assert_eq!((connects(&came), refusals(&came).len()), (1, 0), "{name}");
        assert_eq!(opened_upstream(&upstream_opened(&mut relay)), [name]);
    }
}

#[test]
fn a_fetch_or_track_status_that_waited_for_an_upstream_that_cannot_be_reached_is_refused() {
    for (name, first) in fetch_and_track_status() {
        let mut relay = edge(&[]);
        relay.step(Event::RequestOpened {
            session: ALICE,
            stream: StreamKey(0),
            first,
        });
        assert_eq!(
            refusals(&relay.step(Event::UpstreamFailed)),
            [INTERNAL_ERROR],
            "{name}"
        );
    }
}

/// The `RENDEZVOUS_TIMEOUT` of the relay's SUBSCRIBEs.
fn rendezvous_of(actions: &[Action]) -> Vec<Option<u64>> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::OpenRequest {
                message: ControlMessage::Subscribe(subscribe),
                ..
            } => Some(xmoq_moqt::params::rendezvous_timeout(&subscribe.parameters)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_subscribe_held_for_a_publisher_to_appear_is_held_by_the_upstream_as_well() {
    struct Row {
        name: &'static str,
        announced: bool,
        rendezvous_ms: Option<u64>,
        asked: Option<u64>,
    }
    let rows = [
        Row {
            name: "held",
            announced: false,
            rendezvous_ms: Some(5_000),
            asked: Some(5_000),
        },
        Row {
            // MAX_RENDEZVOUS_MS.
            name: "held for longer than the relay holds",
            announced: false,
            rendezvous_ms: Some(60_000),
            asked: Some(30_000),
        },
        Row {
            name: "not held",
            announced: false,
            rendezvous_ms: None,
            asked: None,
        },
        Row {
            // The publisher that would appear is the one that announced.
            name: "held, of a session that announced the namespace",
            announced: true,
            rendezvous_ms: Some(5_000),
            asked: None,
        },
    ];
    for row in rows {
        let mut relay = edge(&[]);
        if row.announced {
            announce(&mut relay, &[b"xmoq"]);
        }
        upstream_opened(&mut relay);
        let actions = subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], row.rendezvous_ms);
        assert_eq!(rendezvous_of(&actions), [row.asked], "{}", row.name);
    }
}

#[test]
fn a_subscribe_held_while_the_relay_connects_is_held_by_the_upstream_as_well() {
    let mut relay = edge(&[]);
    subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], Some(5_000));
    assert_eq!(rendezvous_of(&upstream_opened(&mut relay)), [Some(5_000)]);
}

/// The GOAWAY of the upstream, on its control stream, naming `uri`.
fn upstream_goes_away(relay: &mut Relay, uri: &[u8]) -> Vec<Action> {
    relay.step(Event::ControlMessage {
        session: UPSTREAM,
        message: ControlMessage::Setup(Setup::default()),
    });
    relay.step(Event::ControlMessage {
        session: UPSTREAM,
        message: ControlMessage::GoAway(GoAway {
            new_session_uri: uri.to_vec(),
            timeout_ms: 10_000,
            request_id: Some(0),
        }),
    })
}

#[test]
fn a_goaway_of_the_upstream_has_the_relay_connect_where_it_says_for_what_it_carries() {
    struct Row {
        name: &'static str,
        /// Alice subscribed before the GOAWAY.
        subscribed: bool,
        uri: &'static [u8],
        connects_to: Option<&'static [u8]>,
    }
    let rows = [
        Row {
            name: "a URI, with a subscription",
            subscribed: true,
            uri: b"https://127.0.0.1:4444/",
            connects_to: Some(b"https://127.0.0.1:4444/"),
        },
        Row {
            // §10.4: the current URI.
            name: "no URI, with a subscription",
            subscribed: true,
            uri: b"",
            connects_to: Some(b""),
        },
        Row {
            name: "a URI, with nothing carried",
            subscribed: false,
            uri: b"https://127.0.0.1:4444/",
            connects_to: None,
        },
    ];
    for row in rows {
        let mut relay = edge(&[]);
        upstream_opened(&mut relay);
        if row.subscribed {
            subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], None);
        }
        let gone = upstream_goes_away(&mut relay, row.uri);
        let connects_to: Vec<&[u8]> = gone
            .iter()
            .filter_map(|action| match action {
                Action::ConnectUpstream { uri } => Some(uri.as_slice()),
                _ => None,
            })
            .collect();
        assert_eq!(
            connects_to,
            row.connects_to.into_iter().collect::<Vec<_>>(),
            "{}",
            row.name
        );
    }
}

#[test]
fn the_session_that_replaces_one_that_went_away_is_asked_for_the_tracks_the_old_one_carries() {
    const NEW_UPSTREAM: SessionKey = SessionKey(10);
    let mut relay = edge(&[]);
    upstream_opened(&mut relay);
    subscribe_to(&mut relay, 0, &[b"xmoq", b"a"], None);
    // The upstream accepts: the track has its source, and Alice her answer.
    relay.step(Event::OwnRequestMessage {
        session: UPSTREAM,
        request: OwnRequestKey(0),
        message: ControlMessage::SubscribeOk(SubscribeOk {
            track_alias: 1,
            parameters: vec![],
            track_properties: vec![],
        }),
    });
    upstream_goes_away(&mut relay, b"https://127.0.0.1:4444/");
    let opened = relay.step(Event::SessionOpened {
        session: NEW_UPSTREAM,
        role: Role::Client,
    });
    assert_eq!(asked_of(&opened), [NEW_UPSTREAM]);
    // And nothing more of the old one.
    assert_eq!(
        routed(&subscribe_to(&mut relay, 1, &[b"xmoq", b"b"], None)),
        Routed::Asks(vec![NEW_UPSTREAM])
    );
}
