//! The garden's rules: what an action of an actor does to a room's garden.
//! The garden is the one that says who may do what; `read_action` of
//! `xmoq-pet-tracks` reads any kind from any actor.

use std::collections::VecDeque;

use xmoq_pet_tracks::names::PET_NAME;
use xmoq_pet_tracks::payloads::{
    ActionKind, Event, EventKind, Garden, MOST_EVENTS, MOST_THINGS, Place, Thing, ThingKind,
};

/// How long an event stays in the garden: one older than this when a garden
/// is written is left out. WHY a time and not only [`MOST_EVENTS`]: an event
/// is what happened of late, and one who comes a minute later is not to see
/// a call as if it were made now. **Provisional.**
pub const EVENT_KEPT_MS: u64 = 10_000;

/// Why an action does not change the garden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropped {
    /// `eat` or `take` from a person: the pet's alone.
    NotThePet,
    /// What a person does, from the pet.
    NotAPerson,
    /// `eat` of no bowl, or `take` of no ball, that lies on the floor.
    NoSuchThing,
}

impl Dropped {
    /// One word for the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::NotThePet => "not-the-pet",
            Self::NotAPerson => "not-a-person",
            Self::NoSuchThing => "no-such-thing",
        }
    }
}

/// The garden of one room. Ids of things and of events are counted apart,
/// from 1, and none is given twice while the room is kept.
#[derive(Debug, Default)]
pub struct RoomGarden {
    /// The oldest first.
    things: VecDeque<Thing>,
    /// The oldest first.
    events: VecDeque<Event>,
    last_thing: u64,
    last_event: u64,
}

impl RoomGarden {
    /// What `by` does, taken at `now`.
    ///
    /// # Errors
    ///
    /// Why the garden is not changed by it.
    pub fn act(&mut self, by: &str, kind: &ActionKind, now: u64) -> Result<(), Dropped> {
        let pet = by == PET_NAME;
        match kind {
            ActionKind::Feed { .. }
            | ActionKind::Throw { .. }
            | ActionKind::Call
            | ActionKind::Pet
            | ActionKind::Talk { .. }
                if pet =>
            {
                Err(Dropped::NotAPerson)
            }
            ActionKind::Eat { .. } | ActionKind::Take { .. } if !pet => Err(Dropped::NotThePet),
            ActionKind::Feed { to } => {
                self.put(ThingKind::Bowl, *to, by);
                Ok(())
            }
            ActionKind::Throw { to } => {
                // WHY the ball before is taken away and the new one gets
                // a new id: one ball a room, and a reader that knows the
                // ball by its id sees a new throw.
                self.things.retain(|thing| thing.kind != ThingKind::Ball);
                self.put(ThingKind::Ball, *to, by);
                Ok(())
            }
            ActionKind::Call => {
                self.happen(EventKind::Call, by, None, now);
                Ok(())
            }
            ActionKind::Pet => {
                self.happen(EventKind::Pet, by, None, now);
                Ok(())
            }
            ActionKind::Talk { text } => {
                self.happen(EventKind::Talk, by, Some(text.clone()), now);
                Ok(())
            }
            ActionKind::Eat { thing } => self.take_away(*thing, ThingKind::Bowl),
            ActionKind::Take { thing } => self.take_away(*thing, ThingKind::Ball),
        }
    }

    /// The garden as it is at `now`; the events older than
    /// [`EVENT_KEPT_MS`] are forgotten.
    pub fn write(&mut self, now: u64) -> Garden {
        self.events
            .retain(|event| now.saturating_sub(event.at) <= EVENT_KEPT_MS);
        Garden {
            things: self.things.iter().cloned().collect(),
            events: self.events.iter().cloned().collect(),
            at: now,
        }
    }

    /// A new thing; over [`MOST_THINGS`], the oldest is taken away.
    fn put(&mut self, kind: ThingKind, at: Place, by: &str) {
        if self.things.len() >= MOST_THINGS {
            self.things.pop_front();
        }
        self.last_thing += 1;
        self.things.push_back(Thing {
            id: self.last_thing,
            kind,
            at,
            by: by.to_string(),
        });
    }

