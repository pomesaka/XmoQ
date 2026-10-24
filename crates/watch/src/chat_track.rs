//! The chat on `MoQ` tracks (Q36; the room's WebSocket carries none since
//! Q49): what the viewer says on its track `said`, and which lines of the
//! room's track `lines` it shows, and in what order.
//!
//! Decided on 2026-10-24: on the session of the broadcast; with a room, its
//! name and the viewer's name in the chat, without one only with
//! `--chat-room`; the 16 latest lines by a Joining FETCH.

use std::collections::{BTreeMap, VecDeque};

use crate::room;

/// The latest lines a viewer that comes gets (its Joining FETCH), as many as
/// are kept under the picture.
pub const RECENT_LINES: u64 = 16;

/// Lines said before the gatherer subscribes to `said` that are kept for
/// it. **Provisional.**
pub const SAID_WAITING: usize = 16;

/// The Group an Absolute Joining FETCH for the [`RECENT_LINES`] latest lines
/// starts with, from the Group of the `LARGEST_OBJECT` of the subscription's
/// `SUBSCRIBE_OK` (its Joining Location); `None` where it had none, and no
/// line is there to fetch.
///
/// WHY Absolute, with the Group worked out here: the Relative form leaves
/// the relay to work out "{Joining Location.Group - Joining Start, 0}"
/// (§10.12.2.1), which the draft does not say of when it cannot be
/// subtracted, and a room has fewer lines than 16 at first. The own relay
/// starts at Group 0 then; `moqtail`'s relay@0.15.0 refuses the FETCH
/// ("Joining fetch start location is larger than the track's largest
/// location", seen on 2026-10-24). The range is the same either way.
#[must_use]
pub fn fetch_start(largest_group: Option<u64>) -> Option<u64> {
    largest_group.map(|group| group.saturating_sub(RECENT_LINES - 1))
}

/// The name of the room of an address of a room, `http://host:port/rooms/<name>`:
/// its last path segment. `None` where it has none.
#[must_use]
pub fn room_of_address(address: &str) -> Option<&str> {
    let path = address.split_once("://").map_or(address, |(_, rest)| rest);
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let name = path.rsplit('/').next()?;
    (!name.is_empty() && path.contains('/')).then_some(name)
}

/// A line of `lines` as it is shown: its number in the room, who said it,
/// and what, made safe for a terminal.
#[must_use]
pub fn shown(number: u64, name: &str, text: &str) -> String {
    room::clean(&format!("{number} {name}: {text}"), room::LINE_LIMIT)
}

/// What becomes of a line the viewer says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Say {
    /// To be written now, as the Group with this ID.
    Now(u64, String),
    /// Kept until the gatherer subscribes.
    Held,
    /// Not sent: [`SAID_WAITING`] lines wait already.
    Dropped,
}

/// The Groups of `said`: one a line, numbered from 0 on a session.
///
/// WHY from 0 on each session: `said` is the track of this session's
/// namespace, and the gatherer subscribes to it anew on a session (the
/// numbers of a room are the gatherer's, not these).
#[derive(Debug, Default)]
pub struct Saying {
    next_group: u64,
    waiting: VecDeque<String>,
    subscribed: bool,
}

impl Saying {
    /// A line the viewer says.
    pub fn say(&mut self, text: String) -> Say {
        if self.subscribed {
            return Say::Now(self.take_group(), text);
        }
        if self.waiting.len() >= SAID_WAITING {
            return Say::Dropped;
        }
        self.waiting.push_back(text);
        Say::Held
    }

    /// The gatherer subscribed to `said`: the lines that waited, to be
    /// written now in the order they were said.
    pub fn subscribed(&mut self) -> Vec<(u64, String)> {
        self.subscribed = true;
        let waiting: Vec<String> = self.waiting.drain(..).collect();
        waiting
            .into_iter()
            .map(|text| (self.take_group(), text))
            .collect()
    }

    /// The subscription is over: lines wait again.
    pub fn unsubscribed(&mut self) {
        self.subscribed = false;
    }

    fn take_group(&mut self) -> u64 {
        let group = self.next_group;
        self.next_group = self.next_group.saturating_add(1);
        group
    }
}

/// Which lines of `lines` are shown: each once, in the order of their
/// numbers.
///
/// WHY live lines wait while the FETCH is open: the FETCH brings the lines
/// before the ones of the subscription, and shown as they come the later
/// ones would come first. WHY each once: the range of a Joining FETCH and
/// the subscription can overlap (the row on a Joining FETCH in the relay
/// of docs/moqt-implementation-notes.md).
#[derive(Debug)]
pub struct Shown<T> {
    fetching: bool,
    held: BTreeMap<u64, T>,
    last: Option<u64>,
}

impl<T> Shown<T> {
    /// Before the FETCH has ended.
    #[must_use]
    pub fn fetching() -> Self {
        Self {
            fetching: true,
            held: BTreeMap::new(),
            last: None,
        }
    }

    /// A new subscription and its FETCH, after one that ended: the lines
    /// of both wait again for the FETCH, and what was shown stays shown.
    pub fn refetching(&mut self) {
        self.fetching = true;
        self.held.clear();
    }

    /// A line of the FETCH: shown unless one with its number or a later
    /// one was.
    pub fn fetched(&mut self, number: u64, line: T) -> Option<(u64, T)> {
        self.show(number, line)
    }

    /// The FETCH ended, or was refused: the live lines that waited.
    pub fn fetch_ended(&mut self) -> Vec<(u64, T)> {
        self.fetching = false;
        let held = std::mem::take(&mut self.held);
        held.into_iter()
            .filter_map(|(number, line)| self.show(number, line))
            .collect()
    }

