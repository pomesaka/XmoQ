//! The rooms that have a pet: who is in each, its pet, what the pet saw of
//! its garden last, its batch, and what it published. A room is kept in
//! memory alone, from its first person until [`FORGET_AFTER_MS`] after its
//! last one left; nothing is written to a disk, and a pet that is forgotten
//! begins anew.

use std::collections::{BTreeSet, HashMap};

use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_pet_tracks::names::Actor;
use xmoq_pet_tracks::payloads::{Action, ActionKind, action_payload, state_payload};
use xmoq_publisher::fetch::{Plan, PublishedGroups, plan_kept};

use crate::batch::Batch;
use crate::perceive::Perceiver;
use crate::pet::Pet;

/// The rooms that have a pet at once, those whose last person left and that
/// are not forgotten yet among them. A person of one room more is passed
/// over. WHY a limit: each room holds requests at the relay and a pet here,
/// and the rooms are named by whoever announces under the prefix. WHY 48:
/// the own relay keeps 256 requests of a session (`MAX_PEER_REQUESTS` of
/// `xmoq-moqt`), and a room takes five at the most (its `PUBLISH_NAMESPACE`,
/// the PUBLISH of `state` and of `acted`, the SUBSCRIBE to `garden` and its
/// Joining FETCH), besides the `SUBSCRIBE_NAMESPACE`. **Provisional.**
pub const MAX_ROOMS: usize = 48;

/// The persons in every room the pet keeps count of at once. One more is
/// passed over. **Provisional**, as [`MAX_ROOMS`]: they cost the pet a name
/// each, and the relay nothing.
pub const MAX_PEOPLE: usize = 1_024;

/// How long a room's pet is kept, announced and published, after its last
/// person left. WHY kept at all: one who reloads the page finds the same
/// pet. **Provisional.**
pub const FORGET_AFTER_MS: u64 = 60_000;

/// How often the state of a room is published again without a change, so
/// that one who comes gets a state soon from a live Group. **Provisional.**
pub const STATE_AGAIN_MS: u64 = 2_000;

/// A state a FETCH gets: its Group ID and its payload.
pub type Fetched = (u64, Vec<u8>);

/// The Group ID of an Object written at `now` after the Group `last`: `now`,
/// or one more than `last` where that is not more. WHY the clock: a PUBLISH
/// of a later session numbers on above what an earlier one sent, as the
/// chat's counts do, and a relay would take Groups below those it has for
/// old.
fn next_group(last: Option<u64>, now: u64) -> u64 {
    last.map_or(0, |last| last.saturating_add(1)).max(now)
}

/// One room's pet and what it knows of the room.
#[derive(Debug)]
pub struct Room {
    /// The pet.
    pub pet: Pet,
    /// What the pet perceived for its next decision.
    pub batch: Batch,
    /// The garden as the pet saw it last.
    pub perceiver: Perceiver,
    /// The names of the persons in the room: while there is one, its state
    /// is published.
    people: BTreeSet<String>,
    left_at: Option<u64>,
    first_group: Option<u64>,
    latest: Option<(u64, Vec<u8>)>,
    written_at: u64,
    /// The Group ID of the last action sent on `acted`.
    acted_group: Option<u64>,
}

impl Room {
    fn new(pet: Pet) -> Self {
        Self {
            pet,
            batch: Batch::default(),
            perceiver: Perceiver::default(),
            people: BTreeSet::new(),
            left_at: None,
            first_group: None,
            latest: None,
            written_at: 0,
            acted_group: None,
        }
    }

    /// Whether somebody is in the room.
    #[must_use]
    pub fn has_people(&self) -> bool {
        !self.people.is_empty()
    }

    /// The state of the pet written at `now`, as Group ID and payload, kept
    /// as the latest; numbered as [`next_group`] says.
    pub fn write(&mut self, now: u64) -> (u64, Vec<u8>) {
        let mut state = self.pet.state().clone();
        state.at = now;
        let group = next_group(self.latest.as_ref().map(|(last, _)| *last), now);
        let payload = state_payload(&state);
        self.first_group.get_or_insert(group);
        self.latest = Some((group, payload.clone()));
        self.written_at = now;
        (group, payload)
    }

    /// What the pet does to the garden at `now`, as Group ID and payload of
    /// `acted`, numbered as [`next_group`] says.
    pub fn write_acted(&mut self, kind: ActionKind, now: u64) -> (u64, Vec<u8>) {
        let group = next_group(self.acted_group, now);
        self.acted_group = Some(group);
        (group, action_payload(&Action { kind, sent_at: now }))
    }

    /// The Group ID of the latest state; `None` before the first.
    #[must_use]
    pub fn latest_group(&self) -> Option<u64> {
        self.latest.as_ref().map(|(group, _)| *group)
    }

