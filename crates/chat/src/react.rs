//! What becomes of a reaction that reaches the gatherer: counted in its
//! room's while, or dropped (decided on 2026-10-26). The gatherer publishes
//! what a while counted on the room's `reactions`, and keeps nothing.

use std::collections::{BTreeMap, HashMap};

use crate::args::Limits;
use crate::rate::Window;
use xmoq_chat_tracks::names::{Person, Room};
use xmoq_chat_tracks::reactions::{Malformed, Reactions, read_reacted};

/// Why a reaction was not counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionDropped {
    /// Its payload is no reaction ([`xmoq_chat_tracks::reactions`]).
    Malformed(Malformed),
    /// Its sender sent `--reactions-per-second` in the second before it.
    TooFast,
    /// Its room's while counts `--reaction-kinds` other emoji already.
    ///
    /// WHY dropped and not counted under one emoji for the rest: the
    /// viewers would be shown an emoji nobody sent.
    TooManyKinds,
    /// Its sender is not followed: not admitted, or gone. The caller tells
    /// it, as of a line ([`crate::gather::Gatherer::follows`]).
    NotFollowed,
}

impl ReactionDropped {
    /// One word for the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Malformed(why) => why.word(),
            Self::TooFast => "too-fast",
            Self::TooManyKinds => "too-many-kinds",
            Self::NotFollowed => "not-followed",
        }
    }
}

/// The reactions of the rooms in the while under way, and how fast each
/// person sent them.
#[derive(Debug)]
pub struct Reacting {
    per_second: usize,
    kinds: usize,
    people: HashMap<Person, Window>,
    /// Each emoji of each room and how often it came, in the order each
    /// came first. WHY a list: the payload has them in that order
    /// ([`Reactions::counts`]), and a while has few.
    counts: BTreeMap<Room, Vec<(String, u64)>>,
}

