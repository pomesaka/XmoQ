//! What the relay answers a request with where it does not take it
//! (§10.6.2), and what a subscriber gets of a publisher's refusal or reset.

use xmoq_moqt::message::SubscribeNamespace;

use super::routing::{announce, bob_is_done, namespace_of, refused, relay, subscribe_to};
use super::*;

/// The request stream of Alice's FETCH.
pub(super) const FETCH_STREAM: StreamKey = StreamKey(7);

pub(super) fn at(group: u64, object: u64) -> Location {
    Location { group, object }
}

/// A Standalone FETCH for Groups 0 and 1 of the track "a".
pub(super) fn fetch_of_a(request_id: u64) -> ControlMessage {
    ControlMessage::Fetch(Fetch {
        request_id,
        range: FetchRange::Standalone {
            full_track_name: track(b"a"),
            start: at(0, 0),
            end: at(2, 0),
        },
        parameters: vec![],
    })
}

fn subscribe_namespace(request_id: u64, fields: &[&[u8]]) -> ControlMessage {
    ControlMessage::SubscribeNamespace(SubscribeNamespace {
        request_id,
        track_namespace_prefix: namespace_of(fields),
        parameters: vec![],
    })
}

pub(super) fn opened(session: SessionKey, stream: u64, first: ControlMessage) -> Event {
    Event::RequestOpened {
        session,
        stream: StreamKey(stream),
        first,
    }
}

fn open_sessions(relay: &mut Relay, sessions: impl IntoIterator<Item = SessionKey>) {
    for session in sessions {
        relay.step(Event::SessionOpened {
            session,
            role: Role::Server,
        });
    }
}

/// The relay opened a request on a publisher for these actions.
pub(super) fn asks(actions: &[Action]) -> bool {
    matches!(
        actions,
        [
            Action::OpenRequest { .. },
            Action::StartUpstreamTimer { .. }
        ]
    )
}

fn accepted(actions: &[Action]) -> bool {
    matches!(
        actions,
        [Action::SendRequest {
            message: ControlMessage::RequestOk(_),
            ..
        }]
    )
}

/// What of `actions` goes on the request stream `stream` of `session`.
fn on_the_stream(actions: &[Action], session: SessionKey, stream: StreamKey) -> Vec<Action> {
    actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                Action::SendRequest { session: to, stream: on, .. }
                | Action::FinishRequest { session: to, stream: on }
                    if (*to, *on) == (session, stream)
            )
        })
        .cloned()
        .collect()
}

/// A request the relay does not take, by the row of the relay's table it
/// comes from.
#[derive(Debug, Clone, Copy)]
enum Refused {
    /// A `PUBLISH_NAMESPACE` beyond the ones a session may send at once.
    AnnouncementBeyondTheRate,
    /// A SUBSCRIBE beyond the requests the relay has on the one publisher
    /// for this subscriber's session.
    SubscribeBeyondOneSubscribersShare,
    /// The same, with `RENDEZVOUS_TIMEOUT`.
    HeldSubscribeBeyondOneSubscribersShare,
    /// A SUBSCRIBE beyond the requests the relay has on the one publisher
    /// for everybody.
    SubscribeBeyondThePublishersRequests,
    /// A FETCH beyond the FETCH requests the relay has on the one publisher
    /// for this subscriber's session.
    FetchBeyondOneSubscribersShare,
    /// A FETCH beyond the FETCH requests the relay has on the one publisher
    /// for everybody.
    FetchBeyondThePublishersFetches,
    /// A fifth PUBLISH of a track.
    PublishBeyondTheSources,
    /// A PUBLISH of a session that is a source of the track and has not said
    /// `PUBLISH_DONE`.
    PublishOfASourceThatGoesOn,
    /// A PUBLISH whose Track Properties make the track malformed (§12.7).
    PublishOfAMalformedTrack,
    /// A PUBLISH with a Mandatory Track Property (§2.5.1).
    PublishWithAMandatoryTrackProperty,
    /// A second SUBSCRIBE of a session for a track it is subscribed to.
    SecondSubscribeOfASession,
    /// A `REQUEST_UPDATE` for a SUBSCRIBE the relay has not answered.
    UpdateBeforeTheAnswer,
    /// A `SUBSCRIBE_NAMESPACE` beyond the ones a session may have.
    NamespaceSubscriptionBeyondTheSessions,
    /// A `SUBSCRIBE_NAMESPACE` whose prefix overlaps another of the session.
    OverlappingNamespaceSubscription,
    /// A `REQUEST_UPDATE` beyond the ones that may wait for the answer to a
    /// FETCH.
    FetchUpdateBeyondTheHeld,
    /// A SUBSCRIBE beyond the subscribers that may wait for the answer to the
    /// relay's SUBSCRIBE.
    SubscriberBeyondTheWaiting,
    /// A SUBSCRIBE without `RENDEZVOUS_TIMEOUT` for a track whose every
    /// source said `PUBLISH_DONE` and whose last streams are awaited.
    SubscribeOfADrainingTrack,
}

