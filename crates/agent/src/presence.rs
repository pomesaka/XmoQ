//! Who is in the chat of the room: those whose namespace
//! `xmoq/chat/<kind>/<room>/<name>` the relay tells of, to a
//! `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<kind>/<room>`, as the gatherer
//! finds them (`crates/chat/src/lib.rs`). Only those who announce a namespace for
//! the chat are seen: a viewer that does not take part in it is not. The
//! rule across sessions is `xmoq_viewer::presence`'s, which a meeting's
//! participants go by as well; here are the room and the reader itself.

use xmoq_chat_tracks::names::{Person, Room};
use xmoq_viewer::presence::{self, Change};

use crate::events::Event;

pub use xmoq_viewer::presence::SETTLE_MS;

/// The most in the chat who are kept: as many namespaces as the own relay
/// tells of to one `SUBSCRIBE_NAMESPACE` (4,096, decided on 2026-10-24).
/// **Provisional.**
pub const MOST_IN_THE_CHAT: usize = 4_096;

/// Who is in the chat, as told so far.
#[derive(Debug)]
pub struct Presence {
    room: Room,
    me: String,
    people: presence::Presence,
}

/// The event of a change.
fn event_of(change: Change) -> Event {
    match change {
        Change::Came(name) => Event::Joined { name },
        Change::Left(name) => Event::Left { name },
        Change::TooMany(name) => Event::Error {
            message: format!(
                "who is in the chat: {name} came, and more than {MOST_IN_THE_CHAT} are kept"
            ),
        },
    }
}

impl Presence {
    /// Nobody yet, in `room`, where the reader is `me`.
    #[must_use]
    pub fn new(room: &Room, me: &str) -> Self {
        Self {
            room: room.clone(),
            me: me.to_string(),
            people: presence::Presence::new(MOST_IN_THE_CHAT),
        }
    }

    /// Whether `person` is of the room and is somebody other than the
    /// reader. WHY not the reader: it knows it is there, and its own coming
    /// back on a new session would read as somebody's.
    fn other(&self, person: &Person) -> bool {
        person.room == self.room && person.name != self.me
    }

    /// The relay told of `person`'s namespace (`NAMESPACE`).
    pub fn named(&mut self, person: &Person) -> Option<Event> {
        if !self.other(person) {
            return None;
        }
        self.people.named(&person.name).map(event_of)
    }

    /// The relay told that `person`'s namespace is gone (`NAMESPACE_DONE`).
    pub fn done(&mut self, person: &Person) -> Option<Event> {
        if !self.other(person) {
            return None;
        }
        self.people.done(&person.name).map(event_of)
    }

    /// The session ended: whoever is told of on the next one is still
    /// there, and nobody comes or leaves for it.
    pub fn session_ended(&mut self) {
        self.people.session_ended();
    }

    /// [`SETTLE_MS`] after the `SUBSCRIBE_NAMESPACE` of a session was
    /// answered: those not told of on it have left.
    pub fn settled(&mut self) -> Vec<Event> {
        self.people.settled().into_iter().map(event_of).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    enum Step {
        Named(&'static str, &'static str),
        Done(&'static str, &'static str),
        SessionEnded,
        Settled,
    }

    /// A person of the room `room`, as `<kind>/<room>`.
    fn person(room: &str, name: &str) -> Person {
        Person {
            room: Room::parse(room).expect("a room"),
            name: name.to_string(),
        }
    }

    fn joined(name: &str) -> Event {
        Event::Joined {
            name: name.to_string(),
        }
    }

    fn left(name: &str) -> Event {
        Event::Left {
            name: name.to_string(),
        }
    }

    /// What this adds to `xmoq_viewer::presence`, whose table has the rule
    /// across sessions: the room, the reader itself, and the events.
    #[test]
    fn who_comes_and_who_leaves_in_the_room() {
        use Step::{Done, Named, SessionEnded, Settled};
        // (what happens, in the room `broadcast/r` as `me`; the events of all
        // of it)
        let cases: [(&[Step], Vec<Event>); 6] = [
            (
                &[Named("broadcast/r", "alice"), Done("broadcast/r", "alice")],
                vec![joined("alice"), left("alice")],
            ),
            // The reader itself, and somebody of another room.
            (
                &[
                    Named("broadcast/r", "me"),
                    Named("broadcast/other", "bob"),
                    Done("broadcast/r", "me"),
                ],
                vec![],
            ),
            (
                &[
                    Named("broadcast/other", "bob"),
                    Done("broadcast/other", "bob"),
                ],
                vec![],
            ),
            // A room of the same name and of the other kind is another.
            (
                &[Named("meeting/r", "bob"), Done("meeting/r", "bob")],
                vec![],
            ),
            // The reader itself is not taken to have left on a new session.
            (&[Named("broadcast/r", "me"), SessionEnded, Settled], vec![]),
            (
                &[
                    Named("broadcast/r", "alice"),
                    Named("broadcast/r", "bob"),
                    SessionEnded,
                    Named("broadcast/r", "alice"),
                    Settled,
                ],
                vec![joined("alice"), joined("bob"), left("bob")],
            ),
        ];
        for (steps, expected) in cases {
            let mut presence = Presence::new(&person("broadcast/r", "me").room, "me");
            let mut events = Vec::new();
            for step in steps {
                match *step {
                    Named(room, name) => events.extend(presence.named(&person(room, name))),
                    Done(room, name) => events.extend(presence.done(&person(room, name))),
                    SessionEnded => presence.session_ended(),
                    Settled => events.extend(presence.settled()),
                }
            }
            assert_eq!(events, expected, "{steps:?}");
        }
    }

    #[test]
    fn more_than_are_kept_is_an_error() {
        assert_eq!(
            event_of(Change::TooMany("carol".to_string())),
            Event::Error {
                message: "who is in the chat: carol came, and more than 4096 are kept".to_string()
            }
        );
    }
}
