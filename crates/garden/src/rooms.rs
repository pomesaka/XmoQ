//! The rooms that have a garden: who acts in each, its garden, and the
//! gardens it published. A room is kept in memory alone, from its first
//! actor until [`FORGET_AFTER_MS`] after its last one left; nothing is
//! written to a disk, and a garden that is forgotten begins anew.

use std::collections::{BTreeSet, HashMap};

use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_pet_tracks::names::Actor;
use xmoq_pet_tracks::payloads::garden_payload;
use xmoq_publisher::fetch::{Plan, PublishedGroups, plan_kept};

use crate::rules::RoomGarden;

/// The rooms that have a garden at once, those whose last actor left and
/// that are not forgotten yet among them. An actor of one room more is
/// passed over. WHY a limit: each room holds a PUBLISH at the relay and a
/// garden here, and the rooms are named by whoever announces. WHY 64: the
/// own relay keeps 256 requests of a session (`MAX_PEER_REQUESTS` of
/// `xmoq-moqt`), which the PUBLISH of each room and the SUBSCRIBE of each
/// actor share. **Provisional.**
pub const MAX_ROOMS: usize = 64;

/// The actors followed at once, in every room. One more is passed over.
/// **Provisional**, as [`MAX_ROOMS`]; the relay's limit of requests comes
/// first.
pub const MAX_ACTORS: usize = 1_024;

/// How long a room's garden is kept, and published, after its last actor
/// left. WHY kept at all: one who reloads the page finds the same garden.
/// **Provisional.**
pub const FORGET_AFTER_MS: u64 = 60_000;

/// How often the garden of a room is published again without a change, so
/// that one who comes gets a garden soon from a live Group. **Provisional.**
pub const GARDEN_AGAIN_MS: u64 = 2_000;

/// A garden a FETCH gets: its Group ID and its payload.
pub type Fetched = (u64, Vec<u8>);

/// A garden written: its Group ID and payload, and how many things and
/// events it has, for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Written {
    /// The Group ID.
    pub group: u64,
    /// The payload of `garden`.
    pub payload: Vec<u8>,
    /// How many things lie on its floor.
    pub things: usize,
    /// How many events it has.
    pub events: usize,
}

/// One room's garden and who acts in it.
#[derive(Debug, Default)]
pub struct Room {
    /// The garden.
    pub garden: RoomGarden,
    actors: BTreeSet<String>,
    left_at: Option<u64>,
    first_group: Option<u64>,
    latest: Option<(u64, Vec<u8>)>,
    written_at: u64,
}

impl Room {
    /// Whether some actor is in the room.
    #[must_use]
    pub fn has_actors(&self) -> bool {
        !self.actors.is_empty()
    }

    /// The garden written at `now`, kept as the latest. The Group ID is
    /// `now`, or one more than the last where that is not more. WHY the
    /// clock: a PUBLISH of a later session numbers on above what an earlier
    /// one sent, as the chat's counts do, and a relay would take Groups below
    /// those it has for old.
    pub fn write(&mut self, now: u64) -> Written {
        let garden = self.garden.write(now);
        let group = self
            .latest
            .as_ref()
            .map_or(0, |(last, _)| last.saturating_add(1))
            .max(now);
        let payload = garden_payload(&garden);
        self.first_group.get_or_insert(group);
        self.latest = Some((group, payload.clone()));
        self.written_at = now;
        Written {
            group,
            payload,
            things: garden.things.len(),
            events: garden.events.len(),
        }
    }

    /// The Group ID of the latest garden; `None` before the first.
    #[must_use]
    pub fn latest_group(&self) -> Option<u64> {
        self.latest.as_ref().map(|(group, _)| *group)
    }

    /// When the garden is to be published again without a change; `None`
    /// before the first. WHY also while nobody is in the room: the room is
    /// published until it is forgotten, so that one who reloads the page
    /// gets a garden at once.
    #[must_use]
    pub fn again_at(&self) -> Option<u64> {
        self.latest
            .is_some()
            .then(|| self.written_at.saturating_add(GARDEN_AGAIN_MS))
    }

    /// When the room is forgotten; `None` while some actor is in it.
    #[must_use]
    pub fn forget_at(&self) -> Option<u64> {
        self.left_at
            .map(|left| left.saturating_add(FORGET_AFTER_MS))
    }

    /// When something of the room is next to be done.
    #[must_use]
    pub fn next_at(&self) -> Option<u64> {
        [self.again_at(), self.forget_at()]
            .into_iter()
            .flatten()
            .min()
    }