/// What the relay does about the last request of a situation, and the
/// session and the request stream that request came on.
type Refusal = (Vec<Action>, SessionKey, StreamKey);

/// The relay brought to `situation`, and what it does about the last
/// request.
fn refusal(situation: Refused) -> Refusal {
    use Refused::*;
    match situation {
        AnnouncementBeyondTheRate => announcement_beyond_the_rate(),
        SubscribeBeyondOneSubscribersShare => subscribe_beyond_one_subscribers_share(vec![]),
        HeldSubscribeBeyondOneSubscribersShare => {
            subscribe_beyond_one_subscribers_share(vec![Parameter::RendezvousTimeout(20_000)])
        }
        SubscribeBeyondThePublishersRequests => subscribe_beyond_the_publishers_requests(),
        FetchBeyondOneSubscribersShare => fetch_beyond_one_subscribers_share(),
        FetchBeyondThePublishersFetches => fetch_beyond_the_publishers_fetches(),
        PublishBeyondTheSources => publish_beyond_the_sources(),
        PublishOfASourceThatGoesOn => {
            let mut relay = relay_with_track();
            let again = relay.step(opened(PUBLISHER, 1, publish(2, b"a")));
            (again, PUBLISHER, StreamKey(1))
        }
        PublishOfAMalformedTrack => {
            // Immutable Properties inside Immutable Properties (§12.7).
            publish_with(KeyValuePair::bytes(0x0b, vec![0x0b, 0x00]).unwrap())
        }
        PublishWithAMandatoryTrackProperty => {
            publish_with(KeyValuePair::varint(0x4000, 1).unwrap())
        }
        SecondSubscribeOfASession => {
            let mut relay = relay_with_track();
            subscribe_to_track(&mut relay, ALICE);
            let again = relay.step(opened(ALICE, 1, subscribe(2, b"a")));
            (again, ALICE, StreamKey(1))
        }
        UpdateBeforeTheAnswer => update_before_the_answer(),
        NamespaceSubscriptionBeyondTheSessions => namespace_subscription_beyond_the_sessions(),
        OverlappingNamespaceSubscription => {
            let mut relay = relay();
            let first = subscribe_namespace(0, &[b"xmoq"]);
            assert!(accepted(&relay.step(opened(ALICE, 0, first))));
            let under_it = subscribe_namespace(2, &[b"xmoq", b"a"]);
            (relay.step(opened(ALICE, 1, under_it)), ALICE, StreamKey(1))
        }
        FetchUpdateBeyondTheHeld => fetch_update_beyond_the_held(),
        SubscriberBeyondTheWaiting => subscriber_beyond_the_waiting(),
        SubscribeOfADrainingTrack => {
            let mut relay = relay();
            relay.step(opened(BOB, 0, publish(0, b"a")));
            // PUBLISH_DONE that counts a stream, which does not come.
            bob_is_done(&mut relay);
            let actions = subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"]);
            (actions, ALICE, StreamKey(0))
        }
    }
}

