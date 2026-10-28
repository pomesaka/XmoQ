//! What the pet perceives of its room's garden: each garden read is
//! compared with the one read before it, and what is new of it (the events
//! after the last one seen, the things put on the floor since) and the
//! things gone from the floor are what the pet perceives.
//!
//! WHY compared and not taken as it is: the garden is the whole of the room
//! again in every Group, its events the 16 newest; a pet that reacted to
//! every garden would react to the same call again at every change.

use xmoq_pet_tracks::payloads::{Event, Garden, Thing};

/// Something that happened in the garden, for the pet to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Happening {
    /// A thing put on the floor: a bowl put down, the ball thrown.
    Put(Thing),
    /// A call, a pet or a talk.
    Done(Event),
}

impl Happening {
    /// The name of the actor that did it.
    #[must_use]
    pub fn by(&self) -> &str {
        match self {
            Self::Put(thing) => &thing.by,
            Self::Done(event) => &event.by,
        }
    }
}

/// What one garden read showed of new.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Perceived {
    /// What happened since the garden read before: the new events, the
    /// newest last, and then the new things, in the order of their ids.
    /// WHY the things after the events: the garden orders the two apart (an
    /// event has the time it was taken, a thing none), and the table
    /// decides by the last ([`crate::decide::fixed`]), where a thing on the
    /// floor, which the pet can go to, is the more for it to do.
    /// **Provisional.**
    pub new: Vec<Happening>,
    /// The things that were on the floor and are not.
    pub gone: Vec<Thing>,
}

/// The garden of a room as the pet saw it last.
#[derive(Debug, Clone, Default)]
pub struct Perceiver {
    /// The Group of the garden read last on this subscription.
    group: Option<u64>,
    /// The largest id of an event seen, and of a thing.
    events_seen: Option<u64>,
    things_seen: Option<u64>,
    floor: Vec<Thing>,
    /// Whether a garden was read at all.
    read_one: bool,
}

impl Perceiver {
    /// The things on the floor, as the garden read last has them.
    #[must_use]
    pub fn floor(&self) -> &[Thing] {
        &self.floor
    }

    /// A new subscription to the garden begins: its Groups are compared
    /// among themselves, and not with the Groups of the one before. WHY: a
    /// garden that began anew may number its Groups anew; what was seen of
    /// it is still known by the ids.
    pub fn subscribed(&mut self) {
        self.group = None;
    }

    /// The garden of Group `group`: what is new of it. `None` for a Group
    /// that is not after the one read last on this subscription, such as
    /// the one a Joining FETCH gives after a live Object of a later Group.
    ///
    /// The first garden read is taken as it is, with nothing new: what
    /// happened before the pet came is not reacted to. So is a garden whose
    /// ids are below those seen, as one that began anew (ids go up in a
    /// room and are never used twice, `fixtures/pet-tracks/README.md`).
    pub fn read(&mut self, group: u64, garden: Garden) -> Option<Perceived> {
        if self.group.is_some_and(|last| group <= last) {
            return None;
        }
        self.group = Some(group);
        let events = garden.events.iter().map(|event| event.id).max();
        let things = garden.things.iter().map(|thing| thing.id).max();
        let below = |now: Option<u64>, seen: Option<u64>| {
            now.zip(seen).is_some_and(|(now, seen)| now < seen)
        };
        let anew =
            !self.read_one || below(events, self.events_seen) || below(things, self.things_seen);
        let mut perceived = Perceived::default();
        if anew {
            self.events_seen = events;
            self.things_seen = things;
        } else {
            let after = |seen: Option<u64>, id: u64| seen.is_none_or(|seen| id > seen);
            perceived.new.extend(
                garden
                    .events
                    .iter()
                    .filter(|event| after(self.events_seen, event.id))
                    .cloned()
                    .map(Happening::Done),
            );
            let mut put: Vec<&Thing> = garden
                .things
                .iter()
                .filter(|thing| after(self.things_seen, thing.id))
                .collect();
            put.sort_by_key(|thing| thing.id);
            perceived
                .new
                .extend(put.into_iter().cloned().map(Happening::Put));
            perceived.gone = self
                .floor
                .iter()
                .filter(|was| garden.things.iter().all(|thing| thing.id != was.id))
                .cloned()
                .collect();
            self.events_seen = self.events_seen.max(events);
            self.things_seen = self.things_seen.max(things);
        }
        self.read_one = true;
        self.floor = garden.things;
        Some(perceived)
    }
}

#[cfg(test)]
mod tests {
    use xmoq_pet_tracks::payloads::{EventKind, Place, ThingKind};

    use super::*;

    fn thing(id: u64, kind: ThingKind) -> Thing {
        Thing {
            id,
            kind,
            at: Place { x: 10, y: 20 },
            by: "ann".to_string(),
        }
    }

    fn event(id: u64, kind: EventKind) -> Event {
        Event {
            id,
            kind,
            by: "bob".to_string(),
            text: (kind == EventKind::Talk).then(|| "hi".to_string()),
            at: 7,
        }
    }

