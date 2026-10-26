//! The transcript of one prefix: its pieces numbered, and the latest kept
//! for FETCH (decided on 2026-10-25: in memory, none kept on disk; whether
//! and how long to keep them is in `docs/open-questions.md`).

use std::collections::VecDeque;

use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_publisher::fetch::{Plan, PublishedGroups, plan_kept};
use xmoq_transcript_tracks::payloads::Speech;

/// A piece as a FETCH sends it: its number and its payload.
pub type Fetched = (u64, Vec<u8>);

/// The pieces of one transcript.
#[derive(Debug)]
pub struct Transcript {
    /// The number of the first piece.
    first: u64,
    next: u64,
    kept: VecDeque<Speech>,
    keep: usize,
}

/// A piece as it was read, before it is numbered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPiece {
    /// The namespace of the broadcast, each field as UTF-8.
    pub from: Vec<String>,
    /// The words.
    pub text: String,
    /// Where the piece begins on the clock of the broadcast's Timestamps.
    pub start_us: u64,
    /// Where it ends.
    pub end_us: u64,
    /// The transcriber's clock when it had read it, in milliseconds since
    /// the Unix epoch.
    pub at: u64,
}

impl Transcript {
    /// A transcript whose first piece is numbered `first`, and that keeps
    /// the latest `keep`.
    ///
    /// WHY the caller gives the first number: the transcriber numbers from
    /// the time it started, in milliseconds since the Unix epoch, so that a
    /// transcriber that starts again goes on above the numbers it gave
    /// before, which a relay and its subscribers may still have. WHY NOT
    /// from 0, as the gatherer of the chat numbers on from its store: the
    /// transcriber keeps nothing, and its next Group would be below the
    /// Largest Object a subscriber was told of (§10.2.11), which it takes for
    /// an Object of the past.
    #[must_use]
    pub fn new(first: u64, keep: usize) -> Self {
        Self {
            first,
            next: first,
            kept: VecDeque::new(),
            keep,
        }
    }

    /// Numbers a piece and keeps it, letting go of the oldest past
    /// [`Self::new`]'s `keep`.
    pub fn add(&mut self, said: ReadPiece) -> Speech {
        let speech = Speech {
            number: self.next,
            from: said.from,
            text: said.text,
            start_us: said.start_us,
            end_us: said.end_us,
            at: said.at,
        };
        self.next += 1;
        if self.kept.len() >= self.keep {
            self.kept.pop_front();
        }
        self.kept.push_back(speech.clone());
        speech
    }

    /// The number of the latest piece; `None` before the first.
    #[must_use]
    pub fn largest(&self) -> Option<u64> {
        self.kept.back().map(|speech| speech.number)
    }