    /// A line of the subscription. While the FETCH is open the latest
    /// [`RECENT_LINES`] wait.
    pub fn live(&mut self, number: u64, line: T) -> Option<(u64, T)> {
        if !self.fetching {
            return self.show(number, line);
        }
        self.held.insert(number, line);
        if self.held.len() > usize::try_from(RECENT_LINES).unwrap_or(usize::MAX) {
            self.held.pop_first();
        }
        None
    }

    fn show(&mut self, number: u64, line: T) -> Option<(u64, T)> {
        if self.last.is_some_and(|last| number <= last) {
            return None;
        }
        self.last = Some(number);
        Some((number, line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_room_is_the_last_segment_of_its_address() {
        let cases = [
            ("http://h:8787/rooms/a", Some("a")),
            ("http://h/rooms/demo-1?x=1", Some("demo-1")),
            ("https://h/rooms/b#top", Some("b")),
            ("http://h/rooms/", None),
            ("http://h", None),
        ];
        for (address, room) in cases {
            assert_eq!(room_of_address(address), room, "{address}");
        }
    }

    #[test]
    fn the_joining_fetch_reaches_back_sixteen_lines() {
        // (the Group of the Joining Location; the Group the FETCH starts with)
        let cases = [
            (None, None),
            (Some(0), Some(0)),
            (Some(3), Some(0)),
            (Some(15), Some(0)),
            (Some(16), Some(1)),
            (Some(100), Some(85)),
        ];
        for (largest, start) in cases {
            assert_eq!(fetch_start(largest), start, "{largest:?}");
        }
    }

    #[test]
    fn a_line_shows_its_number_and_who_said_it() {
        let cases = [
            (shown(7, "ann", "hi"), "7 ann: hi"),
            // Made safe for a terminal.
            (shown(1, "x\u{1b}[2J", "y"), "1 x[2J: y"),
        ];
        for (shown, expected) in cases {
            assert_eq!(shown, expected);
        }
    }

    #[test]
    fn lines_said_before_the_gatherer_subscribes_wait_for_it() {
        let mut saying = Saying::default();
        assert_eq!(saying.say("a".into()), Say::Held);
        assert_eq!(saying.say("b".into()), Say::Held);
        assert_eq!(
            saying.subscribed(),
            [(0, "a".to_string()), (1, "b".to_string())]
        );
        assert_eq!(saying.say("c".into()), Say::Now(2, "c".into()));
        saying.unsubscribed();
        assert_eq!(saying.say("d".into()), Say::Held);
        assert_eq!(saying.subscribed(), [(3, "d".to_string())]);
    }

    #[test]
    fn no_more_than_sixteen_lines_wait() {
        let mut saying = Saying::default();
        for n in 0..16 {
            assert_eq!(saying.say(format!("{n}")), Say::Held, "{n}");
        }
        assert_eq!(saying.say("17th".into()), Say::Dropped);
        assert_eq!(saying.subscribed().len(), 16);
    }

    /// What comes: `Some((true for the FETCH, number))`, or `None` for the
    /// end of the FETCH.
    type Comes = &'static [Option<(bool, u64)>];

    #[test]
    fn lines_are_shown_once_in_the_order_of_their_numbers() {
        // (what comes: Some((true for the FETCH, number)) or None for the
        //  end of the FETCH; the numbers shown at each)
        let cases: [(Comes, &[u64]); 4] = [
            // The FETCH, then the subscription.
            (
                &[Some((true, 4)), Some((true, 5)), None, Some((false, 6))],
                &[4, 5, 6],
            ),
            // The subscription first: its lines wait for the FETCH.
            (
                &[Some((false, 6)), Some((true, 4)), Some((true, 5)), None],
                &[4, 5, 6],
            ),
            // The two overlap: 5 comes both ways.
            (
                &[
                    Some((false, 5)),
                    Some((false, 6)),
                    Some((true, 4)),
                    Some((true, 5)),
                    None,
                ],
                &[4, 5, 6],
            ),
            // A FETCH that brings nothing (a room without lines yet).
            (&[Some((false, 0)), None, Some((false, 1))], &[0, 1]),
        ];
        for (comes, expected) in cases {
            let mut shown = Shown::fetching();
            let mut numbers = Vec::new();
            for event in comes {
                match event {
                    Some((true, number)) => {
                        numbers.extend(shown.fetched(*number, ()).map(|(n, ())| n));
                    }
                    Some((false, number)) => {
                        numbers.extend(shown.live(*number, ()).map(|(n, ())| n));
                    }
                    None => numbers.extend(shown.fetch_ended().into_iter().map(|(n, ())| n)),
                }
            }
            assert_eq!(numbers, expected, "{comes:?}");
        }
    }

    #[test]
    fn a_new_subscription_shows_only_the_lines_after_those_shown() {
        let mut shown = Shown::fetching();
        shown.fetched(3, ());
        shown.fetch_ended();
        shown.live(4, ());
        // Subscribed again: its FETCH brings 1 to 6, its subscription 7.
        shown.refetching();
        assert_eq!(shown.live(7, ()), None);
        let mut numbers: Vec<u64> = (1..=6)
            .filter_map(|n| shown.fetched(n, ()).map(|(n, ())| n))
            .collect();
        numbers.extend(shown.fetch_ended().into_iter().map(|(n, ())| n));
        assert_eq!(numbers, [5, 6, 7]);
    }

    #[test]
    fn while_the_fetch_is_open_the_latest_sixteen_live_lines_wait() {
        let mut shown = Shown::fetching();
        for number in 0..20 {
            assert_eq!(shown.live(number, ()), None);
        }
        let numbers: Vec<u64> = shown.fetch_ended().into_iter().map(|(n, ())| n).collect();
        assert_eq!(numbers, (4..20).collect::<Vec<u64>>());
    }
}