    fn garden(things: &[(u64, ThingKind)], events: &[(u64, EventKind)]) -> Garden {
        Garden {
            things: things.iter().map(|(id, kind)| thing(*id, *kind)).collect(),
            events: events.iter().map(|(id, kind)| event(*id, *kind)).collect(),
            at: 1,
        }
    }

    /// What a read gives: the ids of the new events and things, as `e<id>`
    /// and `t<id>`, and of the things gone; `None` for a Group passed over.
    type Seen = Option<(Vec<String>, Vec<u64>)>;

    fn seen(perceived: Option<Perceived>) -> Seen {
        perceived.map(|perceived| {
            (
                perceived
                    .new
                    .iter()
                    .map(|happening| match happening {
                        Happening::Done(event) => format!("e{}", event.id),
                        Happening::Put(thing) => format!("t{}", thing.id),
                    })
                    .collect(),
                perceived.gone.iter().map(|thing| thing.id).collect(),
            )
        })
    }

    fn of(new: &[&str], gone: &[u64]) -> (Vec<String>, Vec<u64>) {
        (new.iter().map(ToString::to_string).collect(), gone.to_vec())
    }

    enum Step {
        Read(u64, Garden, Seen),
        Subscribed,
    }
    use Step::{Read, Subscribed};

    #[test]
    fn what_is_new_of_a_garden_is_what_was_not_seen() {
        use EventKind::{Call, Pet, Talk};
        use ThingKind::{Ball, Bowl};
        let cases: [(&str, Vec<Step>); 6] = [
            (
                "the first garden is taken as it is",
                vec![
                    Read(5, garden(&[(1, Bowl)], &[(3, Call)]), Some(of(&[], &[]))),
                    Read(
                        6,
                        garden(&[(1, Bowl), (2, Ball)], &[(3, Call), (4, Pet)]),
                        Some(of(&["e4", "t2"], &[])),
                    ),
                ],
            ),
            (
                "new events, new things and things gone",
                vec![
                    Read(0, garden(&[(1, Bowl), (2, Ball)], &[]), Some(of(&[], &[]))),
                    Read(
                        1,
                        garden(&[(2, Ball), (4, Bowl), (3, Bowl)], &[(1, Talk), (2, Call)]),
                        Some(of(&["e1", "e2", "t3", "t4"], &[1])),
                    ),
                    Read(2, garden(&[], &[(2, Call)]), Some(of(&[], &[2, 4, 3]))),
                ],
            ),
            (
                "a Group not after the last is passed over",
                vec![
                    Read(9, garden(&[], &[(1, Call)]), Some(of(&[], &[]))),
                    Read(9, garden(&[], &[(1, Call), (2, Pet)]), None),
                    Read(8, garden(&[], &[(1, Call), (2, Pet)]), None),
                    Read(
                        10,
                        garden(&[], &[(1, Call), (2, Pet)]),
                        Some(of(&["e2"], &[])),
                    ),
                ],
            ),
            (
                "a new subscription numbers anew and knows the ids",
                vec![
                    Read(9, garden(&[], &[(1, Call)]), Some(of(&[], &[]))),
                    Subscribed,
                    Read(
                        0,
                        garden(&[], &[(1, Call), (2, Pet)]),
                        Some(of(&["e2"], &[])),
                    ),
                ],
            ),
            (
                "events that fell out of the garden are no event again",
                vec![
                    Read(0, garden(&[], &[(5, Call), (6, Pet)]), Some(of(&[], &[]))),
                    Read(
                        1,
                        garden(&[], &[(6, Pet), (7, Talk)]),
                        Some(of(&["e7"], &[])),
                    ),
                    Read(2, garden(&[], &[]), Some(of(&[], &[]))),
                    Read(3, garden(&[], &[(8, Call)]), Some(of(&["e8"], &[]))),
                ],
            ),
            (
                "a garden whose ids are below those seen began anew",
                vec![
                    Read(0, garden(&[(9, Bowl)], &[(9, Call)]), Some(of(&[], &[]))),
                    Subscribed,
                    Read(0, garden(&[(1, Ball)], &[(1, Pet)]), Some(of(&[], &[]))),
                    Read(
                        1,
                        garden(&[(1, Ball)], &[(1, Pet), (2, Call)]),
                        Some(of(&["e2"], &[])),
                    ),
                ],
            ),
        ];
        for (case, steps) in cases {
            let mut perceiver = Perceiver::default();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Read(group, garden, expected) => {
                        let things = garden.things.clone();
                        let got = perceiver.read(group, garden);
                        let passed_over = got.is_none();
                        assert_eq!(seen(got), expected, "{case}, {index}");
                        if !passed_over {
                            assert_eq!(perceiver.floor(), things, "{case}, {index}");
                        }
                    }
                    Subscribed => perceiver.subscribed(),
                }
            }
        }
    }

    #[test]
    fn a_happening_is_by_its_actor() {
        assert_eq!(Happening::Put(thing(1, ThingKind::Bowl)).by(), "ann");
        assert_eq!(Happening::Done(event(1, EventKind::Call)).by(), "bob");
    }
}