    /// A new event; over [`MOST_EVENTS`], the oldest is forgotten.
    fn happen(&mut self, kind: EventKind, by: &str, text: Option<String>, now: u64) {
        if self.events.len() >= MOST_EVENTS {
            self.events.pop_front();
        }
        self.last_event += 1;
        self.events.push_back(Event {
            id: self.last_event,
            kind,
            by: by.to_string(),
            text,
            at: now,
        });
    }

    fn take_away(&mut self, id: u64, kind: ThingKind) -> Result<(), Dropped> {
        let index = self
            .things
            .iter()
            .position(|thing| thing.id == id && thing.kind == kind)
            .ok_or(Dropped::NoSuchThing)?;
        self.things.remove(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use xmoq_pet_tracks::payloads::garden_payload;

    use super::*;

    fn at(x: u16, y: u16) -> Place {
        Place { x, y }
    }

    /// (who, what, when, what becomes of it)
    type Step = (&'static str, ActionKind, u64, Result<(), Dropped>);

    fn feed(x: u16) -> ActionKind {
        ActionKind::Feed { to: at(x, 1) }
    }

    fn throw(x: u16) -> ActionKind {
        ActionKind::Throw { to: at(x, 1) }
    }

    /// Each case: its steps, and the payload of the garden written at the
    /// last step's time.
    #[test]
    #[expect(clippy::too_many_lines, reason = "a table of 10 rows and one loop")]
    fn what_an_action_does_to_the_garden() {
        let ok = Ok(());
        let cases: Vec<(&str, Vec<Step>, &str)> = vec![
            ("nothing", vec![], r#"{"things":[],"events":[],"at":0}"#),
            (
                "a bowl and a ball",
                vec![("alice", feed(300), 5, ok), ("bob", throw(400), 6, ok)],
                r#"{"things":[{"id":1,"kind":"bowl","at":[300,1],"by":"alice"},{"id":2,"kind":"ball","at":[400,1],"by":"bob"}],"events":[],"at":6}"#,
            ),
            (
                "a throw takes the ball before away, and the new one has a new id",
                vec![
                    ("alice", throw(1), 5, ok),
                    ("alice", feed(2), 5, ok),
                    ("bob", throw(3), 6, ok),
                ],
                r#"{"things":[{"id":2,"kind":"bowl","at":[2,1],"by":"alice"},{"id":3,"kind":"ball","at":[3,1],"by":"bob"}],"events":[],"at":6}"#,
            ),
            (
                "events, counted apart from things",
                vec![
                    ("alice", feed(1), 5, ok),
                    ("alice", ActionKind::Call, 5, ok),
                    ("bob", ActionKind::Pet, 6, ok),
                    (
                        "bob",
                        ActionKind::Talk {
                            text: "hi".to_string(),
                        },
                        7,
                        ok,
                    ),
                ],
                r#"{"things":[{"id":1,"kind":"bowl","at":[1,1],"by":"alice"}],"events":[{"id":1,"kind":"call","by":"alice","text":null,"at":5},{"id":2,"kind":"pet","by":"bob","text":null,"at":6},{"id":3,"kind":"talk","by":"bob","text":"hi","at":7}],"at":7}"#,
            ),
            (
                "the pet eats a bowl and takes the ball",
                vec![
                    ("alice", feed(1), 5, ok),
                    ("alice", feed(2), 5, ok),
                    ("bob", throw(3), 5, ok),
                    ("pet", ActionKind::Eat { thing: 1 }, 6, ok),
                    ("pet", ActionKind::Take { thing: 3 }, 7, ok),
                ],
                r#"{"things":[{"id":2,"kind":"bowl","at":[2,1],"by":"alice"}],"events":[],"at":7}"#,
            ),
            (
                "what is not there, or is of the other kind",
                vec![
                    ("alice", feed(1), 5, ok),
                    ("bob", throw(2), 5, ok),
                    (
                        "pet",
                        ActionKind::Eat { thing: 9 },
                        6,
                        Err(Dropped::NoSuchThing),
                    ),
                    (
                        "pet",
                        ActionKind::Eat { thing: 2 },
                        6,
                        Err(Dropped::NoSuchThing),
                    ),
                    (
                        "pet",
                        ActionKind::Take { thing: 1 },
                        6,
                        Err(Dropped::NoSuchThing),
                    ),
                    ("pet", ActionKind::Eat { thing: 1 }, 6, ok),
                    (
                        "pet",
                        ActionKind::Eat { thing: 1 },
                        7,
                        Err(Dropped::NoSuchThing),
                    ),
                ],
                r#"{"things":[{"id":2,"kind":"ball","at":[2,1],"by":"bob"}],"events":[],"at":7}"#,
            ),
            (
                "a person does not eat or take",
                vec![
                    ("alice", feed(1), 5, ok),
                    (
                        "alice",
                        ActionKind::Eat { thing: 1 },
                        5,
                        Err(Dropped::NotThePet),
                    ),
                    (
                        "alice",
                        ActionKind::Take { thing: 1 },
                        5,
                        Err(Dropped::NotThePet),
                    ),
                ],
                r#"{"things":[{"id":1,"kind":"bowl","at":[1,1],"by":"alice"}],"events":[],"at":5}"#,
            ),
            (
                "the pet does nothing of a person",
                vec![
                    ("pet", feed(1), 5, Err(Dropped::NotAPerson)),
                    ("pet", throw(1), 5, Err(Dropped::NotAPerson)),
                    ("pet", ActionKind::Call, 5, Err(Dropped::NotAPerson)),
                    ("pet", ActionKind::Pet, 5, Err(Dropped::NotAPerson)),
                    (
                        "pet",
                        ActionKind::Talk {
                            text: "hi".to_string(),
                        },
                        5,
                        Err(Dropped::NotAPerson),
                    ),
                ],
                r#"{"things":[],"events":[],"at":5}"#,
            ),
            (
                "a thing taken away keeps its id used",
                vec![
                    ("alice", feed(1), 5, ok),
                    ("pet", ActionKind::Eat { thing: 1 }, 5, ok),
                    ("alice", feed(2), 5, ok),
                ],
                r#"{"things":[{"id":2,"kind":"bowl","at":[2,1],"by":"alice"}],"events":[],"at":5}"#,
            ),
            (
                "an event is left out once it is older than the time it is kept",
                vec![
                    ("alice", ActionKind::Call, 1_000, ok),
                    ("bob", ActionKind::Pet, 1_001, ok),
                    ("alice", feed(1), 11_001, ok),
                ],
                r#"{"things":[{"id":1,"kind":"bowl","at":[1,1],"by":"alice"}],"events":[{"id":2,"kind":"pet","by":"bob","text":null,"at":1001}],"at":11001}"#,
            ),
        ];
        for (case, steps, expected) in cases {
            let mut garden = RoomGarden::default();
            let mut now = 0;
            for (index, (by, kind, at, done)) in steps.into_iter().enumerate() {
                now = at;
                assert_eq!(garden.act(by, &kind, at), done, "{case}, {index}");
            }
            assert_eq!(
                String::from_utf8(garden_payload(&garden.write(now))).unwrap(),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn no_more_things_and_events_than_the_most_the_oldest_going_first() {
        // Three over each most.
        assert_eq!((MOST_THINGS, MOST_EVENTS), (8, 16));
        let mut garden = RoomGarden::default();
        for x in 0..11 {
            garden.act("alice", &feed(x), 5).unwrap();
        }
        for _ in 0..19 {
            garden.act("alice", &ActionKind::Call, 5).unwrap();
        }
        let written = garden.write(5);
        assert_eq!(
            written
                .things
                .iter()
                .map(|thing| thing.id)
                .collect::<Vec<_>>(),
            [4, 5, 6, 7, 8, 9, 10, 11]
        );
        assert_eq!(
            written
                .events
                .iter()
                .map(|event| event.id)
                .collect::<Vec<_>>(),
            [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]
        );
    }
}