    /// The Group ID of the last action; `None` before the first.
    #[must_use]
    pub fn acted_group(&self) -> Option<u64> {
        self.acted_group
    }

    /// When the state is to be published again without a change; `None`
    /// while nobody is in the room.
    #[must_use]
    pub fn again_at(&self) -> Option<u64> {
        (self.has_people() && self.latest.is_some())
            .then(|| self.written_at.saturating_add(STATE_AGAIN_MS))
    }

    /// When the room is forgotten; `None` while somebody is in it.
    #[must_use]
    pub fn forget_at(&self) -> Option<u64> {
        self.left_at
            .map(|left| left.saturating_add(FORGET_AFTER_MS))
    }

    /// When something of the room is next to be done.
    #[must_use]
    pub fn next_at(&self) -> Option<u64> {
        [
            Some(self.pet.next_at()),
            self.batch.due_at(),
            self.again_at(),
            self.forget_at(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// What a Standalone FETCH from `start` to the End Location `end` gets of
    /// `state` (`plan_kept` of `xmoq-publisher`): the latest state, where it
    /// lies in the range. The Groups before it that the pet published get
    /// the End of Unknown Range indicator: it keeps no more than the latest.
    ///
    /// # Errors
    ///
    /// `INVALID_RANGE` before the first state, and for a range that starts
    /// after the latest.
    pub fn fetch(
        &self,
        start: Location,
        end: Location,
        order: GroupOrder,
    ) -> Result<(Plan, Vec<Fetched>), u64> {
        let kept = self
            .first_group
            .zip(self.latest.as_ref())
            .map(|(first, (latest, _))| PublishedGroups {
                first,
                first_kept: *latest,
                largest: *latest,
            });
        let planned = plan_kept(start, end, kept, u64::MAX, order)?;
        let states = match (planned.groups, &self.latest) {
            (Some(_), Some(latest)) => vec![latest.clone()],
            _ => Vec::new(),
        };
        Ok((planned, states))
    }
}

/// Whether a person is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joined {
    /// From now on. `new_room`: its room had no pet before.
    Now {
        /// Its room's pet was made for it.
        new_room: bool,
    },
    /// It was before.
    Already,
    /// [`MAX_ROOMS`] have a pet.
    TooManyRooms,
    /// [`MAX_PEOPLE`] are counted.
    TooManyPeople,
}

impl Joined {
    /// One word for the log.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Now { .. } => "now",
            Self::Already => "already",
            Self::TooManyRooms => "too-many-rooms",
            Self::TooManyPeople => "too-many-people",
        }
    }
}

/// What became of the room of a person who left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    /// The person was not counted.
    NotThere,
    /// Somebody else is in the room.
    RoomStays,
    /// Nobody is: the room's state is published again no more, and it is
    /// forgotten after [`FORGET_AFTER_MS`].
    RoomEmpty,
}

/// Every room that has a pet.
#[derive(Debug, Default)]
pub struct Rooms {
    rooms: HashMap<String, Room>,
    people: usize,
}

impl Rooms {
    /// Counts `person` in from `now`; a room that has no pet gets one with
    /// the whims of `seed`.
    pub fn join(&mut self, person: &Actor, now: u64, seed: u64) -> Joined {
        if self.counts(person) {
            return Joined::Already;
        }
        if self.people >= MAX_PEOPLE {
            return Joined::TooManyPeople;
        }
        let new_room = !self.rooms.contains_key(&person.room);
        if new_room && self.rooms.len() >= MAX_ROOMS {
            return Joined::TooManyRooms;
        }
        let room = self
            .rooms
            .entry(person.room.clone())
            .or_insert_with(|| Room::new(Pet::new(seed, now)));
        room.people.insert(person.name.clone());
        room.left_at = None;
        self.people += 1;
        Joined::Now { new_room }
    }

    /// Counts the person out at `now`.
    pub fn leave(&mut self, person: &Actor, now: u64) -> Left {
        let Some(room) = self.rooms.get_mut(&person.room) else {
            return Left::NotThere;
        };
        if !room.people.remove(&person.name) {
            return Left::NotThere;
        }
        self.people -= 1;
        if room.has_people() {
            return Left::RoomStays;
        }
        room.left_at = Some(now);
        Left::RoomEmpty
    }

    /// Counts nobody, as before the first NAMESPACE of a session: each room
    /// is forgotten [`FORGET_AFTER_MS`] from `now` unless somebody comes
    /// again. WHY the pets are kept: a session that connects again is told
    /// again who is in the rooms, and finds the same pets.
    pub fn leave_everybody(&mut self, now: u64) {
        for room in self.rooms.values_mut() {
            if room.has_people() {
                room.people.clear();
                room.left_at = Some(now);
            }
        }
        self.people = 0;
    }

