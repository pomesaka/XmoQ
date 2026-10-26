//! What becomes of a line that reaches the gatherer: taken and numbered, or
//! dropped and counted.

use std::collections::{BTreeMap, HashMap};

use crate::args::Limits;
use crate::rate::Window;
use crate::rooms::Rooms;
use xmoq_chat_tracks::names::Person;
use xmoq_chat_tracks::payloads::{Line, Malformed, read_said};

/// The participants the gatherer follows at once. One more is passed over
/// and counted: it is not subscribed to, and nothing it says is taken.
///
/// WHY 4,096: the most namespaces the own relay tells one
/// `SUBSCRIBE_NAMESPACE` of (`MAX_NAMESPACES_OF_A_SUBSCRIPTION` of
/// `crates/relay-core/src/discovery.rs`), and the most a session of
/// `xmoq-moqt` keeps of one. WHY a limit: each has a subscription and a
/// window here, and the names are whoever announces them. **Provisional.**
pub const MAX_PEOPLE: usize = 4_096;

/// Why a line was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// Its payload is not a line ([`xmoq_chat_tracks::payloads`]).
    ///
    /// WHY dropped, also for a `sentAt` alone that is not of its form, and
    /// not passed on with the gatherer's clock or without the member: the
    /// viewers would then have lines of two forms, and a line whose time
    /// was made up by the gatherer would say that the sender said so.
    Malformed(Malformed),
    /// Its text is longer than `--text-bytes`.
    TooLong,
    /// Its sender said `--lines-per-second` lines in the second before it.
    TooFast,
    /// Its sender is not followed: not admitted, or gone.
    NotFollowed,
    /// Its room has no number left (2^64 lines).
    NoNumber,
}

impl Dropped {
    /// One word for the log, and the key of its count.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Malformed(why) => why.word(),
            Self::TooLong => "too-long",
            Self::TooFast => "too-fast",
            Self::NotFollowed => "not-followed",
            Self::NoNumber => "no-number",
        }
    }
}

/// What became of the room of a participant who left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    /// The participant was not followed.
    NotFollowed,
    /// Somebody else is followed in the room.
    RoomStays,
    /// Nobody is: the room is numbered no more.
    RoomEmpty,
}

/// Whether a participant is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admitted {
    /// From now on.
    Now,
    /// It was before.
    Already,
    /// [`MAX_PEOPLE`] are followed.
    TooManyPeople,
    /// Its room is not numbered ([`Rooms::add`]): [`crate::rooms::MAX_ROOMS`]
    /// are.
    NoRoom,
}

/// The participants that are followed, the numbering of their rooms, and
/// the counts of what was dropped.
#[derive(Debug)]
pub struct Gatherer {
    limits: Limits,
    people: HashMap<Person, Window>,
    /// The numbering, which the caller adds rooms to from the store.
    pub rooms: Rooms,
    dropped: BTreeMap<&'static str, u64>,
}

