//! The chat on `MoQ` tracks (the room carries none): what the
//! viewer says on its track `said`, and which lines of the room's track
//! `lines` it shows, and in what order.
//!
//! Decided on 2026-10-24: on the session of the broadcast; with a room, its
//! kind and name and the viewer's name in the chat, without one only with
//! `--chat-room`; the 16 latest lines by a Joining FETCH, and the lines
//! missed when the subscription is made again (`past_to_fetch` of
//! `xmoq-subscriber`, with [`RECENT_LINES`] and [`CATCH_UP_LINES`]).

use std::collections::VecDeque;

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

/// The line shown where lines were not fetched: those after `after`, the
/// last line had, and before `before`, the first fetched (the `Gap` of a
/// followed track, `xmoq-subscriber`).
#[must_use]
pub fn shown_gap(after: u64, before: u64) -> String {
    format!(
        "{} lines not fetched ({} to {})",
        before - after - 1,
        after + 1,
        before - 1
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_says_which_lines_were_not_fetched() {
        assert_eq!(shown_gap(50, 52), "1 lines not fetched (51 to 51)");
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
}
