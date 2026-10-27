//! What a followed track decides, without input or output: a track of one
//! numbered item a Group (the Group ID its number), read from a point in
//! its past by a Joining FETCH and live after it ([`crate::follow`]). Which
//! of its past to fetch ([`past_to_fetch`]), the items not fetched between
//! the two ([`Gap`]), and which items are given, each once and in the order
//! of their numbers ([`Items`]). The chat's `lines` and a transcript's
//! `speech` are followed so; `RECENT_LINES` and `CATCH_UP_LINES` of
//! `xmoq-viewer` are the chat's values.

use std::collections::BTreeMap;

/// Items that were not fetched: those after `after` and before `before`,
/// the first item fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    /// The last item had.
    pub after: u64,
    /// The first item fetched.
    pub before: u64,
}

/// What to ask for of the past of a followed track, before its subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PastToFetch {
    /// The numbers had are to be forgotten: the room was numbered anew.
    pub forget: bool,
    /// The first Group of the Absolute Joining FETCH, which ends at the
    /// Joining Location, and the gap before it; `None` where nothing is to
    /// be fetched.
    pub fetch: Option<(u64, Option<Gap>)>,
}

/// The first Group of the newest `items` items, an item a Group, the Group
/// of the `LARGEST_OBJECT` among them.
fn newest(largest: u64, items: u64) -> u64 {
    largest.saturating_sub(items.saturating_sub(1))
}

/// What to fetch for a subscription whose `SUBSCRIBE_OK` said `largest`
/// (the Group of its `LARGEST_OBJECT`), where the client had items up to
/// `had`, catching up `catch_up` of the items it missed at the most.
/// The rule of `historyToFetch` of `@xmoq/chat`; both read the rows of
/// `fixtures/chat-history/`.
///
/// - Nothing had: the newest `recent`.
/// - Items had: from the item after `had` to `largest`; where they are more
///   than `catch_up`, the newest `catch_up` and a gap before them. Nothing
///   where `had` is `largest` already: an Absolute Joining FETCH that starts
///   above the Largest Object is refused with `INVALID_RANGE` (§10.12.3).
/// - `largest` below `had`, or none where items were had: the track was
///   numbered anew, and it is as if nothing was had (`forget`). WHY, of the
///   chat's `lines`: the gatherer numbers a room on from what its store holds and keeps every
///   line (`crates/chat/README.md`), so its Groups only go up; a smaller one
///   comes of a gatherer with another store, whose numbers name other lines.
///   WHY NOT go on with what was had: its new lines under that number would
///   be passed over as shown, and the room would look silent until it
///   passed the old number.
///
/// WHY Absolute, with the Group worked out here: the Relative form leaves
/// the relay to work out "{Joining Location.Group - Joining Start, 0}"
/// (§10.12.2.1), which the draft does not say of when it cannot be
/// subtracted, and a chat's room has fewer lines than 16 at first. The own relay
/// starts at Group 0 then; `moqtail`'s relay@0.15.0 refuses the FETCH
/// ("Joining fetch start location is larger than the track's largest
/// location", seen on 2026-10-24).
#[must_use]
pub fn past_to_fetch(
    had: Option<u64>,
    largest: Option<u64>,
    recent: u64,
    catch_up: u64,
) -> PastToFetch {
    let (Some(had), Some(largest)) = (had, largest) else {
        return PastToFetch {
            forget: had.is_some(),
            fetch: largest.map(|largest| (newest(largest, recent), None)),
        };
    };
    if largest < had {
        return PastToFetch {
            forget: true,
            fetch: Some((newest(largest, recent), None)),
        };
    }
    if largest == had {
        return PastToFetch {
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
            Some(Gap {
                after: had,
                before: start,
            }),
        )
    };
    PastToFetch {
        forget: false,
        fetch: Some(fetch),
    }
}

/// Which items of a followed track are given: each once, in the order of
/// their numbers.
///
/// WHY live items wait while the FETCH is open: the FETCH brings the items
/// before the ones of the subscription, and shown as they come the later
/// ones would come first. WHY each once: the range of a Joining FETCH and
/// the subscription can overlap (the row on a Joining FETCH in the relay
/// of docs/moqt-implementation-notes.md).
#[derive(Debug)]
pub struct Items<T> {
    /// How many live items wait at most while the FETCH is open.
    recent: u64,
    fetching: bool,
    held: BTreeMap<u64, T>,
    last: Option<u64>,
}

impl<T> Items<T> {
    /// Before the FETCH has ended.
    #[must_use]
    pub fn fetching(recent: u64) -> Self {
        Self {
            recent,
            fetching: true,
            held: BTreeMap::new(),
            last: None,
        }
    }

    /// A new subscription and its FETCH, after one that ended: the items
    /// of both wait again for the FETCH, and what was shown stays shown.
    pub fn refetching(&mut self) {
        self.fetching = true;
        self.held.clear();
    }

    /// The number of the newest item given, if any.
    #[must_use]
    pub fn last(&self) -> Option<u64> {
        self.last
    }

    /// Before the FETCH of a session whose client gave items up to `had`
    /// on the session before (`lastHad` of `@xmoq/chat`).
    #[must_use]
    pub fn fetching_after(had: Option<u64>, recent: u64) -> Self {
        Self {
            recent,
            fetching: true,
            held: BTreeMap::new(),
            last: had,
        }
    }