impl Gatherer {
    /// A gatherer with these limits, which follows nobody yet.
    #[must_use]
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            people: HashMap::new(),
            rooms: Rooms::default(),
            dropped: BTreeMap::new(),
        }
    }

    /// The limits.
    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Follows a participant whose room is numbered.
    pub fn admit(&mut self, person: &Person) -> Admitted {
        if self.people.contains_key(person) {
            return Admitted::Already;
        }
        if !self.rooms.knows(&person.room) {
            return Admitted::NoRoom;
        }
        if self.people.len() >= MAX_PEOPLE {
            return Admitted::TooManyPeople;
        }
        self.people
            .insert(person.clone(), Window::new(self.limits.lines_per_second));
        Admitted::Now
    }

    /// Follows the participant no more: its window goes with it. Where
    /// nobody else is followed in its room, the room is numbered no more
    /// ([`crate::rooms`]), and the caller ends its PUBLISH.
    pub fn leave(&mut self, person: &Person) -> Left {
        if self.people.remove(person).is_none() {
            return Left::NotFollowed;
        }
        if self.people.keys().any(|other| other.room == person.room) {
            return Left::RoomStays;
        }
        self.rooms.remove(&person.room);
        Left::RoomEmpty
    }

    /// Follows nobody and numbers no room, as before the first NAMESPACE.
    /// WHY the rooms as well: a new session is told again who is in them,
    /// and numbers a room on from the store when somebody is.
    pub fn leave_everybody(&mut self) {
        self.people.clear();
        self.rooms.clear();
    }

    /// Whether the participant is followed.
    #[must_use]
    pub fn follows(&self, person: &Person) -> bool {
        self.people.contains_key(person)
    }

    /// Takes what `person` said, `payload` of an Object of its `said`, at
    /// `now_ms` of a clock that does not go back and `wall_ms` of the
    /// gatherer's clock since the epoch. A line that is taken has its
    /// number, and is counted against its sender; the caller keeps it, or
    /// gives the number back ([`Rooms::give_back`]).
    ///
    /// # Errors
    ///
    /// Why it was dropped; it is counted.
    pub fn take(
        &mut self,
        person: &Person,
        payload: &[u8],
        now_ms: u64,
        wall_ms: u64,
    ) -> Result<Line, Dropped> {
        let taken = self.decide(person, payload, now_ms, wall_ms);
        if let Err(dropped) = taken {
            *self.dropped.entry(dropped.word()).or_default() += 1;
        }
        taken
    }

    fn decide(
        &mut self,
        person: &Person,
        payload: &[u8],
        now_ms: u64,
        wall_ms: u64,
    ) -> Result<Line, Dropped> {
        let window = self.people.get_mut(person).ok_or(Dropped::NotFollowed)?;
        let said = read_said(payload).map_err(Dropped::Malformed)?;
        if said.text.len() > self.limits.text_bytes {
            return Err(Dropped::TooLong);
        }
        // Last of the checks, so that a line dropped for another reason
        // is not counted against its sender.
        if !window.take(now_ms) {
            return Err(Dropped::TooFast);
        }
        let number = self.rooms.number(&person.room).ok_or(Dropped::NoNumber)?;
        Ok(Line {
            number,
            name: person.name.clone(),
            text: said.text,
            sent_at: said.sent_at,
            at: wall_ms,
        })
    }

    /// How many lines were dropped for the reason of this word.
    #[must_use]
    pub fn dropped(&self, word: &str) -> u64 {
        self.dropped.get(word).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use xmoq_chat_tracks::names::{Kind, Room};

    use super::*;

    /// The room of a broadcast that `name` names.
    fn broadcast(name: &str) -> Room {
        Room {
            kind: Kind::Broadcast,
            name: name.to_string(),
        }
    }

    fn person(room: &str, name: &str) -> Person {
        Person {
            room: broadcast(room),
            name: name.to_string(),
        }
    }

    fn said(text: &str) -> Vec<u8> {
        format!(r#"{{"text":"{text}","sentAt":7}}"#).into_bytes()
    }

    /// Who says what, at which time of the clock that does not go back.
    struct Saying(&'static str, Vec<u8>, u64);

    /// The number a line got, or the word of why it was dropped.
    type Taken = Result<u64, &'static str>;

    /// (case, admitted before, the room of the one admitted, expected)
    type Admitting = (
        &'static str,
        Vec<(&'static str, &'static str)>,
        &'static str,
        Admitted,
    );

    #[test]
    fn lines_are_numbered_in_their_room_or_dropped_for_their_reason() {
        let limits = Limits {
            text_bytes: 4,
            lines_per_second: 2,
            fetch_lines: 10,
        };
        let long = "x".repeat(5);
        let ok = |number: u64| Ok::<u64, &str>(number);
        let cases: [(&str, Vec<Saying>, Vec<Taken>); 6] = [
            (
                "two people of one room, in the order the lines come",
                vec![
                    Saying("a", said("1"), 0),
                    Saying("b", said("2"), 0),
                    Saying("a", said("3"), 10),
                ],
                vec![ok(0), ok(1), ok(2)],
            ),
            (
                "the longest text and one byte more",
                vec![Saying("a", said("xxxx"), 0), Saying("a", said(&long), 0)],
                vec![ok(0), Err("too-long")],
            ),
            (
                "a person over the rate, and another not",
                vec![
                    Saying("a", said("1"), 0),
                    Saying("a", said("2"), 0),
                    Saying("a", said("3"), 0),
                    Saying("b", said("4"), 0),
                    Saying("a", said("5"), 1_000),
                ],
                vec![ok(0), ok(1), Err("too-fast"), ok(2), ok(3)],
            ),
            (
                "a malformed line is not counted against its sender",
                vec![
                    Saying("a", b"{}".to_vec(), 0),
                    Saying("a", br#"{"text":"x","sentAt":-1}"#.to_vec(), 0),
                    Saying("a", said("1"), 0),
                    Saying("a", said("2"), 0),
                ],
                vec![Err("text"), Err("sent-at"), ok(0), ok(1)],
            ),
            (
                "a line too long is not counted against its sender",
                vec![
                    Saying("a", said(&long), 0),
                    Saying("a", said("1"), 0),
                    Saying("a", said("2"), 0),
                ],
                vec![Err("too-long"), ok(0), ok(1)],
            ),
            (
                "somebody not followed",
                vec![Saying("c", said("1"), 0)],
                vec![Err("not-followed")],
            ),
        ];
        for (case, sayings, expected) in cases {
            let mut gatherer = Gatherer::new(limits);
            assert!(gatherer.rooms.add(&broadcast("r"), 0));
            assert_eq!(gatherer.admit(&person("r", "a")), Admitted::Now);
            assert_eq!(gatherer.admit(&person("r", "b")), Admitted::Now);
            let got: Vec<Taken> = sayings
                .iter()
                .map(|Saying(name, payload, now)| {
                    gatherer
                        .take(&person("r", name), payload, *now, 99)
                        .map(|line| line.number)
                        .map_err(Dropped::word)
                })
                .collect();
            assert_eq!(got, expected, "{case}");
            for word in ["too-long", "too-fast", "text", "sent-at", "not-followed"] {
                let counted = expected.iter().filter(|each| **each == Err(word)).count();
                assert_eq!(
                    gatherer.dropped(word),
                    u64::try_from(counted).unwrap(),
                    "{case}: {word}"
                );
            }
        }
    }

    #[test]
    fn a_line_that_is_taken_has_what_its_sender_said_and_the_gatherers_time() {
        let mut gatherer = Gatherer::new(Limits::default());
        gatherer.rooms.add(&broadcast("r"), 41);
        gatherer.admit(&person("r", "alice"));
        let line = gatherer
            .take(
                &person("r", "alice"),
                br#"{"text":"hi","sentAt":1761300000000}"#,
                0,
                1_761_300_000_009,
            )
            .unwrap();
        assert_eq!(
            line,
            Line {
                number: 41,
                name: "alice".to_string(),
                text: "hi".to_string(),
                sent_at: 1_761_300_000_000,
                at: 1_761_300_000_009,
            }
        );
    }

    #[test]
    fn who_is_followed() {
        let cases: [Admitting; 3] = [
            ("a room that is numbered", vec![], "r", Admitted::Now),
            ("again", vec![("r", "a")], "r", Admitted::Already),
            ("a room that is not", vec![], "s", Admitted::NoRoom),
        ];
        for (case, before, room, expected) in cases {
            let mut gatherer = Gatherer::new(Limits::default());
            gatherer.rooms.add(&broadcast("r"), 0);
            for (room, name) in before {
                gatherer.admit(&person(room, name));
            }
            assert_eq!(gatherer.admit(&person(room, "a")), expected, "{case}");
        }
    }

    #[test]
    fn a_room_is_numbered_while_somebody_is_followed_in_it() {
        // (case, admitted in room r, who leaves, what becomes of the room,
        // whether r is numbered after)
        type Names = &'static [&'static str];
        let cases: [(&str, Names, Names, &[Left], bool); 4] = [
            (
                "the last one leaves",
                &["a"],
                &["a"],
                &[Left::RoomEmpty],
                false,
            ),
            (
                "one of two leaves",
                &["a", "b"],
                &["a"],
                &[Left::RoomStays],
                true,
            ),
            (
                "both leave",
                &["a", "b"],
                &["b", "a"],
                &[Left::RoomStays, Left::RoomEmpty],
                false,
            ),
            (
                "somebody not followed leaves",
                &["a"],
                &["c"],
                &[Left::NotFollowed],
                true,
            ),
        ];
        for (case, admitted, leaving, expected, numbered) in cases {
            let mut gatherer = Gatherer::new(Limits::default());
            gatherer.rooms.add(&broadcast("r"), 0);
            gatherer.rooms.add(&broadcast("s"), 0);
            gatherer.admit(&person("s", "a"));
            for name in admitted {
                gatherer.admit(&person("r", name));
            }
            let left: Vec<Left> = leaving
                .iter()
                .map(|name| gatherer.leave(&person("r", name)))
                .collect();
            assert_eq!(left, expected, "{case}");
            assert_eq!(gatherer.rooms.knows(&broadcast("r")), numbered, "{case}");
            assert!(
                gatherer.rooms.knows(&broadcast("s")),
                "{case}: another room stays"
            );
        }
    }

    #[test]
    fn no_more_people_than_the_limit_are_followed_and_one_who_left_is_not() {
        let mut gatherer = Gatherer::new(Limits::default());
        gatherer.rooms.add(&broadcast("r"), 0);
        for index in 0..MAX_PEOPLE {
            assert_eq!(
                gatherer.admit(&person("r", &index.to_string())),
                Admitted::Now
            );
        }
        assert_eq!(
            gatherer.admit(&person("r", "one more")),
            Admitted::TooManyPeople
        );
        assert_eq!(gatherer.leave(&person("r", "0")), Left::RoomStays);
        assert!(!gatherer.follows(&person("r", "0")));
        assert_eq!(gatherer.admit(&person("r", "one more")), Admitted::Now);
        gatherer.leave_everybody();
        assert!(!gatherer.follows(&person("r", "1")));
        assert_eq!(gatherer.admit(&person("r", "1")), Admitted::NoRoom);
    }
}
