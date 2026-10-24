//! When a track that ended, or was refused, is asked for again on the same
//! session: the terminal viewer's sound and chat (`xmoq-watch`), whose
//! subscription a relay can end while the session goes on, and the
//! gatherer's subscription to a participant (`xmoq-chat`).
//!
//! The rule and the default values are those of the watching page
//! (`stayOnSession` in `packages/media/src/core/keep-session.ts`, which
//! `keepTrackOnSession` uses for its sound), so that every client asks a
//! relay as often. All of them are provisional there.
//!
//! WHY a crate of its own: the terminal viewer and the gatherer both ask
//! by it, and neither may depend on the other (the viewer builds
//! `openh264` from C++, the gatherer SQLite from C). WHY NOT in
//! `xmoq-moqt`: how often an application asks is its policy and not the
//! protocol's, and the protocol libraries hand a Retry Interval on without
//! waiting by themselves (as `@xmoq/moqt` does). WHY NOT a copy in each
//! client: the rule would be in two places, and the two would drift.

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
/// a client told to come back much later has in effect been told to go.
pub const MOST_NAMED_MS: u64 = 30_000;

/// The pauses of the rule, in milliseconds. The default is the page's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pauses {
    /// After an end, and after a refusal that names no time.
    pub after_end_ms: u64,
    /// What the shortest pause after a refusal that names a time begins at.
    pub shortest_ms: u64,
    /// What it grows to.
    pub most_shortest_ms: u64,
    /// The longest a refusal is let name.
    pub most_named_ms: u64,
}

impl Default for Pauses {
    fn default() -> Self {
        Self {
            after_end_ms: ASK_AGAIN_MS,
            shortest_ms: SHORTEST_MS,
            most_shortest_ms: MOST_SHORTEST_MS,
            most_named_ms: MOST_NAMED_MS,
        }
    }
}

/// How long to wait before the next SUBSCRIBE for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAgain {
    pauses: Pauses,
    shortest_ms: u64,
}

impl Default for AskAgain {
    fn default() -> Self {
        Self::new(Pauses::default())
    }
}

impl AskAgain {
    /// The rule with these pauses.
    #[must_use]
    pub const fn new(pauses: Pauses) -> Self {
        Self {
            pauses,
            shortest_ms: pauses.shortest_ms,
        }
    }

    /// The track is subscribed to again.
    pub const fn subscribed(&mut self) {
        self.shortest_ms = self.pauses.shortest_ms;
    }

    /// Its subscription ended while the session goes on: the pause.
    #[must_use]
    pub const fn after_end(&self) -> u64 {
        self.pauses.after_end_ms
    }

    /// It was refused, with the wait its Retry Interval names
    /// (`RequestError::retry_wait_ms` of `xmoq-moqt`; `None` for none): the
    /// pause.
    pub fn after_refusal(&mut self, named_ms: Option<u64>) -> u64 {
        let Some(named_ms) = named_ms else {
            return self.pauses.after_end_ms;
        };
        let ms = named_ms
            .max(self.shortest_ms)
            .min(self.pauses.most_named_ms);
        self.shortest_ms = self
            .shortest_ms
            .saturating_mul(2)
            .min(self.pauses.most_shortest_ms);
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
        let cases: [(&str, Pauses, Vec<Step>); 8] = [
            ("an end", Pauses::default(), vec![Ended(1000)]),
            (
                "a refusal without a time",
                Pauses::default(),
                vec![Refused(None, 1000)],
            ),
            (
                "a refusal that names 3 s",
                Pauses::default(),
                vec![Refused(Some(3000), 3000)],
            ),
            (
                "refusals that say at once: 100, 200, 400 ms",
                Pauses::default(),
                vec![
                    Refused(Some(0), 100),
                    Refused(Some(0), 200),
                    Refused(Some(0), 400),
                ],
            ),
            (
                "the shortest grows to 5 s and no further, and a subscription resets it",
                Pauses::default(),
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
                Pauses::default(),
                vec![Refused(Some(60_000), 30_000)],
            ),
            (
                "another pause after an end is the pause after a refusal without a time too",
                Pauses {
                    after_end_ms: 2500,
                    ..Pauses::default()
                },
                vec![Ended(2500), Refused(None, 2500), Refused(Some(0), 100)],
            ),
            (
                "other bounds of the named pause",
                Pauses {
                    after_end_ms: 1000,
                    shortest_ms: 300,
                    most_shortest_ms: 500,
                    most_named_ms: 4000,
                },
                vec![
                    Refused(Some(0), 300),
                    Refused(Some(0), 500),
                    Refused(Some(0), 500),
                    Refused(Some(9000), 4000),
                    Subscribed,
                    Refused(Some(0), 300),
                ],
            ),
        ];
        for (name, pauses, steps) in cases {
            let mut again = AskAgain::new(pauses);
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
