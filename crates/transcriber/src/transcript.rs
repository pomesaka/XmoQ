//! The transcript of one prefix: its pieces numbered, and the latest kept
//! for FETCH (decided on 2026-10-25: in memory, none kept on disk; whether
//! and how long to keep them is in `docs/open-questions.md`).

use std::collections::VecDeque;

use xmoq_transcript_tracks::payloads::Speech;

/// The pieces of one transcript.
#[derive(Debug)]
pub struct Transcript {
    next: u64,
    kept: VecDeque<Speech>,
    keep: usize,
}

/// A piece as it was read, before it is numbered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
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
            next: first,
            kept: VecDeque::new(),
            keep,
        }
    }

    /// Numbers a piece and keeps it, letting go of the oldest past
    /// [`Self::new`]'s `keep`.
    pub fn add(&mut self, said: Said) -> Speech {
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

    /// How many pieces are kept: the most a FETCH gets.
    #[must_use]
    pub fn kept(&self) -> u64 {
        u64::try_from(self.kept.len()).unwrap_or(u64::MAX)
    }

    /// The pieces kept from `first` to `last`, both included, in order.
    #[must_use]
    pub fn between(&self, first: u64, last: u64) -> Vec<&Speech> {
        self.kept
            .iter()
            .filter(|speech| (first..=last).contains(&speech.number))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(text: &str) -> Said {
        Said {
            from: vec!["xmoq".into(), "demo".into()],
            text: text.into(),
            start_us: 1,
            end_us: 2,
            at: 3,
        }
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
        assert_eq!(transcript.kept(), 3);
        let texts: Vec<&str> = transcript
            .between(0, 503)
            .iter()
            .map(|speech| speech.text.as_str())
            .collect();
        assert_eq!(texts, ["c", "d"]);
    }
}
