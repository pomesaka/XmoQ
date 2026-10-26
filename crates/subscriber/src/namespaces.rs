//! A `SUBSCRIBE_NAMESPACE` kept on a session: when it is asked for again
//! after it ended or was refused, by the rule of `xmoq-again`. The gatherer
//! of the chat, the transcriber and the agent keep theirs so; each sends it
//! and reads its stream itself.

use std::time::{Duration, Instant};

use xmoq_again::AskAgain;
use xmoq_again::next_session::{Growing, Pause};
use xmoq_moqt::name::TrackNamespace;

/// One `SUBSCRIBE_NAMESPACE` for `prefix`, and when it is asked for again.
#[derive(Debug, Clone)]
pub struct KeptNamespace {
    prefix: TrackNamespace,
    again_at: Option<Instant>,
    asked_at: Instant,
    /// The pause after it ended, which grows while it ends again and again
    /// for a role whose [`Pause`] grows.
    ends: Growing,
    /// The pause after it was refused, by the Retry Interval.
    refusals: AskAgain,
}

impl KeptNamespace {
    /// The request for `prefix`, to be asked for at once; after an end it
    /// waits by `pause`, the role's: `Pause::SERVING` for the gatherer and
    /// the transcriber, `Pause::VIEWER` for the agent.
    ///
    /// WHY by the role after an end, and by the Retry Interval after a
    /// refusal: a relay that ends the request every time (one that tells
    /// more namespaces than the session keeps) is asked no more often than
    /// the role's longest pause, and one that refuses says when to come back
    /// (§10.6.2), as for a track (`fixtures/ask-again/same-session.tsv`).
    #[must_use]
    pub fn new(prefix: TrackNamespace, pause: Pause) -> Self {
        Self {
            prefix,
            again_at: Some(Instant::now()),
            asked_at: Instant::now(),
            ends: Growing::new(pause),
            refusals: AskAgain::default(),
        }
    }

    /// The prefix it asks for.
    #[must_use]
    pub fn prefix(&self) -> &TrackNamespace {
        &self.prefix
    }

    /// When it is to be asked for (again); `None` while it is open.
    #[must_use]
    pub fn again_at(&self) -> Option<Instant> {
        self.again_at
    }

    /// Whether its time to be asked for has come at `now`.
    #[must_use]
    pub fn due(&self, now: Instant) -> bool {
        self.again_at.is_some_and(|at| at <= now)
    }

    /// It was asked for at `now`.
    pub fn asked(&mut self, now: Instant) {
        self.again_at = None;
        self.asked_at = now;
    }

    /// It was answered with `REQUEST_OK`: the shortest pause after a refusal
    /// begins anew.
    pub fn subscribed(&mut self) {
        self.refusals.subscribed();
    }

    /// It ended at `now`, or could not be sent: the pause before it is asked
    /// for again, in milliseconds, counted from when it was asked.
    pub fn ended(&mut self, now: Instant) -> u64 {
        let lasted = u64::try_from(now.saturating_duration_since(self.asked_at).as_millis())
            .unwrap_or(u64::MAX);
        let pause_ms = self.ends.after(lasted);
        self.again_at = Some(now + Duration::from_millis(pause_ms));
        pause_ms
    }

    /// It was refused at `now`, with the wait its Retry Interval names
    /// (`RequestError::retry_wait_ms`; `None` for none): the pause before it
    /// is asked for again, in milliseconds.
    pub fn refused(&mut self, named_ms: Option<u64>, now: Instant) -> u64 {
        let pause_ms = self.refusals.after_refusal(named_ms);
        self.again_at = Some(now + Duration::from_millis(pause_ms));
        pause_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What happens to the request, and the pause after it.
    enum Step {
        /// It ended after it lasted this many milliseconds.
        Ended(u64),
        /// It was refused with this wait named.
        Refused(Option<u64>),
        /// It was answered.
        Subscribed,
    }

    #[test]
    fn it_is_asked_again_by_the_roles_pause_after_an_end_and_by_the_retry_interval_after_a_refusal()
    {
        let prefix = TrackNamespace::new(vec![b"xmoq".to_vec()]).unwrap();
        // (case, the role's pause, what happens, the pause after each)
        let cases: [(&str, Pause, Vec<Step>, &[u64]); 5] = [
            (
                "the gatherer's grows while it ends at once",
                Pause::SERVING,
                vec![Step::Ended(0), Step::Ended(0), Step::Ended(0)],
                &[1_000, 2_000, 4_000],
            ),
            (
                "and begins again after it lasted a minute",
                Pause::SERVING,
                vec![Step::Ended(0), Step::Ended(0), Step::Ended(60_000)],
                &[1_000, 2_000, 1_000],
            ),
            (
                "the agent's does not grow",
                Pause::VIEWER,
                vec![Step::Ended(0), Step::Ended(0)],
                &[1_000, 1_000],
            ),
            (
                "a refusal waits its Retry Interval, or the pause of one without",
                Pause::VIEWER,
                vec![Step::Refused(Some(3_000)), Step::Refused(None)],
                &[3_000, 1_000],
            ),
            (
                "refusals that say at once double from 100 ms, and an answer begins them anew",
                Pause::SERVING,
                vec![
                    Step::Refused(Some(0)),
                    Step::Refused(Some(0)),
                    Step::Subscribed,
                    Step::Refused(Some(0)),
                ],
                &[100, 200, 100],
            ),
        ];
        for (case, pause, steps, expected) in cases {
            let mut kept = KeptNamespace::new(prefix.clone(), pause);
            let mut now = Instant::now();
            assert!(kept.due(now), "{case}: asked for at once");
            let mut pauses = Vec::new();
            for step in steps {
                kept.asked(now);
                assert_eq!(kept.again_at(), None, "{case}: open");
                match step {
                    Step::Ended(lasted) => {
                        now += Duration::from_millis(lasted);
                        pauses.push(kept.ended(now));
                    }
                    Step::Refused(named) => pauses.push(kept.refused(named, now)),
                    Step::Subscribed => {
                        kept.subscribed();
                        continue;
                    }
                }
                let pause = *pauses.last().unwrap();
                assert!(!kept.due(now), "{case}: not before its pause");
                now += Duration::from_millis(pause);
                assert!(kept.due(now), "{case}: after it");
            }
            assert_eq!(pauses, expected, "{case}");
        }
    }
}
