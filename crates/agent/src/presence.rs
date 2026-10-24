//! Who is in the chat of the room: those whose namespace
//! `xmoq/chat/<room>/<name>` the relay tells of, to a `SUBSCRIBE_NAMESPACE`
//! for `xmoq/chat/<room>` (decided on 2026-10-25), as the gatherer finds
//! them (`crates/chat/README.md`). Only those who announce a namespace for
//! the chat are seen: a viewer that does not take part in it is not.

use std::collections::BTreeMap;

use xmoq_chat_tracks::names::Person;

use crate::events::Event;

/// How long after a `SUBSCRIBE_NAMESPACE` of a new session was answered
/// those not told of again are taken to have left. **Provisional.**
///
/// WHY: the relay tells of every namespace under the prefix when it is
/// asked (§6.1), and nobody is told to have left while the reader was away;
/// those it does not tell of again left meanwhile. WHY a wait and not the
/// answer: draft 18 has no message that says the namespaces there were are
/// all told.
pub const SETTLE_MS: u64 = 2_000;

/// Who is in the chat, as told so far.
#[derive(Debug)]
pub struct Presence {
    room: String,
    me: String,
    /// Each name, and whether the relay told of it on this session.
    here: BTreeMap<String, bool>,
}

impl Presence {
    /// Nobody yet, in `room`, where the reader is `me`.
    #[must_use]
    pub fn new(room: &str, me: &str) -> Self {
        Self {
            room: room.to_string(),
            me: me.to_string(),
            here: BTreeMap::new(),
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
        let known = self.here.insert(person.name.clone(), true).is_some();
        (!known).then(|| Event::Joined {
            name: person.name.clone(),
        })
    }

    /// The relay told that `person`'s namespace is gone (`NAMESPACE_DONE`).
    pub fn done(&mut self, person: &Person) -> Option<Event> {
        if !self.other(person) {
            return None;
        }
        self.here.remove(&person.name).map(|_| Event::Left {
            name: person.name.clone(),
        })
    }

    /// The session ended: whoever is told of on the next one is still
    /// there, and nobody comes or leaves for it.
    pub fn session_ended(&mut self) {
        for told in self.here.values_mut() {
            *told = false;
        }
    }

    /// [`SETTLE_MS`] after the `SUBSCRIBE_NAMESPACE` of a session was
    /// answered: those not told of on it have left.
    pub fn settled(&mut self) -> Vec<Event> {
        let gone: Vec<String> = self
            .here
            .iter()
            .filter(|(_, told)| !**told)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &gone {
            self.here.remove(name);
        }
        gone.into_iter().map(|name| Event::Left { name }).collect()
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

    fn person(room: &str, name: &str) -> Person {
        Person {
            room: room.to_string(),
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

    #[test]
    fn who_comes_and_who_leaves() {
        use Step::{Done, Named, SessionEnded, Settled};
        // (what happens, in the room `r` as `me`; the events of all of it)
        let cases: [(&[Step], Vec<Event>); 8] = [
            (&[Named("r", "alice")], vec![joined("alice")]),
            (
                &[Named("r", "alice"), Done("r", "alice")],
                vec![joined("alice"), left("alice")],
            ),
            // Told twice: one came.
            (
                &[Named("r", "alice"), Named("r", "alice")],
                vec![joined("alice")],
            ),
            // Gone without having come.
            (&[Done("r", "alice")], vec![]),
            // The reader itself, and somebody of another room.
            (
                &[Named("r", "me"), Named("other", "bob"), Done("r", "me")],
                vec![],
            ),
            // A new session tells of alice again and not of bob: bob left
            // meanwhile, and alice was there all along.
            (
                &[
                    Named("r", "alice"),
                    Named("r", "bob"),
                    SessionEnded,
                    Named("r", "alice"),
                    Settled,
                ],
                vec![joined("alice"), joined("bob"), left("bob")],
            ),
            // Somebody new on the new session came.
            (
                &[SessionEnded, Named("r", "carol"), Settled],
                vec![joined("carol")],
            ),
            // Settled on a session that told of everybody: nobody left.
            (&[Named("r", "alice"), Settled], vec![joined("alice")]),
        ];
        for (steps, expected) in cases {
            let mut presence = Presence::new("r", "me");
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
}