    /// The room was numbered anew ([`past_to_fetch`]): what was shown
    /// says nothing of the numbers from now on.
    pub fn forget(&mut self) {
        self.last = None;
        self.held.clear();
    }

    /// An item of the FETCH: given unless one with its number or a later
    /// one was.
    pub fn fetched(&mut self, number: u64, item: T) -> Option<(u64, T)> {
        self.show(number, item)
    }

    /// The FETCH ended, or was refused: the live items that waited.
    pub fn fetch_ended(&mut self) -> Vec<(u64, T)> {
        self.fetching = false;
        let held = std::mem::take(&mut self.held);
        held.into_iter()
            .filter_map(|(number, item)| self.show(number, item))
            .collect()
    }

    /// The FETCH was cancelled with its subscription, of a track found
    /// malformed (§2.4.2): the live items that waited are let go of, and the
    /// last shown stays where it was, so that the FETCH of the next
    /// subscription asks for the items from there.
    ///
    /// WHY NOT given as at [`Self::fetch_ended`]: the items of the FETCH not
    /// read yet come before them, and showing them would make the last pass
    /// over those without a gap said.
    pub fn fetch_given_up(&mut self) {
        self.fetching = false;
        self.held.clear();
    }

    /// An item of the subscription. While the FETCH is open the latest
    /// `recent` wait.
    pub fn live(&mut self, number: u64, item: T) -> Option<(u64, T)> {
        if !self.fetching {
            return self.show(number, item);
        }
        self.held.insert(number, item);
        if self.held.len() > usize::try_from(self.recent).unwrap_or(usize::MAX) {
            self.held.pop_first();
        }
        None
    }

    fn show(&mut self, number: u64, item: T) -> Option<(u64, T)> {
        if self.last.is_some_and(|last| number <= last) {
            return None;
        }
        self.last = Some(number);
        Some((number, item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `fixtures/chat-history/`, which `@xmoq/chat` reads as
    /// well.
    #[test]
    fn what_is_fetched_for_a_subscription_to_lines() {
        let rows = include_str!("../../../fixtures/chat-history/history-to-fetch.tsv");
        // The newest lines of a room had nothing: the chat's, which
        // `fixtures/shared-values/` holds.
        let recent = include_str!("../../../fixtures/shared-values/values.tsv")
            .lines()
            .find_map(|row| row.strip_prefix("chat-recent-lines\t"))
            .and_then(|rest| rest.split('\t').next())
            .map(|value| value.parse::<u64>().unwrap())
            .unwrap();
        let number = |field: &str| (field != "-").then(|| field.parse::<u64>().unwrap());
        let mut read = 0;
        for row in rows.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, had, largest, catch_up, forget, start, after, before] = fields[..] else {
                panic!("a row of eight fields: {row}");
            };
            let gap = match (number(after), number(before)) {
                (Some(after), Some(before)) => Some(Gap { after, before }),
                _ => None,
            };
            let expected = PastToFetch {
                forget: forget == "yes",
                fetch: number(start).map(|start| (start, gap)),
            };
            assert_eq!(
                past_to_fetch(
                    number(had),
                    number(largest),
                    recent,
                    catch_up.parse().unwrap()
                ),
                expected,
                "{name}"
            );
            read += 1;
        }
        assert_eq!(read, 14);
    }

    /// What comes: `Some((true for the FETCH, number))`, or `None` for the
    /// end of the FETCH.
    type Comes = &'static [Option<(bool, u64)>];

    #[test]
    fn items_are_given_once_in_the_order_of_their_numbers() {
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
            let mut shown = Items::fetching(16);
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
        let mut shown = Items::fetching(16);
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
        let mut shown = Items::fetching_after(Some(5), 16);
        assert_eq!(shown.fetched(5, ()), None);
        assert_eq!(shown.fetched(6, ()), Some((6, ())));
        // Numbered anew: what was shown says nothing any more.
        shown.forget();
        assert_eq!(shown.last(), None);
        assert_eq!(shown.fetched(2, ()), Some((2, ())));
    }

    #[test]
    fn a_fetch_given_up_shows_none_of_what_waited_and_the_next_goes_on_from_the_last() {
        // (lines of the FETCH read before it was given up; live lines that
        //  waited; the last shown after), after a session that showed 2
        let cases: [(&[u64], &[u64], u64); 3] =
            [(&[3, 4], &[9, 10], 4), (&[], &[9], 2), (&[3], &[], 3)];
        for (fetched, waited, last) in cases {
            let mut shown = Items::fetching_after(Some(2), 16);
            for number in fetched {
                shown.fetched(*number, ());
            }
            for number in waited {
                assert_eq!(shown.live(*number, ()), None);
            }
            shown.fetch_given_up();
            assert_eq!(shown.last(), Some(last), "{fetched:?} {waited:?}");
            // The next subscription's FETCH brings what was left out.
            shown.refetching();
            assert_eq!(
                shown.fetched(last + 1, ()),
                Some((last + 1, ())),
                "{fetched:?} {waited:?}"
            );
            assert_eq!(shown.fetch_ended(), [], "{fetched:?} {waited:?}");
        }
    }

    #[test]
    fn while_the_fetch_is_open_the_latest_sixteen_live_lines_wait() {
        let mut shown = Items::fetching(16);
        for number in 0..20 {
            assert_eq!(shown.live(number, ()), None);
        }
        let numbers: Vec<u64> = shown.fetch_ended().into_iter().map(|(n, ())| n).collect();
        assert_eq!(numbers, (4..20).collect::<Vec<u64>>());
    }
}
