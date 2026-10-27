//! The namespaces of a meeting (`docs/scale-and-meetings.md`), as every
//! side uses them: every participant publishes a broadcast of its own under
//! `xmoq/meet/<room>/<id>` ([`participant`]), and is found with one
//! `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>` ([`prefix`]); the room's
//! seat gives a token to subscribe to `xmoq/meet/<room>/*`
//! ([`seat_subscribe`]). `@xmoq/meeting` (`packages/meeting/src/core/names.ts`)
//! has the same, and `fixtures/meeting-presence/namespaces.tsv` and
//! `participant-of.tsv` pin both.
//!
//! WHY a crate of its own, apart from `xmoq-viewer`: the names of each kind
//! of a room's tracks are in a crate of the same form (`xmoq-chat-tracks`,
//! `xmoq-transcript-tracks`), one that a part which is not a viewer can
//! depend on.

/// The first two fields of a meeting's namespace.
pub const MEET: [&str; 2] = ["xmoq", "meet"];

/// The prefix of a meeting's broadcasts: `xmoq/meet/<room>`.
#[must_use]
pub fn prefix(room: &str) -> Vec<String> {
    MEET.iter()
        .map(ToString::to_string)
        .chain(std::iter::once(room.to_string()))
        .collect()
}

/// The namespace the participant `id` publishes its broadcast under:
/// `xmoq/meet/<room>/<id>`.
#[must_use]
pub fn participant(room: &str, id: &str) -> Vec<String> {
    let mut namespace = prefix(room);
    namespace.push(id.to_string());
    namespace
}

/// The namespace of a seat's token to subscribe: `xmoq/meet/<room>/*`.
/// WHY the `*` and not the prefix: the relay takes a token for a
/// `SUBSCRIBE_NAMESPACE` only where its last field is `*`, which allows the
/// namespaces under the fields before it (`crates/relay/src/auth.rs`,
/// decided on 2026-10-26).
#[must_use]
pub fn seat_subscribe(room: &str) -> Vec<String> {
    participant(room, "*")
}

/// The participant a namespace is the broadcast of: the field after the
/// meeting's prefix, where the namespace has exactly one more. `None` for
/// any other, the prefix itself and a deeper one among them.
#[must_use]
pub fn participant_of<'a>(room: &str, namespace: &'a [String]) -> Option<&'a str> {
    match namespace {
        [xmoq, meet, of_room, id] if [xmoq.as_str(), meet.as_str()] == MEET && of_room == room => {
            Some(id.as_str())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `fixtures/meeting-presence/namespaces.tsv`, which
    /// `@xmoq/meeting` and the room read as well.
    #[test]
    fn the_namespaces_of_a_meeting() {
        let rows = include_str!("../../../fixtures/meeting-presence/namespaces.tsv");
        let mut read = 0;
        for row in rows.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, room, id, what, namespace] = fields[..] else {
                panic!("a row of five fields: {row}");
            };
            let got = match (what, id) {
                ("prefix", "-") => prefix(room),
                ("participant", id) if id != "-" => participant(room, id),
                ("seat-subscribe", "-") => seat_subscribe(room),
                _ => panic!("no such row: {row}"),
            };
            assert_eq!(got.join(" "), namespace, "{name}");
            read += 1;
        }
        assert_eq!(read, 6);
    }

    /// The rows of `fixtures/meeting-presence/participant-of.tsv`, which
    /// `@xmoq/meeting` reads as well.
    #[test]
    fn a_participant_is_the_field_after_the_prefix() {
        let rows = include_str!("../../../fixtures/meeting-presence/participant-of.tsv");
        let mut read = 0;
        for row in rows.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, room, namespace, expected] = fields[..] else {
                panic!("a row of four fields: {row}");
            };
            let namespace: Vec<String> = namespace.split(' ').map(ToString::to_string).collect();
            assert_eq!(
                participant_of(room, &namespace),
                (expected != "-").then_some(expected),
                "{name}"
            );
            read += 1;
        }
        assert_eq!(read, 6);
    }
}