fn announcement_beyond_the_rate() -> Refusal {
    let mut relay = Relay::new("test/0");
    // One at once, and one more every 2.5 seconds.
    relay.set_announcements(1, 2_500);
    open_sessions(&mut relay, [ALICE]);
    assert!(accepted(&announce(&mut relay, ALICE, 0, &[b"xmoq"])));
    (
        announce(&mut relay, ALICE, 1, &[b"xmoq"]),
        ALICE,
        StreamKey(1),
    )
}

/// Alice's ninth SUBSCRIBE for a track of the one publisher, with these
/// parameters.
fn subscribe_beyond_one_subscribers_share(parameters: Vec<Parameter>) -> Refusal {
    assert_eq!(crate::MAX_OWN_REQUESTS_FOR_ONE_SUBSCRIBER, 8);
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    for stream in 0..8 {
        let name = format!("a{stream}");
        let first = subscribe(stream * 2, name.as_bytes());
        assert!(asks(&relay.step(opened(ALICE, stream, first))), "{stream}");
    }
    let ninth = ControlMessage::Subscribe(Subscribe {
        request_id: 16,
        full_track_name: track(b"a8"),
        parameters,
    });
    (relay.step(opened(ALICE, 8, ninth)), ALICE, StreamKey(8))
}

fn subscribe_beyond_the_publishers_requests() -> Refusal {
    assert_eq!(crate::MAX_OWN_REQUESTS_PER_SESSION, 64);
    let mut relay = relay();
    // One session takes all the publisher has.
    relay.set_share_of_one_subscriber(64, 16);
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    for stream in 0..64 {
        let name = format!("a{stream}");
        let first = subscribe(stream * 2, name.as_bytes());
        assert!(asks(&relay.step(opened(ALICE, stream, first))), "{stream}");
    }
    let one_more = relay.step(opened(ALICE, 64, subscribe(128, b"a64")));
    (one_more, ALICE, StreamKey(64))
}

fn fetch_beyond_one_subscribers_share() -> Refusal {
    assert_eq!(crate::MAX_OWN_FETCHES_FOR_ONE_SUBSCRIBER, 4);
    let mut relay = relay_with_track();
    for stream in 0..4 {
        let actions = relay.step(opened(ALICE, stream, fetch_of_a(stream * 2)));
        assert!(asks(&actions), "{stream}");
    }
    (
        relay.step(opened(ALICE, 4, fetch_of_a(8))),
        ALICE,
        StreamKey(4),
    )
}

fn fetch_beyond_the_publishers_fetches() -> Refusal {
    assert_eq!(crate::fetch::MAX_OWN_FETCHES_PER_SESSION, 16);
    let mut relay = relay_with_track();
    relay.set_share_of_one_subscriber(64, 16);
    for stream in 0..16 {
        let actions = relay.step(opened(BOB, stream, fetch_of_a(stream * 2)));
        assert!(asks(&actions), "{stream}");
    }
    (
        relay.step(opened(ALICE, 0, fetch_of_a(0))),
        ALICE,
        StreamKey(0),
    )
}

fn publish_beyond_the_sources() -> Refusal {
    assert_eq!(crate::MAX_SOURCES, 4);
    let mut relay = relay_with_track();
    let others = [BOB, SessionKey(11), SessionKey(12)];
    open_sessions(&mut relay, others);
    for session in others {
        assert!(accepted(&relay.step(opened(session, 0, publish(0, b"a")))));
    }
    (
        relay.step(opened(ALICE, 0, publish(0, b"a"))),
        ALICE,
        StreamKey(0),
    )
}

/// Alice's PUBLISH of a track of her own with this Track Property.
fn publish_with(track_property: KeyValuePair) -> Refusal {
    let mut relay = relay_with_track();
    let first = ControlMessage::Publish(Publish {
        request_id: 0,
        full_track_name: track(b"mine"),
        track_alias: 3,
        parameters: vec![],
        track_properties: vec![track_property],
    });
    (relay.step(opened(ALICE, 0, first)), ALICE, StreamKey(0))
}