    /// What a Standalone FETCH from `start` to the End Location `end` gets
    /// (`plan_kept` of `xmoq-publisher`): the plan, and the pieces kept
    /// in the range, each `(number, payload)`, in the order they are sent.
    /// The pieces of the range that were let go of get the End of Unknown
    /// Range indicator. The numbers before the first piece were never a
    /// piece: in Descending order the indicator names none of them, and in
    /// Ascending order it covers them with the rest of the range's start
    /// (why, at `plan_kept`).
    ///
    /// # Errors
    ///
    /// `INVALID_RANGE` before the first piece, and for a range that starts
    /// after the latest.
    pub fn fetch(
        &self,
        start: Location,
        end: Location,
        order: GroupOrder,
    ) -> Result<(Plan, Vec<Fetched>), u64> {
        let kept = self
            .kept
            .front()
            .zip(self.kept.back())
            .map(|(oldest, latest)| PublishedGroups {
                first: self.first,
                first_kept: oldest.number,
                largest: latest.number,
            });
        // No limit but what is kept: an answer holds no more than `keep`.
        let planned = plan_kept(start, end, kept, u64::MAX, order)?;
        let mut pieces: Vec<Fetched> = planned
            .groups
            .map(|(first, last)| {
                self.kept
                    .iter()
                    .filter(|speech| (first..=last).contains(&speech.number))
                    .map(|speech| (speech.number, speech.payload()))
                    .collect()
            })
            .unwrap_or_default();
        if order == GroupOrder::Descending {
            pieces.reverse();
        }
        Ok((planned, pieces))
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::message::request_error_code;

    use super::*;

    fn said(text: &str) -> ReadPiece {
        ReadPiece {
            from: vec!["xmoq".into(), "demo".into()],
            text: text.into(),
            start_us: 1,
            end_us: 2,
            at: 3,
        }
    }

    const fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    /// A transcript numbered from 500 that keeps 3, after 5 pieces: 502 to
    /// 504 are kept, 500 and 501 were let go of.
    fn five_kept_three() -> Transcript {
        let mut transcript = Transcript::new(500, 3);
        for text in ["a", "b", "c", "d", "e"] {
            transcript.add(said(text));
        }
        transcript
    }

    #[test]
    fn pieces_are_numbered_from_the_first_and_the_latest_kept() {
        let mut transcript = Transcript::new(500, 3);
        assert_eq!(transcript.largest(), None);
        let numbers: Vec<u64> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|text| transcript.add(said(text)).number)
            .collect();
        assert_eq!(numbers, [500, 501, 502, 503, 504]);
        assert_eq!(transcript.largest(), Some(504));
    }

    #[test]
    fn a_fetch_gets_the_pieces_kept_and_says_which_were_let_go_of() {
        // (case, start, end, order; the numbers sent, the indicator and
        // whether it stands before them, or the error)
        type Case = (
            &'static str,
            Location,
            Location,
            GroupOrder,
            Result<(Vec<u64>, Option<(Location, bool)>), u64>,
        );
        let cases: [Case; 8] = [
            (
                "the pieces kept",
                at(502, 0),
                at(505, 0),
                GroupOrder::Ascending,
                Ok((vec![502, 503, 504], None)),
            ),
            (
                // §10.12.3: a gap would say 500 and 501 do not exist.
                "a range that ends before the latest, over what was let go of",
                at(500, 0),
                at(503, 0),
                GroupOrder::Ascending,
                Ok((vec![502, 503], Some((at(501, 0), true)))),
            ),
            (
                "the same, Descending",
                at(500, 0),
                at(503, 0),
                GroupOrder::Descending,
                Ok((vec![503, 502], Some((at(500, 0), false)))),
            ),
            (
                "what was let go of alone",
                at(500, 0),
                at(501, 0),
                GroupOrder::Ascending,
                Ok((vec![], Some((at(501, 0), true)))),
            ),
            (
                "from before the first piece, Ascending: the indicator covers the start",
                at(0, 0),
                at(505, 0),
                GroupOrder::Ascending,
                Ok((vec![502, 503, 504], Some((at(501, 0), true)))),
            ),
            (
                "before the first piece nothing was",
                at(0, 0),
                at(505, 0),
                GroupOrder::Descending,
                Ok((vec![504, 503, 502], Some((at(500, 0), false)))),
            ),
            (
                "after the latest",
                at(505, 0),
                at(506, 0),
                GroupOrder::Ascending,
                Err(request_error_code::INVALID_RANGE),
            ),
            (
                "before the first piece is read",
                at(0, 0),
                at(1, 0),
                GroupOrder::Ascending,
                Err(request_error_code::INVALID_RANGE),
            ),
        ];
        for (case, start, end, order, expected) in cases {
            let transcript = if case == "before the first piece is read" {
                Transcript::new(500, 3)
            } else {
                five_kept_three()
            };
            let got = transcript
                .fetch(start, end, order)
                .map(|(planned, pieces)| {
                    let numbers: Vec<u64> = pieces.iter().map(|(number, _)| *number).collect();
                    (numbers, planned.left_out)
                });
            assert_eq!(got, expected, "{case}");
        }
    }
}