impl Reacting {
    /// No reaction yet, with these limits.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            per_second: limits.reactions_per_second,
            kinds: limits.reaction_kinds,
            people: HashMap::new(),
            counts: BTreeMap::new(),
        }
    }

    /// A reaction of `person`, followed, that came at `now_ms` (a clock that
    /// does not go back). WHY the emoji is looked at before the rate: a
    /// reaction that is dropped is not counted against its sender, as of a
    /// line ([`crate::rate`]).
    ///
    /// # Errors
    ///
    /// Why it was dropped.
    pub fn react(
        &mut self,
        person: &Person,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<(), ReactionDropped> {
        let reacted = read_reacted(payload).map_err(ReactionDropped::Malformed)?;
        let counts = self.counts.entry(person.room.clone()).or_default();
        let known = counts.iter().any(|(emoji, _)| *emoji == reacted.emoji);
        if !known && counts.len() >= self.kinds {
            return Err(ReactionDropped::TooManyKinds);
        }
        let per_second = self.per_second;
        let window = self
            .people
            .entry(person.clone())
            .or_insert_with(|| Window::new(per_second));
        if !window.take(now_ms) {
            return Err(ReactionDropped::TooFast);
        }
        match counts.iter_mut().find(|(emoji, _)| *emoji == reacted.emoji) {
            Some((_, count)) => *count += 1,
            None => counts.push((reacted.emoji, 1)),
        }
        Ok(())
    }

    /// The person is followed no more: its window goes.
    pub fn leave(&mut self, person: &Person) {
        self.people.remove(person);
    }

    /// The room is published no more: what its while counted goes.
    pub fn forget(&mut self, room: &Room) {
        self.counts.remove(room);
    }

    /// The end of a while at `at_ms`, the gatherer's clock since the epoch:
    /// what each room that had reactions counted, and a new while.
    pub fn take(&mut self, at_ms: u64) -> Vec<(Room, Reactions)> {
        std::mem::take(&mut self.counts)
            .into_iter()
            .filter(|(_, counts)| !counts.is_empty())
            .map(|(room, counts)| (room, Reactions { counts, at: at_ms }))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmoq_chat_tracks::names::Kind;

    fn person(room: &str, name: &str) -> Person {
        Person {
            room: Room {
                kind: Kind::Broadcast,
                name: room.to_string(),
            },
            name: name.to_string(),
        }
    }

    fn reaction(emoji: &str) -> Vec<u8> {
        format!(r#"{{"emoji":"{emoji}","sentAt":5}}"#).into_bytes()
    }

    /// A reaction: who, its payload, when.
    type Step<'a> = (&'a str, Vec<u8>, u64);

    /// (case, the reactions, what became of each, what the room `r` counted
    /// at the end)
    type Case<'a> = (&'a str, Vec<Step<'a>>, Vec<&'a str>, &'a [(&'a str, u64)]);

    #[test]
    fn reactions_are_counted_in_their_room_or_dropped_for_their_reason() {
        let limits = Limits {
            reactions_per_second: 2,
            reaction_kinds: 2,
            ..Limits::default()
        };
        let cases: [Case; 5] = [
            (
                "one emoji twice, in the order it came",
                vec![("a", reaction("👍"), 0), ("b", reaction("👍"), 0)],
                vec!["ok", "ok"],
                &[("👍", 2)],
            ),
            (
                "a third in a second from one person",
                vec![
                    ("a", reaction("👍"), 0),
                    ("a", reaction("👍"), 500),
                    ("a", reaction("👍"), 999),
                    ("a", reaction("👍"), 1_000),
                ],
                vec!["ok", "ok", "too-fast", "ok"],
                &[("👍", 3)],
            ),
            (
                "a third emoji in a while; the two it has still count",
                vec![
                    ("a", reaction("👍"), 0),
                    ("b", reaction("❤"), 0),
                    ("c", reaction("👏"), 0),
                    ("c", reaction("👍"), 0),
                ],
                vec!["ok", "ok", "too-many-kinds", "ok"],
                &[("👍", 2), ("❤", 1)],
            ),
            (
                "a reaction that is dropped is not counted against its sender",
                vec![
                    ("a", b"{}".to_vec(), 0),
                    ("a", reaction(""), 0),
                    ("a", reaction("👍"), 0),
                    ("a", reaction("👍"), 0),
                ],
                vec!["emoji", "emoji", "ok", "ok"],
                &[("👍", 2)],
            ),
            ("nothing", vec![], vec![], &[]),
        ];
        for (case, steps, expected, counted) in cases {
            let mut reacting = Reacting::new(limits);
            let got: Vec<&str> = steps
                .iter()
                .map(|(name, payload, at)| {
                    reacting
                        .react(&person("r", name), payload, *at)
                        .map_or_else(ReactionDropped::word, |()| "ok")
                })
                .collect();
            assert_eq!(got, expected, "{case}");
            let taken = reacting.take(7);
            let counts: Vec<(&str, u64)> = taken
                .iter()
                .flat_map(|(_, reactions)| reactions.counts.iter())
                .map(|(emoji, count)| (emoji.as_str(), *count))
                .collect();
            assert_eq!(counts, counted, "{case}");
            // A while is taken once.
            assert!(reacting.take(8).is_empty(), "{case}");
        }
    }

    #[test]
    fn each_room_counts_its_own_while() {
        let mut reacting = Reacting::new(Limits::default());
        for (room, emoji) in [("r", "👍"), ("s", "👏"), ("r", "👍")] {
            reacting
                .react(&person(room, "a"), &reaction(emoji), 0)
                .unwrap();
        }
        reacting.forget(&person("s", "a").room);
        let taken: Vec<(String, Vec<(String, u64)>)> = reacting
            .take(9)
            .into_iter()
            .map(|(room, reactions)| (room.to_string(), reactions.counts))
            .collect();
        assert_eq!(
            taken,
            [("broadcast/r".to_string(), vec![("👍".to_string(), 2)])]
        );
    }
}