fn update_before_the_answer() -> Refusal {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    assert!(asks(&subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"])));
    let update = relay.step(Event::RequestMessage {
        session: ALICE,
        stream: StreamKey(0),
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id: 2,
            parameters: vec![],
        }),
    });
    (update, ALICE, StreamKey(0))
}

fn namespace_subscription_beyond_the_sessions() -> Refusal {
    assert_eq!(crate::discovery::MAX_NAMESPACE_SUBSCRIPTIONS_PER_SESSION, 8);
    let mut relay = relay();
    for stream in 0..8 {
        let field = format!("n{stream}");
        let first = subscribe_namespace(stream * 2, &[field.as_bytes()]);
        let actions = relay.step(opened(ALICE, stream, first));
        assert!(accepted(&actions), "{stream}");
    }
    let ninth = subscribe_namespace(16, &[b"n8"]);
    (relay.step(opened(ALICE, 8, ninth)), ALICE, StreamKey(8))
}

fn fetch_update_beyond_the_held() -> Refusal {
    assert_eq!(crate::fetch::MAX_HELD_FETCH_UPDATES, 64);
    let mut relay = relay_with_track();
    assert!(asks(&relay.step(opened(ALICE, 7, fetch_of_a(0)))));
    let update = |request_id| Event::RequestMessage {
        session: ALICE,
        stream: FETCH_STREAM,
        message: ControlMessage::RequestUpdate(RequestUpdate {
            request_id,
            parameters: vec![],
        }),
    };
    for index in 1..=64u64 {
        assert_eq!(relay.step(update(index * 2)), [], "{index}");
    }
    (relay.step(update(130)), ALICE, FETCH_STREAM)
}

fn subscriber_beyond_the_waiting() -> Refusal {
    assert_eq!(crate::MAX_WAITING_SUBSCRIBERS, 64);
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    let waiting = (100..165).map(SessionKey);
    open_sessions(&mut relay, waiting.clone());
    for session in waiting.take(64) {
        let actions = subscribe_to(&mut relay, session, 0, &[b"xmoq"]);
        // The first has the relay ask the publisher; the others wait with
        // it.
        let first = session == SessionKey(100);
        assert_eq!(
            (asks(&actions), actions.is_empty()),
            (first, !first),
            "{session:?}: {actions:?}"
        );
    }
    let last = SessionKey(164);
    let actions = subscribe_to(&mut relay, last, 0, &[b"xmoq"]);
    (actions, last, StreamKey(0))
}

