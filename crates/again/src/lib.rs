//! When a track that ended, or was refused, is asked for again on the same
//! session, whose subscription a relay can end while the session goes on:
//! by the terminal viewer (`xmoq-watch`), the agent (`xmoq-agent`), the
//! part they share (`xmoq-viewer-io`), the gatherer (`xmoq-chat`) and the
//! transcriber (`xmoq-transcriber`).
//!
//! The rule and the default values are those of the pages
//! (`createAskAgain` in `packages/moqt/src/core/again.ts`), so that every
//! client asks a relay as often; both are tested with the rows of
//! `fixtures/ask-again/same-session.tsv`. All of them are provisional.
//!
//! WHY a crate of its own: the terminal viewer and the gatherer both ask
//! by it, and neither may depend on the other (the viewer builds
//! `openh264` from C++, the gatherer SQLite from C). WHY NOT in
//! `xmoq-moqt`: how often an application asks is its policy and not the
//! protocol's, and the protocol libraries hand a Retry Interval on without
//! waiting by themselves. The pages have it in `@xmoq/moqt` only because
//! their packages of layer 0 cannot depend on each other (the top of
//! `again.ts` there). WHY NOT a copy in each client: the rule would be in
//! two places, and the two would drift.

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

    /// A number of the table, or `None` for `-`.
    fn value_of(field: &str) -> Option<u64> {
        (field != "-").then(|| field.parse().unwrap())
    }

    /// `fixtures/ask-again/same-session.tsv`, which `createAskAgain` of
    /// `@xmoq/moqt` is tested with too.
    #[test]
    fn pauses_follow_the_shared_table() {
        let table = include_str!("../../../fixtures/ask-again/same-session.tsv");
        let mut rows = 0;
        for row in table.lines() {
            let [name, after_end, shortest, most_shortest, most_named, steps] =
                row.split('\t').collect::<Vec<_>>()[..]
            else {
                panic!("{row}");
            };
            let default = Pauses::default();
            let mut again = AskAgain::new(Pauses {
                after_end_ms: value_of(after_end).unwrap_or(default.after_end_ms),
                shortest_ms: value_of(shortest).unwrap_or(default.shortest_ms),
                most_shortest_ms: value_of(most_shortest).unwrap_or(default.most_shortest_ms),
                most_named_ms: value_of(most_named).unwrap_or(default.most_named_ms),
            });
            for step in steps.split(' ') {
                if step == "subscribed" {
                    again.subscribed();
                    continue;
                }
                let (what, ms) = step.split_once('=').unwrap();
                let pause = match what.strip_prefix("refused:") {
                    Some(named) => again.after_refusal(value_of(named)),
                    None if what == "ended" => again.after_end(),
                    None => panic!("{name}: no step {step}"),
                };
                assert_eq!(pause, ms.parse::<u64>().unwrap(), "{name}: {step}");
            }
            rows += 1;
        }
        // WHY not a count of rows: a row that is added would have it changed in
        // both implementations; a table that is not read is what this guards against.
        assert!(rows > 0);
    }
}
