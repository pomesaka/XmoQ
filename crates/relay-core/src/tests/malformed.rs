//! A publisher's answer whose Track Properties the relay cannot take
//! (§2.5.1, §12.7).

use xmoq_moqt::message::FetchOk;

use super::answers::{Answer, FETCH_STREAM, answer_on, asks, at, fetch_of_a, opened};
use super::routing::{announce, relay, subscribe_to};
use super::*;

/// The relay's request that the publisher answers.
#[derive(Debug, Clone, Copy)]
enum Asked {
    /// Its SUBSCRIBE, for a SUBSCRIBE of Alice that waits for the answer.
    Subscribe,
    /// Its FETCH, for a FETCH of Alice.
    Fetch,
}

/// The publisher answers `asked` with these Track Properties: the code the
/// relay cancels its request with, and what Alice is answered.
fn after_an_answer_with(asked: Asked, track_property: KeyValuePair) -> (u32, Answer) {
    let (mut relay, answer, alices) = match asked {
        Asked::Subscribe => {
            let mut relay = relay();
            announce(&mut relay, PUBLISHER, 0, &[b"xmoq"]);
            assert!(asks(&subscribe_to(&mut relay, ALICE, 0, &[b"xmoq"])));
            let answer = ControlMessage::SubscribeOk(SubscribeOk {
                track_alias: UPSTREAM_ALIAS,
                parameters: vec![],
                track_properties: vec![track_property],
            });
            (relay, answer, StreamKey(0))
        }
        Asked::Fetch => {
            let mut relay = relay_with_track();
            assert!(asks(&relay.step(opened(ALICE, 7, fetch_of_a(0)))));
            let answer = ControlMessage::FetchOk(FetchOk {
                end_of_track: false,
                end_location: at(2, 0),
                parameters: vec![],
                track_properties: vec![track_property],
            });
            (relay, answer, FETCH_STREAM)
        }
    };
    let actions = relay.step(Event::OwnRequestMessage {
        session: PUBLISHER,
        request: OwnRequestKey(0),
        message: answer,
    });
    let cancelled: Vec<u32> = actions
        .iter()
        .filter_map(|action| match action {
            Action::CancelOwnRequest {
                session: PUBLISHER,
                request: OwnRequestKey(0),
                code,
            } => Some(*code),
            _ => None,
        })
        .collect();
    let [code] = cancelled[..] else {
        panic!("not cancelled once: {actions:?}");
    };
    (code, answer_on(&actions, ALICE, alices))
}

/// A PUBLISH, or an upstream `SUBSCRIBE_OK`, whose Track Properties make the
/// Track malformed (§12.7), in Rust alone; the same for one with a Mandatory
/// Track Property (§2.5.1). The PUBLISH is a row of the table of refusals.
#[test]
fn an_answer_whose_track_the_relay_cannot_take_is_cancelled_and_its_subscriber_refused() {
    // Immutable Properties inside Immutable Properties (§12.7), and a
    // Mandatory Track Property nobody here knows.
    let malformed = || KeyValuePair::bytes(0x0b, vec![0x0b, 0x00]).unwrap();
    let mandatory = || KeyValuePair::varint(0x4000, 1).unwrap();
    // (the relay's request; the Track Property of the answer; the Stream
    //  Reset Error Code it is cancelled with; Alice's REQUEST_ERROR, as
    //  (Error Code, whether it has a reason, Retry Interval))
    let cases = [
        // MALFORMED_TRACK (0x12) both ways.
        (Asked::Subscribe, malformed(), 0x12, (0x12, false, 0)),
        (Asked::Fetch, malformed(), 0x12, (0x12, false, 0)),
        // CANCELLED (0x1), and UNSUPPORTED_EXTENSION (0x33).
        (Asked::Subscribe, mandatory(), 0x1, (0x33, false, 0)),
        (Asked::Fetch, mandatory(), 0x1, (0x33, false, 0)),
    ];
    for (asked, property, cancelled, refused) in cases {
        assert_eq!(
            after_an_answer_with(asked, property.clone()),
            (cancelled, refused),
            "{asked:?} {property:?}"
        );
    }
}
