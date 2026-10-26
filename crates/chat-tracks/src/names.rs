//! The namespaces and tracks of the chat: a room of a broadcast or of a
//! meeting is `xmoq/chat/<kind>/<room>` (`kind` `broadcast` or `meeting`),
//! where the gatherer publishes the track `lines`; a participant announces
//! `xmoq/chat/<kind>/<room>/<name>` and publishes the track `said` there.
//!
//! WHY the kind is a field of its own: a broadcast and a meeting are rooms
//! of two applications, each of its own names, and one gatherer can take
//! the rooms of both apart (`--room broadcast/<room>`).

use std::fmt;

use xmoq_moqt::UsageError;
use xmoq_moqt::name::{FullTrackName, TrackNamespace};

/// The fields every namespace of the chat begins with, which the gatherer
/// asks for with one `SUBSCRIBE_NAMESPACE` for every room of both kinds.
pub const PREFIX: [&str; 2] = ["xmoq", "chat"];

/// The track of a participant: what it says.
pub const SAID: &str = "said";

/// The track of a room: its lines, numbered.
pub const LINES: &str = "lines";

/// What a room is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// A broadcast's: one publishes, others watch.
    Broadcast,
    /// A meeting's: every participant publishes.
    Meeting,
}

impl Kind {
    /// Its field of the namespace.
    #[must_use]
    pub fn field(self) -> &'static str {
        match self {
            Self::Broadcast => "broadcast",
            Self::Meeting => "meeting",
        }
    }

    /// The kind whose field `field` is; `None` for any other.
    #[must_use]
    pub fn of(field: &[u8]) -> Option<Self> {
        [Self::Broadcast, Self::Meeting]
            .into_iter()
            .find(|kind| kind.field().as_bytes() == field)
    }
}

/// One room of the chat.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Room {
    /// The third field of the namespace.
    pub kind: Kind,
    /// The fourth.
    pub name: String,
}

impl Room {
    /// A room as text names it, `<kind>/<room>` (the gatherer's `--room`,
    /// and the key of its store): `broadcast/demo`, `meeting/m1`. `None`
    /// for another kind, or a room of no characters. The room may hold a
    /// `/` of its own: the kind ends at the first.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (kind, name) = text.split_once('/')?;
        let kind = Kind::of(kind.as_bytes())?;
        (!name.is_empty()).then(|| Self {
            kind,
            name: name.to_string(),
        })
    }
}

/// `<kind>/<room>`, as [`Room::parse`] reads it.
impl fmt::Display for Room {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.kind.field(), self.name)
    }
}

/// One participant of one room.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Person {
    /// Its room.
    pub room: Room,
    /// The fifth field of the namespace.
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

/// The room a namespace's fields after the prefix are: a kind and a room,
/// the room UTF-8. `None` for any other.
fn room_of(prefix: [&Vec<u8>; 2], kind: &[u8], room: &[u8]) -> Option<Room> {
    let ours = prefix
        .iter()
        .zip(PREFIX)
        .all(|(field, expected)| field.as_slice() == expected.as_bytes());
    if !ours {
        return None;
    }
    Some(Room {
        kind: Kind::of(kind)?,
        name: String::from_utf8(room.to_vec()).ok()?,
    })
}

/// The participant a namespace is of: one of exactly five fields, `xmoq`,
/// `chat`, a kind, the room and the name, the last two UTF-8. `None` for
/// any other.
///
/// WHY any other is passed over and not refused: the prefix is the chat's,
/// and what else is announced under it (the namespace of a room, a field
/// more, a kind this side does not know) is nobody's line. WHY UTF-8: the
/// name goes into the JSON of a line, and the room into the store as text;
/// bytes that are not UTF-8 would have to be changed there, and two names
/// could come out the same.
#[must_use]
pub fn person_of(namespace: &TrackNamespace) -> Option<Person> {
    let [xmoq, chat, kind, room, name] = namespace.fields() else {
        return None;
    };
    Some(Person {
        room: room_of([xmoq, chat], kind, room)?,
        name: String::from_utf8(name.clone()).ok()?,
    })
}

/// `xmoq/chat/<kind>/<room>/<name>`: what a participant announces.
///
/// # Errors
///
/// Fails where a field is empty or the namespace is longer than §2.4.1
/// allows.
pub fn person_namespace(room: &Room, name: &str) -> Result<TrackNamespace, UsageError> {
    let mut fields = room_namespace(room)?.fields().to_vec();
    fields.push(name.as_bytes().to_vec());
    TrackNamespace::new(fields)
}

/// `xmoq/chat/<kind>/<room>`: the namespace of the track `lines` of a room.
///
/// # Errors
///
/// As [`person_namespace`].
pub fn room_namespace(room: &Room) -> Result<TrackNamespace, UsageError> {
    TrackNamespace::new(vec![
        PREFIX[0].as_bytes().to_vec(),
        PREFIX[1].as_bytes().to_vec(),
        room.kind.field().as_bytes().to_vec(),
        room.name.as_bytes().to_vec(),
    ])
}

