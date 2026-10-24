//! Whether, and after how long, the gatherer asks again for the `said` of a
//! participant whose subscription ended or was refused while its namespace
//! stays announced.
//!
//! The pauses are the rule of every client (`xmoq-again`), with the pause
//! after an end set by `--ask-again-ms`. WHY asked again after a refusal
//! with a Retry Interval of 0 ("SHOULD NOT be retried", §10.6.2), as the
//! terminal viewer and the watching page do: the namespace that is still
//! announced says the participant is there, and a relay that gives every
//! refusal a 0 (moqtail's 0.15.0 does) would otherwise have the gatherer
//! leave its participants for good. WHY NOT after a track that is
//! malformed: §2.4.2 has the subscriber cancel it, and the same track would
//! be malformed again.
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

use xmoq_again::{AskAgain, Pauses};

/// How many times in a row a participant is asked again by default,
/// without a subscription that was established between. **Provisional**:
/// against the own relay, which refuses a `said` nobody publishes with
/// `DOES_NOT_EXIST` and a Retry Interval of 1,001, about two minutes.
pub const TIMES: u32 = 30;

/// The most `--ask-again-times`. 0 is the least: never asked again.
pub const TIMES_MOST: u32 = 10_000;

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

/// How a subscription to a participant's `said` came to an end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Over {
    /// It ended (`PUBLISH_DONE`, its stream), was not answered in time, or
    /// could not be sent.
    Ended,
    /// It was refused, with the wait its Retry Interval names
    /// (`RequestError::retry_wait_ms`; `None` for none).
    Refused(Option<u64>),
    /// It was cancelled for a malformed track.
    Malformed,
}

/// When one participant is asked again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaidAgain {
    rule: AskAgain,
    asked: u32,
    times: u32,
}

impl SaidAgain {
    /// A participant that is followed anew, or whose NAMESPACE came again.
    #[must_use]
    pub fn new(settings: Settings) -> Self {
        Self {
            rule: AskAgain::new(Pauses {
                after_end_ms: settings.pause_ms,
                ..Pauses::default()
            }),
            asked: 0,
            times: settings.times,
        }
    }

    /// Its subscription was established.
    pub fn subscribed(&mut self) {
        self.rule.subscribed();
        self.asked = 0;
    }

    /// Its subscription is over: the pause in milliseconds before it is
    /// asked again, or `None` where it is not, until its NAMESPACE comes
    /// again.
    pub fn over(&mut self, over: Over) -> Option<u64> {
        if self.asked >= self.times {
            return None;
        }
        let pause = match over {
            Over::Ended => self.rule.after_end(),
            Over::Refused(named) => self.rule.after_refusal(named),
            Over::Malformed => return None,
        };
        self.asked += 1;
        Some(pause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What happens to the subscription, in order, with the pause expected.
    enum Step {
        Over(Over, Option<u64>),
        Subscribed,
    }

    #[test]
    fn a_participant_is_asked_again_by_the_rule_up_to_the_times_set() {
        use Step::Subscribed;
        let ended = |pause| Step::Over(Over::Ended, pause);
        let refused = |named, pause| Step::Over(Over::Refused(named), pause);
        let settings = |times, pause_ms| Settings { times, pause_ms };
        let cases: [(&str, Settings, Vec<Step>); 8] = [
            ("an end", Settings::default(), vec![ended(Some(1000))]),
            (
                "a refusal that says do not retry: after the pause of an end",
                Settings::default(),
                vec![refused(None, Some(1000))],
            ),
            (
                "the own relay's DOES_NOT_EXIST",
                Settings::default(),
                vec![refused(Some(1000), Some(1000))],
            ),
            (
                "a malformed track is not asked again",
                Settings::default(),
                vec![Step::Over(Over::Malformed, None)],
            ),
            (
                "two times, then not again",
                settings(2, 1000),
                vec![ended(Some(1000)), ended(Some(1000)), ended(None)],
            ),
            (
                "an established subscription counts again from none",
                settings(2, 1000),
                vec![
                    ended(Some(1000)),
                    ended(Some(1000)),
                    Subscribed,
                    ended(Some(1000)),
                    ended(Some(1000)),
                    ended(None),
                ],
            ),
            (
                "no times: never asked again",
                settings(0, 1000),
                vec![ended(None)],
            ),
            (
                "another pause after an end and after a refusal without a time, not after one that names a time",
                settings(30, 250),
                vec![
                    ended(Some(250)),
                    refused(None, Some(250)),
                    refused(Some(0), Some(100)),
                ],
            ),
        ];
        for (name, settings, steps) in cases {
            let mut again = SaidAgain::new(settings);
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Step::Over(over, pause) => {
                        assert_eq!(again.over(over), pause, "{name}: step {index}");
                    }
                    Subscribed => again.subscribed(),
                }
            }
        }
    }
}
