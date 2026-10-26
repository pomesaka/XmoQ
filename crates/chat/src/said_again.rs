//! Whether, and after how long, the gatherer asks again for a track of a
//! participant (`said`, `reacted`) whose subscription ended or was refused
//! while its namespace stays announced: [`Settings::keeping`] and
//! [`Settings::tries`], by which `kept::decide` of `xmoq-subscriber` decides,
//! as it does for a track a viewer keeps (`fixtures/kept-track/`).
//!
//! WHY the decision of `xmoq-subscriber` alone, and not its `KeptTrack`: a
//! `KeptTrack` is read by its caller's loop, one future for each track,
//! which suits the few tracks of a viewer; the gatherer follows up to
//! [`crate::gather::MAX_PEOPLE`] participants with two tracks each, whose
//! streams are read by a task each that reports to one queue (`OwnRequests`
//! of `xmoq-publisher`).
//!
//! The pauses are the rule of every client (`xmoq-again`), with the pause
//! after an end set by `--ask-again-ms`. WHY asked again after a refusal
//! with a Retry Interval of 0 ("SHOULD NOT be retried", §10.6.2), as the
//! terminal viewer and the watching page do: the namespace that is still
//! announced says the participant is there, and a relay that gives every
//! refusal a 0 (moqtail's 0.15.0 does) would otherwise have the gatherer
//! leave its participants for good. WHY a `reacted` that does not exist is
//! not asked again (decided on 2026-10-26): a participant need not publish
//! it, and a viewer or an agent that publishes none would be asked
//! `--ask-again-times` times each, as many requests as the room has people,
//! for nothing.
//!
//! WHY a number of times, after which a participant is asked again only
//! when its NAMESPACE comes again: the gatherer does not see a
//! `NAMESPACE_DONE` that comes while its `SUBSCRIBE_NAMESPACE` is being
//! asked for again (`crates/chat/README.md`), and would ask a participant
//! that has gone for as long as the session lasts. A subscription that is
//! established counts as the participant being there, and the count begins
//! again.
//!
//! WHY NOT FETCH what was said meanwhile (decided on 2026-10-24): the lines
//! after the new subscription alone are taken, as after a session that
//! connected anew.

use xmoq_again::Pauses;
use xmoq_chat_tracks::names::PersonTrack;
use xmoq_subscriber::kept::{Keeping, Tries};

/// `--ask-again-times` by default, and its most: the transcriber's are the
/// same.
pub use xmoq_subscriber::kept::{TIMES, TIMES_MOST};

/// The pause after an end, and after a refusal that names no time, by
/// default: the rule's.
pub const PAUSE_MS: u64 = xmoq_again::ASK_AGAIN_MS;

/// The least `--ask-again-ms`: as short as the rule's shortest pause after
/// a refusal, so that a relay that ends every subscription at once is not
/// asked more often than that.
pub const PAUSE_MS_LEAST: u64 = xmoq_again::SHORTEST_MS;

/// The most `--ask-again-ms`.
pub const PAUSE_MS_MOST: u64 = 60_000;

/// What the flags set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// `--ask-again-times`.
    pub times: u32,
    /// `--ask-again-ms`.
    pub pause_ms: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            times: TIMES,
            pause_ms: PAUSE_MS,
        }
    }
}

impl Settings {
    /// How `track` of a participant is kept.
    #[must_use]
    pub fn keeping(self, track: PersonTrack) -> Keeping {
        Keeping {
            again_after_failure: true,
            times: Some(self.times),
            optional: track == PersonTrack::Reacted,
        }
    }

    /// What is kept of a track of a participant that is followed anew, or
    /// whose NAMESPACE came again.
    #[must_use]
    pub fn tries(self) -> Tries {
        Tries::new(Pauses {
            after_end_ms: self.pause_ms,
            ..Pauses::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use xmoq_subscriber::kept::{Becomes, Happened, decide};

    use super::*;

    #[test]
    fn the_flags_set_the_times_and_the_pause_after_an_end_and_reacted_alone_is_optional() {
        let refused = |error_code, named_ms| Happened::Refused {
            error_code,
            named_ms,
        };
        let not_there = xmoq_moqt::message::request_error_code::DOES_NOT_EXIST;
        let cases = [
            (
                "said, by default",
                Settings::default(),
                PersonTrack::Said,
                vec![Happened::Ended, refused(not_there, Some(1001))],
                vec![Becomes::Again(1000), Becomes::Again(1001)],
            ),
            (
                "reacted that does not exist",
                Settings::default(),
                PersonTrack::Reacted,
                vec![refused(not_there, Some(1001))],
                vec![Becomes::GivenUp],
            ),
            (
                "--ask-again-ms 250 --ask-again-times 1",
                Settings {
                    times: 1,
                    pause_ms: 250,
                },
                PersonTrack::Said,
                vec![Happened::NotSent, Happened::Ended],
                vec![Becomes::Again(250), Becomes::GivenUp],
            ),
        ];
        for (case, settings, track, happened, expected) in cases {
            let mut tries = settings.tries();
            let got: Vec<Becomes> = happened
                .into_iter()
                .map(|happened| decide(happened, settings.keeping(track), &mut tries))
                .collect();
            assert_eq!(got, expected, "{case}");
        }
    }
}