/// `xmoq/chat/<kind>/<room>/<name>`, `said`.
///
/// # Errors
///
/// As [`person_namespace`].
pub fn said_track(person: &Person) -> Result<FullTrackName, UsageError> {
    let namespace = person_namespace(&person.room, &person.name)?;
    FullTrackName::new(namespace, SAID.as_bytes().to_vec())
}

/// `xmoq/chat/<kind>/<room>`, `lines`.
///
/// # Errors
///
/// As [`person_namespace`].
pub fn lines_track(room: &Room) -> Result<FullTrackName, UsageError> {
    FullTrackName::new(room_namespace(room)?, LINES.as_bytes().to_vec())
}

/// The room whose `lines` the track is; `None` for any other track.
#[must_use]
pub fn room_of_lines(track: &FullTrackName) -> Option<Room> {
    let [xmoq, chat, kind, room] = track.namespace().fields() else {
        return None;
    };
    if track.name() != LINES.as_bytes() {
        return None;
    }
    room_of([xmoq, chat], kind, room)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `fixtures/chat-tracks/names.tsv`, which `@xmoq/chat`
    /// reads as well: (case, the fields of the namespace, the track's name
    /// where it is a track, what it is read as).
    fn rows() -> Vec<(String, Vec<String>, Option<String>, String)> {
        let table = include_str!("../../../fixtures/chat-tracks/names.tsv");
        assert!(table.ends_with('\n') && !table.ends_with("\n\n"));
        table
            .lines()
            .map(|row| {
                let [case, fields, track, read] = row.split('\t').collect::<Vec<_>>()[..] else {
                    panic!("a row of four columns: {row}");
                };
                let fields: Vec<String> = serde_json::from_str(fields).expect("a JSON array");
                let track = (track != "-").then(|| track.to_string());
                (case.to_string(), fields, track, read.to_string())
            })
            .collect()
    }

    fn namespace(fields: &[&[u8]]) -> TrackNamespace {
        TrackNamespace::new(fields.iter().map(|field| field.to_vec()).collect()).unwrap()
    }

    fn room(kind: Kind, name: &str) -> Room {
        Room {
            kind,
            name: name.to_string(),
        }
    }

    #[test]
    fn a_namespace_and_a_track_are_read_as_the_shared_table_says() {
        let mut read = 0;
        for (case, fields, track, expected) in rows() {
            let fields: Vec<&[u8]> = fields.iter().map(String::as_bytes).collect();
            let namespace = namespace(&fields);
            let got = match track {
                None => person_of(&namespace).map_or_else(
                    || "none".to_string(),
                    |person| format!("participant {} {}", person.room, person.name),
                ),
                Some(name) => {
                    let track = FullTrackName::new(namespace, name.into_bytes()).unwrap();
                    room_of_lines(&track)
                        .map_or_else(|| "none".to_string(), |room| format!("lines {room}"))
                }
            };
            assert_eq!(got, expected, "{case}");
            read += 1;
        }
        // A table that lost rows would pass.
        assert_eq!(read, 14);
    }

    #[test]
    fn a_field_that_is_not_utf8_is_nobodys() {
        // Bytes the text of the table cannot hold.
        let cases: [(&str, &[&[u8]]); 2] = [
            ("a name", &[b"xmoq", b"chat", b"broadcast", b"r", b"\xff"]),
            ("a room", &[b"xmoq", b"chat", b"meeting", b"\xc3", b"a"]),
        ];
        for (case, fields) in cases {
            assert_eq!(person_of(&namespace(fields)), None, "{case}");
        }
    }

    #[test]
    fn a_room_is_read_from_its_text_and_written_back() {
        let cases = [
            ("broadcast/demo", Some(room(Kind::Broadcast, "demo"))),
            ("meeting/m1", Some(room(Kind::Meeting, "m1"))),
            ("meeting/a/b", Some(room(Kind::Meeting, "a/b"))),
            ("demo", None),
            ("talk/demo", None),
            ("broadcast/", None),
            ("", None),
        ];
        for (text, expected) in cases {
            let parsed = Room::parse(text);
            assert_eq!(parsed, expected, "{text}");
            if let Some(parsed) = parsed {
                assert_eq!(parsed.to_string(), text);
            }
        }
    }

    #[test]
    fn the_tracks_made_are_the_ones_read() {
        for kind in [Kind::Broadcast, Kind::Meeting] {
            let alice = Person {
                room: room(kind, "r"),
                name: "alice".to_string(),
            };
            let said = said_track(&alice).unwrap();
            assert_eq!(person_of(said.namespace()), Some(alice));
            assert_eq!(said.name(), b"said");
            let lines = lines_track(&room(kind, "r")).unwrap();
            assert_eq!(room_of_lines(&lines), Some(room(kind, "r")));
        }
        assert_eq!(
            prefix().unwrap().fields(),
            [b"xmoq".to_vec(), b"chat".to_vec()]
        );
    }
}