    /// What a Standalone FETCH from `start` to the End Location `end` gets of
    /// `garden` (`plan_kept` of `xmoq-publisher`): the latest garden, where
    /// it lies in the range. The Groups before it that were published get
    /// the End of Unknown Range indicator: no more than the latest is kept.
    ///
    /// # Errors
    ///
    /// `INVALID_RANGE` before the first garden, and for a range that starts
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
        let gardens = match (planned.groups, &self.latest) {
            (Some(_), Some(latest)) => vec![latest.clone()],
            _ => Vec::new(),
        };
        Ok((planned, gardens))
    }
}

/// Whether an actor is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joined {
    /// From now on.
    Now {
        /// Its room had no garden before.
        new_room: bool,
    },
    /// It was before.
    Already,
    /// [`MAX_ROOMS`] have a garden.
    TooManyRooms,
    /// [`MAX_ACTORS`] are followed.
    TooManyActors,
}

impl Joined {
    /// One word for the log, of one that is not followed.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Now { .. } => "now",
            Self::Already => "already",
            Self::TooManyRooms => "too-many-rooms",
            Self::TooManyActors => "too-many-actors",
        }
    }
}

/// What became of the room of an actor who left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Left {
    /// The actor was not followed.
    NotThere,
    /// Some other actor is in the room.
    RoomStays,
    /// None is: the room is forgotten after [`FORGET_AFTER_MS`].
    RoomEmpty,
}

/// Every room that has a garden.
#[derive(Debug, Default)]
pub struct Rooms {
    rooms: HashMap<String, Room>,
    actors: usize,
}

impl Rooms {
    /// Follows `actor`; a room that has no garden gets one.
    pub fn join(&mut self, actor: &Actor) -> Joined {
        if self.follows(actor) {
            return Joined::Already;
        }
        if self.actors >= MAX_ACTORS {
            return Joined::TooManyActors;
        }
        let new_room = !self.rooms.contains_key(&actor.room);
        if new_room && self.rooms.len() >= MAX_ROOMS {
            return Joined::TooManyRooms;
        }
        let room = self.rooms.entry(actor.room.clone()).or_default();
        room.actors.insert(actor.name.clone());
        room.left_at = None;
        self.actors += 1;
        Joined::Now { new_room }
    }

    /// Follows the actor no more.
    pub fn leave(&mut self, actor: &Actor, now: u64) -> Left {
        let Some(room) = self.rooms.get_mut(&actor.room) else {
            return Left::NotThere;
        };
        if !room.actors.remove(&actor.name) {
            return Left::NotThere;
        }
        self.actors -= 1;
        if room.has_actors() {
            return Left::RoomStays;
        }
        room.left_at = Some(now);
        Left::RoomEmpty
    }

    /// Follows nobody, as before the first NAMESPACE of a session: each
    /// room is forgotten [`FORGET_AFTER_MS`] from `now` unless some actor
    /// comes again. WHY the gardens are kept: a session that connects again
    /// is told again who is in the rooms, and finds the same gardens.
    pub fn leave_everybody(&mut self, now: u64) {
        for room in self.rooms.values_mut() {
            if room.has_actors() {
                room.actors.clear();
                room.left_at = Some(now);
            }
        }
        self.actors = 0;
    }

    /// Whether the actor is followed.
    #[must_use]
    pub fn follows(&self, actor: &Actor) -> bool {
        self.rooms
            .get(&actor.room)
            .is_some_and(|room| room.actors.contains(&actor.name))
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
    use xmoq_pet_tracks::payloads::{ActionKind, read_garden};

    use super::*;

    fn who(room: &str, name: &str) -> Actor {
        Actor {
            room: room.to_string(),
            name: name.to_string(),
        }
    }

    enum Step {
        Join(&'static str, &'static str, Joined),
        Leave(&'static str, &'static str, u64, Left),
        Everybody(u64),
        Forget(u64, &'static [&'static str]),
    }
    use Step::{Everybody, Forget, Join, Leave};

