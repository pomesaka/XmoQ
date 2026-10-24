//! The chat on `MoQ` tracks (Q36; the room's WebSocket carries none since
//! Q49): what the viewer says on its track `said`, and which lines of the
//! room's track `lines` it shows, and in what order.
//!
//! Decided on 2026-10-24: on the session of the broadcast; with a room, its
//! name and the viewer's name in the chat, without one only with
//! `--chat-room`; the 16 latest lines by a Joining FETCH, and the lines
//! missed when the subscription is made again ([`history_to_fetch`]).

use std::collections::{BTreeMap, VecDeque};

use crate::room;

/// The latest lines a viewer that comes gets (its Joining FETCH), as many as
/// are kept under the picture.
pub const RECENT_LINES: u64 = 16;

/// Lines said before the gatherer subscribes to `said` that are kept for
/// it. **Provisional.**
pub const SAID_WAITING: usize = 16;

/// How many of the lines it missed the viewer asks for when its
/// subscription to `lines` is made again, where `--chat-catch-up-lines`
/// does not say: as many as it keeps under the picture, so that it fills
/// what it keeps and no more, as the page's 200 are what it shows. Decided
/// on 2026-10-24. **Provisional** as a number.
pub const CATCH_UP_LINES: u64 = 16;

/// The most `--chat-catch-up-lines` may be: the lines one FETCH of the
/// gatherer gets by default (`--fetch-lines`), as `CATCH_UP_LINES_MOST` of
/// `@xmoq/chat`.
pub const CATCH_UP_LINES_MOST: u64 = 1000;

/// Lines that were not fetched: those after `after` and before `before`,
/// the first line fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinesGap {
    /// The last line had.
    pub after: u64,
    /// The first line fetched.
    pub before: u64,
}

/// What to ask for of the lines before a subscription to `lines`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryPlan {
    /// The numbers had are to be forgotten: the room was numbered anew.
    pub forget: bool,
    /// The first Group of the Absolute Joining FETCH, which ends at the
    /// Joining Location, and the gap before it; `None` where nothing is to
    /// be fetched.
    pub fetch: Option<(u64, Option<LinesGap>)>,
}

/// The first Group of the newest `lines` lines, a line a Group, the Group
/// of the `LARGEST_OBJECT` among them.
fn newest(largest: u64, lines: u64) -> u64 {
    largest.saturating_sub(lines.saturating_sub(1))
}

/// What to fetch for a subscription to `lines` whose `SUBSCRIBE_OK` said
/// `largest` (the Group of its `LARGEST_OBJECT`), where the viewer had lines
/// up to `had`, catching up `catch_up` of the lines it missed at the most.
/// The rule of `historyToFetch` of `@xmoq/chat`; both read the rows of
/// `fixtures/chat-history/`.
///
/// - Nothing had: the newest [`RECENT_LINES`].
/// - Lines had: from the line after `had` to `largest`; where they are more
///   than `catch_up`, the newest `catch_up` and a gap before them. Nothing
///   where `had` is `largest` already: an Absolute Joining FETCH that starts
///   above the Largest Object is refused with `INVALID_RANGE` (§10.12.3).
/// - `largest` below `had`, or none where lines were had: the room was
///   numbered anew, and it is as if nothing was had (`forget`). WHY: the
///   gatherer numbers a room on from what its store holds and keeps every
///   line (`crates/chat/README.md`), so its Groups only go up; a smaller one
///   comes of a gatherer with another store, whose numbers name other lines.
///   WHY NOT go on with what was had: its new lines under that number would
///   be passed over as shown, and the room would look silent until it
///   passed the old number.
///
/// WHY Absolute, with the Group worked out here: the Relative form leaves
/// the relay to work out "{Joining Location.Group - Joining Start, 0}"
/// (§10.12.2.1), which the draft does not say of when it cannot be
/// subtracted, and a room has fewer lines than 16 at first. The own relay
/// starts at Group 0 then; `moqtail`'s relay@0.15.0 refuses the FETCH
/// ("Joining fetch start location is larger than the track's largest
/// location", seen on 2026-10-24).
#[must_use]
pub fn history_to_fetch(had: Option<u64>, largest: Option<u64>, catch_up: u64) -> HistoryPlan {
    let (Some(had), Some(largest)) = (had, largest) else {
        return HistoryPlan {
            forget: had.is_some(),
            fetch: largest.map(|largest| (newest(largest, RECENT_LINES), None)),
        };
    };
    if largest < had {
        return HistoryPlan {
            forget: true,
            fetch: Some((newest(largest, RECENT_LINES), None)),
        };
    }
    if largest == had {
        return HistoryPlan {
            forget: false,
            fetch: None,
        };
    }
    let start = newest(largest, catch_up);
    let fetch = if start <= had.saturating_add(1) {
        (had + 1, None)
    } else {
        (
            start,
            Some(LinesGap {
                after: had,
                before: start,
            }),
        )
    };
    HistoryPlan {
        forget: false,
        fetch: Some(fetch),
    }
}

/// The line shown where lines were not fetched.
#[must_use]
pub fn shown_gap(gap: LinesGap) -> String {
    format!(
        "{} lines not fetched ({} to {})",
        gap.before - gap.after - 1,
        gap.after + 1,
        gap.before - 1
    )
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

    /// The number of the newest line shown, if any.
    #[must_use]
    pub fn last(&self) -> Option<u64> {
        self.last
    }

    /// Before the FETCH of a session whose viewer showed lines up to `had`
    /// on the session before (`lastHad` of `@xmoq/chat`).
    #[must_use]
    pub fn fetching_after(had: Option<u64>) -> Self {
        Self {
            fetching: true,
            held: BTreeMap::new(),
            last: had,
        }
    }

    /// The room was numbered anew ([`history_to_fetch`]): what was shown
    /// says nothing of the numbers from now on.
    pub fn forget(&mut self) {
        self.last = None;
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

    /// The rows of `fixtures/chat-history/`, which `@xmoq/chat` reads as
    /// well.
    #[test]
    fn what_is_fetched_for_a_subscription_to_lines() {
        let rows = include_str!("../../../fixtures/chat-history/history-to-fetch.tsv");
        let number = |field: &str| (field != "-").then(|| field.parse::<u64>().unwrap());
        let mut read = 0;
        for row in rows.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, had, largest, catch_up, forget, start, after, before] = fields[..] else {
                panic!("a row of eight fields: {row}");
            };
            let gap = match (number(after), number(before)) {
                (Some(after), Some(before)) => Some(LinesGap { after, before }),
                _ => None,
            };
            let expected = HistoryPlan {
                forget: forget == "yes",
                fetch: number(start).map(|start| (start, gap)),
            };
            assert_eq!(
                history_to_fetch(number(had), number(largest), catch_up.parse().unwrap()),
                expected,
                "{name}"
            );
            read += 1;
        }
        assert_eq!(read, 14);
    }

    #[test]
    fn a_gap_says_which_lines_were_not_fetched() {
        assert_eq!(
            shown_gap(LinesGap {
                after: 50,
                before: 52
            }),
            "1 lines not fetched (51 to 51)"
        );
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
    fn a_session_after_another_goes_on_from_what_was_shown() {
        let mut shown = Shown::fetching_after(Some(5));
        assert_eq!(shown.fetched(5, ()), None);
        assert_eq!(shown.fetched(6, ()), Some((6, ())));
        // Numbered anew: what was shown says nothing any more.
        shown.forget();
        assert_eq!(shown.last(), None);
        assert_eq!(shown.fetched(2, ()), Some((2, ())));
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
