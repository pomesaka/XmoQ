//! The numbering of the lines of each room: in the order they reach the
//! gatherer (decided on 2026-10-24), from 0, without a gap, and on from
//! what the store holds when the room is numbered again.
//!
//! A room is numbered while somebody is followed in it
//! ([`crate::gather::Gatherer::leave`]), and not from the store when the
//! gatherer starts. WHY: the history keeps every room for good, and a
//! room that is numbered holds one of [`MAX_ROOMS`] and a PUBLISH at the
//! relay; numbered from the store, a history of that many rooms would leave
//! no room for a new one, and a relay that keeps 256 requests of a session
//! would publish no more than about 250 of them.

use std::collections::HashMap;

/// The rooms the gatherer numbers at once. A participant of one room more
/// is passed over and counted, and its lines are not taken.
///
/// WHY a limit: every room numbered holds a PUBLISH at the relay and its state here,
/// and the rooms are named by whoever announces a namespace. WHY 1,024: a
/// hundred times the ten rooms that are aimed at (decided on
/// 2026-10-24). **Provisional.**
pub const MAX_ROOMS: usize = 1_024;

/// The next number of each room.
#[derive(Debug, Default)]
pub struct Rooms {
    next: HashMap<String, u64>,
}

impl Rooms {
    /// Whether the room is numbered here.
    #[must_use]
    pub fn knows(&self, room: &str) -> bool {
        self.next.contains_key(room)
    }

    /// How many rooms are numbered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.next.len()
    }

    /// Whether none is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.next.is_empty()
    }

    /// Numbers a room from now on, from `next`: one more than the largest
    /// number the store holds of it, 0 for a room it holds nothing of.
    /// `false` where [`MAX_ROOMS`] are numbered already; a room that is
    /// numbered keeps its number.
    pub fn add(&mut self, room: &str, next: u64) -> bool {
        if self.knows(room) {
            return true;
        }
        if self.next.len() >= MAX_ROOMS {
            return false;
        }
        self.next.insert(room.to_string(), next);
        true
    }

    /// Numbers the room no more. Its next number is the store's to say when
    /// it is numbered again.
    pub fn remove(&mut self, room: &str) {
        self.next.remove(room);
    }

    /// Numbers no room.
    pub fn clear(&mut self) {
        self.next.clear();
    }

    /// The largest number given in the room, `None` where none was.
    #[must_use]
    pub fn largest(&self, room: &str) -> Option<u64> {
        self.next.get(room)?.checked_sub(1)
    }

    /// The number of the next line of the room; `None` for a room that is
    /// not numbered here, and after the last number there is.
    pub fn number(&mut self, room: &str) -> Option<u64> {
        let next = self.next.get_mut(room)?;
        let number = *next;
        *next = number.checked_add(1)?;
        Some(number)
    }

    /// The line numbered `number` could not be kept: the number is given
    /// again, so that the room has no gap. Only the last number given can
    /// be.
    pub fn give_back(&mut self, room: &str, number: u64) {
        if let Some(next) = self.next.get_mut(room)
            && number.checked_add(1) == Some(*next)
        {
            *next = number;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is done to the rooms in a case, in order.
    enum Step {
        Add(&'static str, u64, bool),
        Number(&'static str, Option<u64>),
        GiveBack(&'static str, u64),
        Largest(&'static str, Option<u64>),
    }
    use Step::{Add, GiveBack, Largest, Number};

    #[test]
    fn the_lines_of_a_room_are_numbered_without_a_gap() {
        let cases: [(&str, Vec<Step>); 6] = [
            (
                "from 0",
                vec![
                    Add("r", 0, true),
                    Largest("r", None),
                    Number("r", Some(0)),
                    Number("r", Some(1)),
                    Largest("r", Some(1)),
                ],
            ),
            (
                "on from the store",
                vec![
                    Add("r", 9, true),
                    Largest("r", Some(8)),
                    Number("r", Some(9)),
                ],
            ),
            (
                "each room its own",
                vec![
                    Add("r", 0, true),
                    Add("s", 4, true),
                    Number("s", Some(4)),
                    Number("r", Some(0)),
                    Number("s", Some(5)),
                ],
            ),
            (
                "a room added again keeps its number",
                vec![
                    Add("r", 0, true),
                    Number("r", Some(0)),
                    Add("r", 0, true),
                    Number("r", Some(1)),
                ],
            ),
            (
                "a room not added",
                vec![Number("r", None), Largest("r", None)],
            ),
            (
                "a number given back",
                vec![
                    Add("r", 0, true),
                    Number("r", Some(0)),
                    Number("r", Some(1)),
                    GiveBack("r", 0),
                    GiveBack("r", 1),
                    Number("r", Some(1)),
                ],
            ),
        ];
        for (case, steps) in cases {
            let mut rooms = Rooms::default();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Add(room, next, added) => {
                        assert_eq!(rooms.add(room, next), added, "{case}, step {index}");
                    }
                    Number(room, number) => {
                        assert_eq!(rooms.number(room), number, "{case}, step {index}");
                    }
                    GiveBack(room, number) => rooms.give_back(room, number),
                    Largest(room, largest) => {
                        assert_eq!(rooms.largest(room), largest, "{case}, step {index}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_room_numbered_no_more_is_numbered_again_from_what_it_is_given() {
        let mut rooms = Rooms::default();
        assert!(rooms.add("r", 0));
        assert_eq!(rooms.number("r"), Some(0));
        rooms.remove("r");
        assert!(!rooms.knows("r"));
        assert_eq!(rooms.number("r"), None);
        assert!(rooms.add("r", 1));
        assert_eq!(rooms.number("r"), Some(1));
        rooms.clear();
        assert!(rooms.is_empty());
    }

    #[test]
    fn no_more_rooms_than_the_limit_are_numbered() {
        let mut rooms = Rooms::default();
        for index in 0..MAX_ROOMS {
            assert!(rooms.add(&index.to_string(), 0));
        }
        assert!(!rooms.add("one more", 0));
        assert!(rooms.add("0", 0), "one that is numbered already");
        assert_eq!(rooms.len(), MAX_ROOMS);
        rooms.remove("0");
        assert!(rooms.add("one more", 0), "after one is numbered no more");
    }
}