    #[test]
    fn a_room_has_a_garden_while_some_actor_is_in_it_and_a_while_after() {
        let new = Joined::Now { new_room: true };
        let old = Joined::Now { new_room: false };
        let cases: [(&str, Vec<Step>); 5] = [
            (
                "two of one room",
                vec![
                    Join("r", "a", new),
                    Join("r", "b", old),
                    Join("r", "a", Joined::Already),
                    Leave("r", "a", 5, Left::RoomStays),
                    Leave("r", "b", 10, Left::RoomEmpty),
                    Leave("r", "b", 10, Left::NotThere),
                    Forget(60_009, &[]),
                    Forget(60_010, &["r"]),
                    Join("r", "a", new),
                ],
            ),
            (
                "one who comes back in time keeps the garden",
                vec![
                    Join("r", "a", new),
                    Leave("r", "a", 0, Left::RoomEmpty),
                    Join("r", "a", old),
                    Forget(180_000, &[]),
                ],
            ),
            (
                "a new session",
                vec![
                    Join("r", "a", new),
                    Everybody(100),
                    Leave("r", "a", 100, Left::NotThere),
                    Join("r", "a", old),
                    Forget(60_100, &[]),
                    Everybody(300),
                    Forget(60_300, &["r"]),
                ],
            ),
            (
                "somebody never followed",
                vec![Leave("r", "a", 0, Left::NotThere)],
            ),
            (
                "rooms apart",
                vec![
                    Join("r", "a", new),
                    Join("s", "a", new),
                    Leave("s", "a", 0, Left::RoomEmpty),
                    Forget(60_000, &["s"]),
                ],
            ),
        ];
        assert_eq!(FORGET_AFTER_MS, 60_000);
        for (case, steps) in cases {
            let mut rooms = Rooms::default();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Join(room, name, joined) => {
                        assert_eq!(rooms.join(&who(room, name)), joined, "{case}, {index}");
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
    fn no_more_rooms_and_actors_than_the_limits() {
        let mut rooms = Rooms::default();
        for index in 0..MAX_ROOMS {
            assert_eq!(
                rooms.join(&who(&index.to_string(), "a")),
                Joined::Now { new_room: true }
            );
        }
        assert_eq!(rooms.join(&who("one more", "a")), Joined::TooManyRooms);
        for index in MAX_ROOMS..MAX_ACTORS {
            assert_eq!(
                rooms.join(&who("0", &index.to_string())),
                Joined::Now { new_room: false }
            );
        }
        assert_eq!(rooms.join(&who("1", "one more")), Joined::TooManyActors);
        assert_eq!(rooms.leave(&who("1", "a"), 0), Left::RoomEmpty);
        assert_eq!(
            rooms.join(&who("1", "one more")),
            Joined::Now { new_room: false }
        );
    }

    #[test]
    fn the_gardens_are_numbered_by_the_clock_and_published_again_until_the_room_is_forgotten() {
        let mut rooms = Rooms::default();
        rooms.join(&who("r", "a"));
        let room = rooms.get_mut("r").unwrap();
        assert_eq!(room.again_at(), None, "nothing written yet");
        room.garden.act("a", &ActionKind::Call, 1_000).unwrap();
        let cases: [(u64, u64); 3] = [(1_000, 1_000), (1_000, 1_001), (900, 1_002)];
        for (now, group) in cases {
            let written = room.write(now);
            assert_eq!(written.group, group, "at {now}");
            assert_eq!((written.things, written.events), (0, 1), "at {now}");
            assert_eq!(
                read_garden(&written.payload).map(|garden| garden.at),
                Ok(now),
                "at {now}"
            );
        }
        assert_eq!(room.again_at(), Some(2_900));
        assert_eq!(room.next_at(), Some(2_900));
        rooms.leave(&who("r", "a"), 5_000);
        let room = rooms.get("r").unwrap();
        assert_eq!(room.again_at(), Some(2_900));
        assert_eq!(room.forget_at(), Some(65_000));
    }

    /// (case, start, end, what is answered: the Groups sent, the End
    /// Location, the indicator's Group)
    type Fetching = (
        &'static str,
        (u64, u64),
        (u64, u64),
        Result<(Vec<u64>, (u64, u64), Option<u64>), u64>,
    );

    #[test]
    fn a_fetch_gets_the_latest_garden() {
        let mut rooms = Rooms::default();
        rooms.join(&who("r", "a"));
        let room = rooms.get_mut("r").unwrap();
        let origin = Location {
            group: 0,
            object: 0,
        };
        assert_eq!(
            room.fetch(origin, origin, GroupOrder::Ascending)
                .map(|_| ()),
            Err(request_error_code::INVALID_RANGE),
            "before the first garden"
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
                .map(|(plan, gardens)| {
                    (
                        gardens
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
