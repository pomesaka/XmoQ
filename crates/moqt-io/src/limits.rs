//! The limits of this session and of `@xmoq/moqt` against
//! `fixtures/limits/limits.tsv`, which both test suites read.

use xmoq_moqt::{early, kvp, message, name, params, session};

use crate::{IdleSubgroupStreamTimeout, StreamConfig, client};

/// The Rust value of the row named `name`; `None` where the session has
/// none. Panics on a name it does not know, so that a row added to the table
/// is read here too.
fn ours(name: &str) -> Option<u64> {
    let config = StreamConfig::default();
    let millis = |time: std::time::Duration| u64::try_from(time.as_millis()).ok();
    let count = |value: usize| u64::try_from(value).ok();
    match name {
        "namespace-fields" => count(name::MAX_NAMESPACE_FIELDS),
        "full-track-name-bytes" => count(name::MAX_FULL_TRACK_NAME_LEN),
        "reason-phrase-bytes" => count(message::MAX_REASON_PHRASE_LEN),
        "new-session-uri-bytes" => count(message::MAX_NEW_SESSION_URI_LEN),
        "control-message-payload-bytes" => count(message::MAX_PAYLOAD_LEN),
        "close-reason-bytes" => count(crate::MAX_CLOSE_REASON_LEN),
        "key-value-pair-value-bytes" => count(kvp::MAX_VALUE_LEN),
        "key-value-pairs" => count(kvp::MAX_PAIRS),
        "retired-track-aliases" => count(session::RETIRED_ALIAS_LIMIT),
        "early-closed-track-aliases" => count(session::EARLY_CLOSED_ALIAS_LIMIT),
        "request-ids-ahead" => count(session::REQUEST_IDS_AHEAD_LIMIT),
        "unanswered-request-updates" => Some(u64::from(session::MAX_UNANSWERED_UPDATES)),
        "namespaces-of-a-subscription" => count(session::MAX_NAMESPACES_OF_A_SUBSCRIPTION),
        "early-objects-bytes" => count(early::MOST_HELD_BYTES),
        "early-object-overhead-bytes" => count(early::OBJECT_OVERHEAD),
        "early-property-overhead-bytes" => count(early::PROPERTY_OVERHEAD),
        "answer-wait-ms" => Some(params::ANSWER_WAIT_MS),
        "first-bytes-wait-ms" => millis(config.first_bytes_timeout),
        "idle-subgroup-stream-ms" => millis(IdleSubgroupStreamTimeout::default().get()),
        "setup-wait-ms" => Some(client::SETUP_WAIT_MS),
        "peer-uni-streams" => count(config.max_uni_streams),
        "peer-request-streams" => count(config.max_request_streams),
        "peer-requests" => count(session::MAX_PEER_REQUESTS),
        "subgroup-stream-bytes" => count(config.subgroup_buffer_limit),
        "session-subgroup-bytes" => count(config.session_buffer_limit),
        "ended-by-publish-done" => count(session::ENDED_BY_PUBLISH_DONE_LIMIT),
        "held-before-setup" => count(client::MOST_HELD_BEFORE_SETUP),
        "object-payload-bytes"
        | "object-properties-bytes"
        | "peer-fetches"
        | "held-fetch-updates" => None,
        other => panic!("a limit this test does not know: {other}"),
    }
}

#[test]
fn the_limits_are_as_the_shared_table_says() {
    let path = format!(
        "{}/../../fixtures/limits/limits.tsv",
        env!("CARGO_MANIFEST_DIR")
    );
    let table = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
    assert!(table.ends_with('\n') && !table.ends_with("\n\n"), "{path}");
    let rows: Vec<Vec<&str>> = table.lines().map(|row| row.split('\t').collect()).collect();
    // As many as the table has: a table that lost rows would pass.
    assert_eq!(rows.len(), 31, "{path}");
    let mut names = std::collections::HashSet::new();
    for row in rows {
        let [name, rust, _] = row.as_slice() else {
            panic!("a row of three columns: {row:?}");
        };
        assert!(names.insert(*name), "{name} twice");
        let expected = (*rust != "-").then(|| rust.parse::<u64>().expect("a number"));
        assert_eq!(ours(name), expected, "{name}");
    }
}
