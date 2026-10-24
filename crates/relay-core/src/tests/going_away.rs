//! The relay that is to stop: GOAWAY to every session (§3.6, §10.4).

use xmoq_moqt::message::GoAway;

use super::*;
use crate::GoingAway;

const URI: &[u8] = b"https://other.example/";

fn go_away(session: SessionKey, new_session_uri: &[u8], request_id: u64) -> Action {
    Action::SendControl {
        session,
        message: ControlMessage::GoAway(GoAway {
            new_session_uri: new_session_uri.to_vec(),
            timeout_ms: 10_000,
            request_id: Some(request_id),
        }),
    }
}

fn going_away() -> Event {
    Event::GoingAway(GoingAway {
        new_session_uri: URI.to_vec(),
        timeout_ms: 10_000,
    })
}

#[test]
fn every_session_is_told_once_with_the_smallest_request_id_not_seen() {
    let mut relay = relay_with_track();
    // The publisher used Request ID 0, Alice and Bob none.
    assert_eq!(
        relay.step(going_away()),
        [
            go_away(PUBLISHER, URI, 2),
            go_away(ALICE, URI, 0),
            go_away(BOB, URI, 0),
        ]
    );
    assert_eq!(relay.step(going_away()), []);
}

#[test]
fn a_session_that_opens_after_it_is_told_after_its_setup() {
    // (which end of the session the relay is, and the URI its GOAWAY
    //  carries: §10.4, "A client MUST send a zero-length New Session URI")
    let cases: [(Role, &[u8]); 2] = [(Role::Server, URI), (Role::Client, b"")];
    for (role, uri) in cases {
        let mut relay = relay_with_track();
        relay.step(going_away());
        let actions = relay.step(Event::SessionOpened {
            session: SessionKey(9),
            role,
        });
        let first = match role {
            Role::Server => 0,
            Role::Client => 1,
        };
        assert!(
            matches!(
                actions.first(),
                Some(Action::SendControl {
                    message: ControlMessage::Setup(_),
                    ..
                })
            ),
            "{role:?} {actions:?}"
        );
        assert_eq!(
            actions.get(1..),
            Some(&[go_away(SessionKey(9), uri, first)][..]),
            "{role:?}"
        );
    }
}

#[test]
fn a_request_after_it_is_refused_with_going_away_and_what_was_taken_goes_on() {
    let mut relay = relay_with_track();
    subscribe_to_track(&mut relay, ALICE);
    relay.step(going_away());
    // §10.4: "any requests that arrive after the GOAWAY, MUST be rejected
    // with REQUEST_ERROR using error code GOING_AWAY" (0x6).
    assert_eq!(
        subscribe_to_track(&mut relay, BOB),
        [
            Action::SendRequest {
                session: BOB,
                stream: StreamKey(0),
                message: request_error(0x6),
            },
            Action::FinishRequest {
                session: BOB,
                stream: StreamKey(0),
            },
        ]
    );
    // Alice's subscription is as it was: an Object still reaches her.
    data_stream_opened(&mut relay, 7);
    let actions = object_arrived(&mut relay, 7, 0);
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::SendObject { .. })),
        "{actions:?}"
    );
}