    /// Whether the person is counted.
    #[must_use]
    pub fn counts(&self, person: &Actor) -> bool {
        self.rooms
            .get(&person.room)
            .is_some_and(|room| room.people.contains(&person.name))
    }

    /// The room of this name.
    #[must_use]
    pub fn get(&self, room: &str) -> Option<&Room> {
        self.rooms.get(room)
    }

    /// The same, to change.
    pub fn get_mut(&mut self, room: &str) -> Option<&mut Room> {
        self.rooms.get_mut(room)
    }

    /// The names of the rooms.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.rooms.keys().cloned().collect()
    }

    /// Forgets the rooms nobody came back to up to `now`: their names.
    pub fn forget(&mut self, now: u64) -> Vec<String> {
        let gone: Vec<String> = self
            .rooms
            .iter()
            .filter(|(_, room)| room.forget_at().is_some_and(|at| at <= now))
            .map(|(name, _)| name.clone())
            .collect();
        for name in &gone {
            self.rooms.remove(name);
        }
        gone
    }

    /// When something of any room is next to be done.
    #[must_use]
    pub fn next_at(&self) -> Option<u64> {
        self.rooms.values().filter_map(Room::next_at).min()
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::message::request_error_code;
    use xmoq_pet_tracks::payloads::{read_action, read_state};

    use super::*;

    fn who(room: &str, name: &str) -> Actor {
        Actor {
            room: room.to_string(),
            name: name.to_string(),
        }
    }

    enum Step {
        Join(&'static str, &'static str, u64, Joined),
        Leave(&'static str, &'static str, u64, Left),
        Everybody(u64),
        Forget(u64, &'static [&'static str]),
    }
    use Step::{Everybody, Forget, Join, Leave};

    #[test]
    fn a_room_has_a_pet_while_somebody_is_in_it_and_a_while_after() {
        let new = Joined::Now { new_room: true };
        let old = Joined::Now { new_room: false };
        let cases: [(&str, Vec<Step>); 5] = [
            (
                "two of one room",
                vec![
                    Join("r", "a", 0, new),
                    Join("r", "b", 0, old),
                    Join("r", "a", 0, Joined::Already),
                    Leave("r", "a", 5, Left::RoomStays),
                    Leave("r", "b", 10, Left::RoomEmpty),
                    Leave("r", "b", 10, Left::NotThere),
                    Forget(10 + FORGET_AFTER_MS - 1, &[]),
                    Forget(10 + FORGET_AFTER_MS, &["r"]),
                    Join("r", "a", 99_999, new),
                ],
            ),
            (
                "one who comes back in time keeps the pet",
                vec![
                    Join("r", "a", 0, new),
                    Leave("r", "a", 0, Left::RoomEmpty),
                    Join("r", "a", FORGET_AFTER_MS - 1, old),
                    Forget(FORGET_AFTER_MS * 3, &[]),
                ],
            ),
            (
                "a new session",
                vec![
                    Join("r", "a", 0, new),
                    Everybody(100),
                    Leave("r", "a", 100, Left::NotThere),
                    Join("r", "a", 200, old),
                    Forget(100 + FORGET_AFTER_MS, &[]),
                    Everybody(300),
                    Forget(300 + FORGET_AFTER_MS, &["r"]),
                ],
            ),
            (
                "somebody never counted",
                vec![Leave("r", "a", 0, Left::NotThere)],
            ),
            (
                "rooms apart",
                vec![
                    Join("r", "a", 0, new),
                    Join("s", "a", 0, new),
                    Leave("s", "a", 0, Left::RoomEmpty),
                    Forget(FORGET_AFTER_MS, &["s"]),
                ],
            ),
        ];
        for (case, steps) in cases {
            let mut rooms = Rooms::default();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Join(room, name, now, joined) => {
                        assert_eq!(
                            rooms.join(&who(room, name), now, 1),
                            joined,
                            "{case}, {index}"
                        );
                    }
                    Leave(room, name, now, left) => {
                        assert_eq!(rooms.leave(&who(room, name), now), left, "{case}, {index}");
                    }
                    Everybody(now) => rooms.leave_everybody(now),
                    Forget(now, gone) => assert_eq!(rooms.forget(now), gone, "{case}, {index}"),
                }
            }
        }
    }

    #[test]
    fn no_more_rooms_and_people_than_the_limits() {
        let mut rooms = Rooms::default();
        for index in 0..MAX_ROOMS {
            assert_eq!(
                rooms.join(&who(&index.to_string(), "a"), 0, 1),
                Joined::Now { new_room: true }
            );
        }
        assert_eq!(
            rooms.join(&who("one more", "a"), 0, 1),
            Joined::TooManyRooms
        );
        for index in MAX_ROOMS..MAX_PEOPLE {
            assert_eq!(
                rooms.join(&who("0", &index.to_string()), 0, 1),
                Joined::Now { new_room: false }
            );
        }
        assert_eq!(
            rooms.join(&who("1", "one more"), 0, 1),
            Joined::TooManyPeople
        );
        assert_eq!(rooms.leave(&who("1", "a"), 0), Left::RoomEmpty);
        assert_eq!(
            rooms.join(&who("1", "one more"), 0, 1),
            Joined::Now { new_room: false }
        );
    }

    #[test]
    fn the_states_are_numbered_by_the_clock_and_published_again_while_somebody_is_there() {
        let mut rooms = Rooms::default();
        rooms.join(&who("r", "a"), 1_000, 1);
        let room = rooms.get_mut("r").unwrap();
        assert_eq!(room.again_at(), None, "nothing written yet");
        let cases: [(u64, u64); 3] = [(1_000, 1_000), (1_000, 1_001), (900, 1_002)];
        for (now, group) in cases {
            let (got, payload) = room.write(now);
            assert_eq!(got, group, "at {now}");
            assert_eq!(
                read_state(&payload).map(|state| state.at),
                Ok(now),
                "at {now}"
            );
        }
        assert_eq!(room.again_at(), Some(900 + STATE_AGAIN_MS));
        assert_eq!(
            room.next_at(),
            Some(room.pet.next_at().min(900 + STATE_AGAIN_MS))
        );
        rooms.leave(&who("r", "a"), 5_000);
        let room = rooms.get("r").unwrap();
        assert_eq!(room.again_at(), None);
        assert_eq!(room.forget_at(), Some(5_000 + FORGET_AFTER_MS));
    }

    #[test]
    fn the_actions_are_numbered_by_the_clock_apart_from_the_states() {
        let mut rooms = Rooms::default();
        rooms.join(&who("r", "a"), 0, 1);
        let room = rooms.get_mut("r").unwrap();
        room.write(5_000);
        let cases: [(u64, ActionKind, u64); 3] = [
            (1_000, ActionKind::Eat { thing: 3 }, 1_000),
            (1_000, ActionKind::Take { thing: 4 }, 1_001),
            (2_000, ActionKind::Eat { thing: 5 }, 2_000),
        ];
        for (now, kind, group) in cases {
            let (got, payload) = room.write_acted(kind.clone(), now);
            assert_eq!(got, group, "at {now}");
            assert_eq!(
                read_action(&payload),
                Ok(Action { kind, sent_at: now }),
                "at {now}"
            );
        }
        assert_eq!(room.acted_group(), Some(2_000));
        assert_eq!(room.latest_group(), Some(5_000));
    }

    /// (case, start, end, order, what is answered: the Groups sent, the End
    /// Location, the indicator's Group)
    type Fetching = (
        &'static str,
        (u64, u64),
        (u64, u64),
        Result<(Vec<u64>, (u64, u64), Option<u64>), u64>,
    );

    #[test]
    fn a_fetch_gets_the_latest_state() {
        let mut rooms = Rooms::default();
        rooms.join(&who("r", "a"), 100, 1);
        let room = rooms.get_mut("r").unwrap();
        assert_eq!(
            room.fetch(
                Location {
                    group: 0,
                    object: 0
                },
                Location {
                    group: 0,
                    object: 0
                },
                GroupOrder::Ascending
            )
            .map(|_| ()),
            Err(request_error_code::INVALID_RANGE),
            "before the first state"
        );
        for now in [100, 200, 300] {
            room.write(now);
        }
        let cases: [Fetching; 4] = [
            (
                "the latest Group",
                (300, 0),
                (300, 1),
                Ok((vec![300], (300, 1), None)),
            ),
            (
                "all of them",
                (100, 0),
                (300, 1),
                Ok((vec![300], (300, 1), Some(299))),
            ),
            (
                "beyond the latest",
                (300, 0),
                (900, 0),
                Ok((vec![300], (300, 1), None)),
            ),
            (
                "after the latest",
                (301, 0),
                (900, 0),
                Err(request_error_code::INVALID_RANGE),
            ),
        ];
        for (case, (group, object), (end_group, end_object), expected) in cases {
            let got = room
                .fetch(
                    Location { group, object },
                    Location {
                        group: end_group,
                        object: end_object,
                    },
                    GroupOrder::Ascending,
                )
                .map(|(plan, states)| {
                    (
                        states
                            .into_iter()
                            .map(|(group, _)| group)
                            .collect::<Vec<_>>(),
                        (plan.end_location.group, plan.end_location.object),
                        plan.left_out.map(|(location, _)| location.group),
                    )
                });
            assert_eq!(got, expected, "{case}");
        }
    }
}
