//! Who is there, by names the relay tells of with NAMESPACE and
//! `NAMESPACE_DONE` to a `SUBSCRIBE_NAMESPACE` (draft 18 §6.1), across the
//! sessions of a reader: on a new session the relay tells of everybody
//! again, and those it does not tell of are taken to have left.

use std::collections::BTreeMap;

/// How long after a `SUBSCRIBE_NAMESPACE` of a new session was answered
/// those not told of again are taken to have left. **Provisional**; the
/// reader's chat (`crates/agent/src/presence.rs`) goes by it as well.
///
/// WHY: the relay tells of every namespace under the prefix when it is
/// asked (§6.1), and nobody is told to have left while the reader was away;
/// those it does not tell of again left meanwhile. WHY a wait and not the
/// answer: draft 18 has no message that says the namespaces there were are
/// all told.
pub const SETTLE_MS: u64 = 2_000;

/// Somebody came or left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Told of, and not there before.
    Came(String),
    /// Told to be gone, or not told of again on a new session.
    Left(String),
    /// Told of while as many are there as are kept: left out.
    TooMany(String),
}

/// Who is there, as told so far.
#[derive(Debug)]
pub struct Presence {
    /// The most that are kept. At least 1.
    most: usize,
    /// Each name, and whether the relay told of it on this session.
    here: BTreeMap<String, bool>,
}

impl Presence {
    /// Nobody yet, with room for `most` (taken for 1 where it is 0).
    #[must_use]
    pub fn new(most: usize) -> Self {
        Self {
            most: most.max(1),
            here: BTreeMap::new(),
        }
    }

    /// Whether `name` is there.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.here.contains_key(name)
    }

    /// Who is there, in the order of their names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.here.keys().map(String::as_str)
    }

    /// The relay told of `name` (NAMESPACE).
    ///
    /// WHY one more than is kept is told of and not kept: the relay says who
    /// announces, and how many is theirs to choose; the reader keeps what it
    /// can, and says what it left out.
    pub fn named(&mut self, name: &str) -> Option<Change> {
        if let Some(told) = self.here.get_mut(name) {
            *told = true;
            return None;
        }
        if self.here.len() >= self.most {
            return Some(Change::TooMany(name.to_string()));
        }
        self.here.insert(name.to_string(), true);
        Some(Change::Came(name.to_string()))
    }

    /// The relay told that `name` is gone (`NAMESPACE_DONE`).
    pub fn done(&mut self, name: &str) -> Option<Change> {
        self.here
            .remove(name)
            .map(|_| Change::Left(name.to_string()))
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
    pub fn settled(&mut self) -> Vec<Change> {
        let gone: Vec<String> = self
            .here
            .iter()
            .filter(|(_, told)| !**told)
            .map(|(name, _)| name.clone())
            .collect();
        for name in &gone {
            self.here.remove(name);
        }
        gone.into_iter().map(Change::Left).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    enum Step {
        Named(&'static str),
        Done(&'static str),
        SessionEnded,
        Settled,
    }

    fn came(name: &str) -> Change {
        Change::Came(name.to_string())
    }

    fn left(name: &str) -> Change {
        Change::Left(name.to_string())
    }

    #[test]
    fn who_comes_and_who_leaves() {
        use Step::{Done, Named, SessionEnded, Settled};
        // (what happens, with room for 2; the changes of all of it)
        let cases: [(&[Step], Vec<Change>); 9] = [
            (&[Named("a")], vec![came("a")]),
            (&[Named("a"), Done("a")], vec![came("a"), left("a")]),
            // Told twice: one came.
            (&[Named("a"), Named("a")], vec![came("a")]),
            // Gone without having come.
            (&[Done("a")], vec![]),
            // A new session tells of a again and not of b: b left meanwhile.
            (
                &[Named("a"), Named("b"), SessionEnded, Named("a"), Settled],
                vec![came("a"), came("b"), left("b")],
            ),
            (&[SessionEnded, Named("c"), Settled], vec![came("c")]),
            (&[Named("a"), Settled], vec![came("a")]),
            // One more than there is room for is left out, and comes once
            // there is room.
            (
                &[Named("a"), Named("b"), Named("c"), Done("a"), Named("c")],
                vec![
                    came("a"),
                    came("b"),
                    Change::TooMany("c".to_string()),
                    left("a"),
                    came("c"),
                ],
            ),
            // Told of again while full: still there, nothing more.
            (
                &[Named("a"), Named("b"), Named("b")],
                vec![came("a"), came("b")],
            ),
        ];
        for (steps, expected) in cases {
            let mut presence = Presence::new(2);
            let mut changes = Vec::new();
            for step in steps {
                match *step {
                    Named(name) => changes.extend(presence.named(name)),
                    Done(name) => changes.extend(presence.done(name)),
                    SessionEnded => presence.session_ended(),
                    Settled => changes.extend(presence.settled()),
                }
            }
            assert_eq!(changes, expected, "{steps:?}");
        }
    }

    #[test]
    fn room_for_none_is_room_for_one() {
        let mut presence = Presence::new(0);
        assert_eq!(presence.named("a"), Some(came("a")));
        assert_eq!(presence.named("b"), Some(Change::TooMany("b".to_string())));
        assert_eq!(presence.names().collect::<Vec<_>>(), ["a"]);
    }
}