#[test]
fn the_relay_refuses_a_request_with_the_code_and_retry_interval_of_its_situation() {
    use Refused::*;
    // (the request and what was before it; the Error Code; the Retry
    // Interval) of the REQUEST_ERROR, after which the relay finishes the
    // stream. A Retry Interval is the time to wait plus one (§10.6.2).
    let cases: [(Refused, u64, u64); 17] = [
        // How often a session may announce (PUBLISH_NAMESPACE), at the
        // relay: EXCESSIVE_LOAD, until one comes back.
        (AnnouncementBeyondTheRate, 0x9, 2_501),
        // How much of a publisher one subscriber's session may take, at the
        // relay: EXCESSIVE_LOAD, and not held, with RENDEZVOUS_TIMEOUT
        // neither. FETCH requests the relay has open on one publisher's
        // session: the same.
        (SubscribeBeyondOneSubscribersShare, 0x9, 1_001),
        (HeldSubscribeBeyondOneSubscribersShare, 0x9, 1_001),
        (SubscribeBeyondThePublishersRequests, 0x9, 1_001),
        (FetchBeyondOneSubscribersShare, 0x9, 1_001),
        (FetchBeyondThePublishersFetches, 0x9, 1_001),
        // PUBLISH for a track the relay knows already: EXCESSIVE_LOAD beyond
        // the sources a track has, DUPLICATE_SUBSCRIPTION from a source.
        (PublishBeyondTheSources, 0x9, 1_001),
        (PublishOfASourceThatGoesOn, 0x19, 0),
        // A PUBLISH whose Track Properties make the Track malformed:
        // MALFORMED_TRACK; one with a Mandatory Track Property:
        // UNSUPPORTED_EXTENSION.
        (PublishOfAMalformedTrack, 0x12, 0),
        (PublishWithAMandatoryTrackProperty, 0x33, 0),
        // A request that would double a subscription in the same role.
        (SecondSubscribeOfASession, 0x19, 0),
        // REQUEST_UPDATE for a request the relay has not answered yet:
        // NOT_SUPPORTED, the only response, and the request ends.
        (UpdateBeforeTheAnswer, 0x3, 0),
        // SUBSCRIBE_NAMESPACE (relay): EXCESSIVE_LOAD beyond the ones a
        // session may have, PREFIX_OVERLAP for one that overlaps.
        (NamespaceSubscriptionBeyondTheSessions, 0x9, 1_001),
        (OverlappingNamespaceSubscription, 0x30, 0),
        // REQUEST_UPDATEs that wait for the answer to a FETCH in the relay:
        // the FETCH is refused with EXCESSIVE_LOAD.
        (FetchUpdateBeyondTheHeld, 0x9, 1_001),
        // Subscribers while the publisher has not answered the relay's
        // SUBSCRIBE.
        (SubscriberBeyondTheWaiting, 0x9, 1_001),
        // A SUBSCRIBE for a track whose sources have all said PUBLISH_DONE
        // and whose last streams are still awaited: DOES_NOT_EXIST, after a
        // second.
        (SubscribeOfADrainingTrack, 0x10, 1_001),
    ];
    for (situation, error_code, retry_interval) in cases {
        let (actions, session, stream) = refusal(situation);
        let expected = refused(
            session,
            stream.0,
            RequestError {
                error_code,
                retry_interval,
                error_reason: ReasonPhrase::default(),
                redirect: None,
            },
        );
        assert_eq!(
            on_the_stream(&actions, session, stream),
            expected,
            "{situation:?}: {actions:?}"
        );
    }
}

/// What a subscriber gets of a publisher's `REQUEST_ERROR`: the Error Code,
/// whether the publisher's reason goes with it, and the Retry Interval.
pub(super) type Answer = (u64, bool, u64);

/// The publisher's `REQUEST_ERROR` with `error_code`, Retry Interval 7 and the
/// reason "why", and a Redirect where the code is REDIRECT.
fn publishers_refusal(error_code: u64) -> ControlMessage {
    ControlMessage::RequestError(RequestError {
        error_code,
        retry_interval: 7,
        error_reason: ReasonPhrase::new(b"why".to_vec()).unwrap(),
        redirect: (error_code == 0x34).then(|| xmoq_moqt::message::Redirect {
            connect_uri: vec![],
            full_track_name: track(b"elsewhere"),
        }),
    })
}

/// The one `REQUEST_ERROR` among `actions` on the request stream, written as
/// an [`Answer`]; the stream is finished after it.
pub(super) fn answer_on(actions: &[Action], session: SessionKey, stream: StreamKey) -> Answer {
    let on_it = on_the_stream(actions, session, stream);
    let [
        Action::SendRequest {
            message: ControlMessage::RequestError(error),
            ..
        },
        Action::FinishRequest { .. },
    ] = on_it.as_slice()
    else {
        panic!("no REQUEST_ERROR and FIN: {actions:?}");
    };
    let reason = error.error_reason.as_bytes();
    assert!(reason == b"why" || reason.is_empty(), "{error:?}");
    assert_eq!(error.redirect, None);
    (error.error_code, !reason.is_empty(), error.retry_interval)
}

