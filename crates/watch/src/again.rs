//! When a track that ended, or was refused, is asked for again on the same
//! session: the sound, whose subscription a relay can end while the video
//! goes on.
//!
//! The rule and the numbers are those of the watching page
//! (`stayOnSession` in `packages/media/src/core/keep-session.ts`, which
//! `keepTrackOnSession` uses for its sound), so that the two viewers ask a
//! relay as often. All of them are provisional there.

/// The pause after a subscription ended, and after a refusal that names
/// no time (`DEFAULT_ASK_AGAIN_DELAY_MS`).
pub const ASK_AGAIN_MS: u64 = 1000;
/// The shortest pause after a refusal that names one
/// (`MIN_ASK_AGAIN_DELAY_MS`). It doubles with every such refusal that
/// follows another without a subscription in between, up to
/// [`MOST_SHORTEST_MS`].
///
/// WHY any: a relay that says "at once" and refuses at once would be asked
/// in a loop.
pub const SHORTEST_MS: u64 = 100;
/// What [`SHORTEST_MS`] grows to (`MAX_MIN_ASK_AGAIN_DELAY_MS`).
pub const MOST_SHORTEST_MS: u64 = 5000;
/// The longest pause a refusal is let name (`MAX_NAMED_ASK_AGAIN_DELAY_MS`):
/// a viewer told to come back much later has in effect been told to go.
pub const MOST_NAMED_MS: u64 = 30_000;

/// How long to wait before the next SUBSCRIBE for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAgain {
    shortest_ms: u64,
}

impl Default for AskAgain {
    fn default() -> Self {
        Self {
            shortest_ms: SHORTEST_MS,
        }
    }
}

impl AskAgain {
    /// The track plays again.
    pub fn subscribed(&mut self) {
        self.shortest_ms = SHORTEST_MS;
    }

    /// Its subscription ended while the session goes on: the pause.
    #[must_use]
    pub fn after_end(&self) -> u64 {
        ASK_AGAIN_MS
    }

    /// It was refused, with the wait its Retry Interval names
    /// (`RequestError::retry_wait_ms`; `None` for none): the pause.
    pub fn after_refusal(&mut self, named_ms: Option<u64>) -> u64 {
        let Some(named_ms) = named_ms else {
            return ASK_AGAIN_MS;
        };
        let ms = named_ms.max(self.shortest_ms).min(MOST_NAMED_MS);
        self.shortest_ms = self.shortest_ms.saturating_mul(2).min(MOST_SHORTEST_MS);
        ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What happens to the track, in order: `Some(named)` a refusal,
    /// `None` an end; `Subscribed` resets. Each with the pause expected.
    enum Step {
        Ended(u64),
        Refused(Option<u64>, u64),
        Subscribed,
    }

    #[test]
    fn pauses_follow_the_rule_of_the_watching_page() {
        use Step::{Ended, Refused, Subscribed};
        let cases: [(&str, Vec<Step>); 6] = [
            ("an end", vec![Ended(1000)]),
            ("a refusal without a time", vec![Refused(None, 1000)]),
            ("a refusal that names 3 s", vec![Refused(Some(3000), 3000)]),
            (
                "refusals that say at once: 100, 200, 400 ms",
                vec![
                    Refused(Some(0), 100),
                    Refused(Some(0), 200),
                    Refused(Some(0), 400),
                ],
            ),
            (
                "the shortest grows to 5 s and no further, and a subscription resets it",
                vec![
                    Refused(Some(0), 100),
                    Refused(Some(0), 200),
                    Refused(Some(0), 400),
                    Refused(Some(0), 800),
                    Refused(Some(0), 1600),
                    Refused(Some(0), 3200),
                    Refused(Some(0), 5000),
                    Refused(Some(0), 5000),
                    Subscribed,
                    Refused(Some(0), 100),
                ],
            ),
            (
                "a refusal that names a minute: 30 s",
                vec![Refused(Some(60_000), 30_000)],
            ),
        ];
        for (name, steps) in cases {
            let mut again = AskAgain::default();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Ended(ms) => assert_eq!(again.after_end(), ms, "{name}: step {index}"),
                    Refused(named, ms) => {
                        assert_eq!(again.after_refusal(named), ms, "{name}: step {index}");
                    }
                    Subscribed => again.subscribed(),
                }
            }
        }
    }
}
