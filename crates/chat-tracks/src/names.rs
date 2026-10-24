//! The namespaces and tracks of the chat (decided on 2026-10-24): a
//! participant announces `xmoq/chat/<room>/<name>` and publishes the track
//! `said` there; the gatherer publishes the track `lines` of
//! `xmoq/chat/<room>`.

use xmoq_moqt::UsageError;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};

/// The fields every namespace of the chat begins with, which the gatherer
/// asks for with one `SUBSCRIBE_NAMESPACE` for every room.
pub const PREFIX: [&str; 2] = ["xmoq", "chat"];

/// The track of a participant: what it says.
pub const SAID: &str = "said";

/// The track of a room: its lines, numbered.
pub const LINES: &str = "lines";

/// One participant of one room.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Person {
    /// The third field of the namespace.
    pub room: String,
    /// The fourth.
    pub name: String,
}

/// `xmoq/chat`.
///
/// # Errors
///
/// None in fact: two short fields are a namespace.
pub fn prefix() -> Result<TrackNamespace, UsageError> {
    TrackNamespace::new(
        PREFIX
            .iter()
            .map(|field| field.as_bytes().to_vec())
            .collect(),
    )
}

/// The participant a namespace is of: one of exactly four fields, `xmoq`,
/// `chat`, the room and the name, the last two UTF-8. `None` for any other.
///
/// WHY any other is passed over and not refused: the prefix is the chat's,
/// and what else is announced under it (the namespace of a room, a field
/// more) is nobody's line. WHY UTF-8: the name goes into the JSON of a line,
/// and the room into the store as text; bytes that are not UTF-8 would have
/// to be changed there, and two names could come out the same.
#[must_use]
pub fn person_of(namespace: &TrackNamespace) -> Option<Person> {
    let [xmoq, chat, room, name] = namespace.fields() else {
        return None;
    };
    if xmoq.as_slice() != PREFIX[0].as_bytes() || chat.as_slice() != PREFIX[1].as_bytes() {
        return None;
    }
    Some(Person {
        room: String::from_utf8(room.clone()).ok()?,
        name: String::from_utf8(name.clone()).ok()?,
    })
}

/// `xmoq/chat/<room>/<name>`: what a participant announces.
///
/// # Errors
///
/// Fails where a field is empty or the namespace is longer than §2.4.1
/// allows.
pub fn person_namespace(room: &str, name: &str) -> Result<TrackNamespace, UsageError> {
    TrackNamespace::new(vec![
        PREFIX[0].as_bytes().to_vec(),
        PREFIX[1].as_bytes().to_vec(),
        room.as_bytes().to_vec(),
        name.as_bytes().to_vec(),
    ])
}

/// `xmoq/chat/<room>`: the namespace of the track `lines` of a room.
///
/// # Errors
///
/// As [`person_namespace`].
pub fn room_namespace(room: &str) -> Result<TrackNamespace, UsageError> {
    TrackNamespace::new(vec![
        PREFIX[0].as_bytes().to_vec(),
        PREFIX[1].as_bytes().to_vec(),
        room.as_bytes().to_vec(),
    ])
}

/// `xmoq/chat/<room>/<name>`, `said`.
///
/// # Errors
///
/// As [`person_namespace`].
pub fn said_track(person: &Person) -> Result<FullTrackName, UsageError> {
    let namespace = person_namespace(&person.room, &person.name)?;
    FullTrackName::new(namespace, SAID.as_bytes().to_vec())
}

/// `xmoq/chat/<room>`, `lines`.
///
/// # Errors
///
/// As [`person_namespace`].
pub fn lines_track(room: &str) -> Result<FullTrackName, UsageError> {
    FullTrackName::new(room_namespace(room)?, LINES.as_bytes().to_vec())
}

/// The room whose `lines` the track is; `None` for any other track.
#[must_use]
pub fn room_of_lines(track: &FullTrackName) -> Option<String> {
    let [xmoq, chat, room] = track.namespace().fields() else {
        return None;
    };
    let ours = xmoq.as_slice() == PREFIX[0].as_bytes()
        && chat.as_slice() == PREFIX[1].as_bytes()
        && track.name() == LINES.as_bytes();
    ours.then(|| String::from_utf8(room.clone()).ok())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(fields: &[&[u8]]) -> TrackNamespace {
        TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
    }

    fn person(room: &str, name: &str) -> Person {
        Person {
            room: room.to_string(),
            name: name.to_string(),
        }
    }

    /// (case, the fields, the participant)
    type Case = (&'static str, &'static [&'static [u8]], Option<Person>);

    #[test]
    fn a_participant_is_a_namespace_of_four_fields_under_the_prefix() {
        let cases: [Case; 7] = [
            (
                "room and name",
                &[b"xmoq", b"chat", b"r", b"alice"],
                Some(person("r", "alice")),
            ),
            ("the namespace of a room", &[b"xmoq", b"chat", b"r"], None),
            ("a field more", &[b"xmoq", b"chat", b"r", b"a", b"b"], None),
            ("another prefix", &[b"xmoq", b"talk", b"r", b"a"], None),
            ("another first field", &[b"moq", b"chat", b"r", b"a"], None),
            (
                "a name that is not UTF-8",
                &[b"xmoq", b"chat", b"r", b"\xff"],
                None,
            ),
            (
                "a room that is not UTF-8",
                &[b"xmoq", b"chat", b"\xc3", b"a"],
                None,
            ),
        ];
        for (case, fields, expected) in cases {
            assert_eq!(person_of(&namespace(fields)), expected, "{case}");
        }
    }

    #[test]
    fn the_track_of_a_rooms_lines_names_its_room() {
        let lines = |fields: &[&[u8]], name: &[u8]| {
            FullTrackName::new(namespace(fields), name.to_vec()).unwrap()
        };
        let cases: [(&str, FullTrackName, Option<&str>); 5] = [
            (
                "lines of a room",
                lines(&[b"xmoq", b"chat", b"r"], b"lines"),
                Some("r"),
            ),
            (
                "another track",
                lines(&[b"xmoq", b"chat", b"r"], b"said"),
                None,
            ),
            (
                "a participant's",
                lines(&[b"xmoq", b"chat", b"r", b"a"], b"lines"),
                None,
            ),
            (
                "another prefix",
                lines(&[b"xmoq", b"talk", b"r"], b"lines"),
                None,
            ),
            (
                "a room that is not UTF-8",
                lines(&[b"xmoq", b"chat", b"\xff"], b"lines"),
                None,
            ),
        ];
        for (case, track, expected) in cases {
            assert_eq!(room_of_lines(&track).as_deref(), expected, "{case}");
        }
    }

    #[test]
    fn the_tracks_made_are_the_ones_read() {
        let alice = person("r", "alice");
        let said = said_track(&alice).unwrap();
        assert_eq!(person_of(said.namespace()), Some(alice));
        assert_eq!(said.name(), b"said");
        assert_eq!(
            room_of_lines(&lines_track("r").unwrap()).as_deref(),
            Some("r")
        );
        assert_eq!(
            prefix().unwrap().fields(),
            [b"xmoq".to_vec(), b"chat".to_vec()]
        );
    }
}