/// What Alice gets where the publisher refuses the relay's SUBSCRIBE for her
/// with `error_code`.
fn answer_to_a_subscribe(error_code: u64) -> Answer {
    let mut relay = relay();
    announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
    assert!(asks(&subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"])));
    let actions = relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: publishers_refusal(error_code),
    });
    answer_on(&actions, ALICE, StreamKey(0))
}

/// The same for the relay's FETCH. The publisher is the one source of the
/// track and nobody else announced: nobody is asked after it.
fn answer_to_a_fetch(error_code: u64) -> Answer {
    let mut relay = relay_with_track();
    assert!(asks(&relay.step(opened(ALICE, 7, fetch_of_a(0)))));
    let actions = relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: publishers_refusal(error_code),
    });
    answer_on(&actions, ALICE, FETCH_STREAM)
}

/// The publisher's answer to the relay's SUBSCRIBE, and what the subscriber
/// of a FETCH gets from the publisher's answer.
#[test]
fn a_publishers_error_code_is_passed_on_or_replaced() {
    // (the publisher's Error Code; what the subscriber of the relay's
    //  SUBSCRIBE gets; what the subscriber of its FETCH gets). Passed on, a
    // code keeps the publisher's reason and Retry Interval (7); put in its
    // place, a code has the relay's Retry Interval for it and no reason.
    let cases: [(u64, Answer, Answer); 18] = [
        // About the track at the publisher or its load, and the generic
        // error: as they are, also where the relay would give the code
        // another Retry Interval of its own (1 with 0x2, 1,001 with 0x9 and
        // 0x10).
        (0x0, (0x0, true, 7), (0x0, true, 7)),
        (0x2, (0x2, true, 7), (0x2, true, 7)),
        (0x9, (0x9, true, 7), (0x9, true, 7)),
        (0x10, (0x10, true, 7), (0x10, true, 7)),
        (0x33, (0x33, true, 7), (0x33, true, 7)),
        // The three about tokens: as they are, until authorization is
        // decided.
        (0x1, (0x1, true, 7), (0x1, true, 7)),
        (0x4, (0x4, true, 7), (0x4, true, 7)),
        (0x5, (0x5, true, 7), (0x5, true, 7)),
        // NOT_SUPPORTED, GOING_AWAY and REDIRECT are true of the publisher
        // and the relay only: DOES_NOT_EXIST, after a second.
        (0x3, (0x10, false, 1_001), (0x10, false, 1_001)),
        (0x6, (0x10, false, 1_001), (0x10, false, 1_001)),
        (0x34, (0x10, false, 1_001), (0x10, false, 1_001)),
        // INVALID_RANGE and MALFORMED_TRACK answer a FETCH, and no SUBSCRIBE
        // without a range: INTERNAL_ERROR there, not to be retried.
        (0x11, (0x0, false, 0), (0x11, true, 7)),
        (0x12, (0x0, false, 0), (0x12, true, 7)),
        // Codes that answer no request of the relay's: UNINTERESTED,
        // PREFIX_OVERLAP, NAMESPACE_TOO_LARGE, INVALID_JOINING_REQUEST_ID,
        // and a code draft 18 does not have.
        (0x20, (0x0, false, 0), (0x0, false, 0)),
        (0x30, (0x0, false, 0), (0x0, false, 0)),
        (0x31, (0x0, false, 0), (0x0, false, 0)),
        (0x32, (0x0, false, 0), (0x0, false, 0)),
        (0x7777, (0x0, false, 0), (0x0, false, 0)),
    ];
    for (from_publisher, to_subscriber, to_fetcher) in cases {
        assert_eq!(
            answer_to_a_subscribe(from_publisher),
            to_subscriber,
            "SUBSCRIBE {from_publisher:#x}"
        );
        assert_eq!(
            answer_to_a_fetch(from_publisher),
            to_fetcher,
            "FETCH {from_publisher:#x}"
        );
    }
}

/// The publisher's fetch stream: a reset is passed on with the code it maps
/// to.
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
