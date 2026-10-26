//! How long a client that serves tracks waits before it asks again: for a
//! session after one ended ([`NextTry`]), and for its `SUBSCRIBE_NAMESPACE`
//! after the request ended while the session goes on. The gatherer of the
//! chat and the transcriber wait by it.
//!
//! WHY a wait that grows: a relay that ends either at once every time (one
//! that tells more namespaces than the client keeps, one that refuses it)
//! would otherwise have it ask again every second for good. WHY it comes
//! back to the first wait: after a session or a request that went well for
//! a while, the next end is one of its own and not the same one again.

use std::time::Duration;

use xmoq_again::next_session::{Ended, NextSession};

/// The first wait. **Provisional.**
pub const FIRST: Duration = Duration::from_secs(1);

/// The longest wait. **Provisional**: long against a relay that is being
/// started again, short against a person waiting for the chat.
pub const MOST: Duration = Duration::from_secs(30);

/// How long a session or a request has to have lasted for the wait to
/// begin again from [`FIRST`]. **Provisional.**
pub const WENT_WELL: Duration = Duration::from_secs(60);

/// The wait before the next try.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: FIRST }
    }
}

impl Backoff {
    /// The wait after something that lasted `lasted` ended: [`FIRST`] after
    /// one that lasted [`WENT_WELL`] or more, else twice the wait before, up
    /// to [`MOST`].
    pub fn after(&mut self, lasted: Duration) -> Duration {
        if lasted >= WENT_WELL {
            self.next = FIRST;
        }
        let wait = self.next;
        self.next = (wait * 2).min(MOST);
        wait
    }
}

/// Where and when a client that serves tracks tries the next session:
/// where, and at once after a GOAWAY, by the rule every client goes by
/// (`NextSession` of `xmoq-again`, §10.4: "The client MUST use this URI for
/// the new session if provided"); after any other end, after a wait that
/// grows ([`Backoff`]).
///
/// WHY NOT the viewers' wait of 1 s from the try before: nobody watches
/// this client, and a relay that ends every session at once would have it
/// connect every second for good.
#[derive(Debug, Clone)]
pub struct NextTry {
    rule: NextSession,
    backoff: Backoff,
}

impl Default for NextTry {
    fn default() -> Self {
        // No wait of the rule's own: the wait is the Backoff's. The client
        // follows nothing that could end it, so every end is followed by a
        // try.
        Self {
            rule: NextSession::new(0, true, false),
            backoff: Backoff::default(),
        }
    }
}

impl NextTry {
    /// Where the first session goes, for a client told to go to `url`.
    pub fn first(&mut self, url: &str) -> String {
        self.rule.next(url, 0).url
    }

    /// Where the next session goes, and how long to wait first, after one
    /// that lasted `lasted` (from its try) ended so.
    pub fn after(&mut self, url: &str, ended: Ended, lasted: Duration) -> (String, Duration) {
        self.rule.ended(ended);
        let next = self.rule.next(url, 0);
        let wait = if next.at_once {
            Duration::ZERO
        } else {
            self.backoff.after(lasted)
        };
        (next.url, wait)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of the table of [`NextTry`]: its name, each end and how long
    /// its session lasted in seconds, and the URL and the wait in seconds
    /// after each.
    type Case = (
        &'static str,
        Vec<(Ended, u64)>,
        &'static [(&'static str, u64)],
    );

    #[test]
    fn the_wait_doubles_up_to_its_most_and_begins_again_after_a_long_run() {
        let seconds = Duration::from_secs;
        // (case, how long each lasted, the waits after each, in seconds)
        let cases: [(&str, &[u64], &[u64]); 4] = [
            ("one at once", &[0], &[1]),
            (
                "again and again at once",
                &[0, 0, 0, 0, 0, 0, 0, 0],
                &[1, 2, 4, 8, 16, 30, 30, 30],
            ),
            (
                "one that went well between",
                &[0, 0, 0, 60, 0],
                &[1, 2, 4, 1, 2],
            ),
            ("one short of going well", &[0, 0, 59], &[1, 2, 4]),
        ];
        for (case, lasted, expected) in cases {
            let mut backoff = Backoff::default();
            let waits: Vec<u64> = lasted
                .iter()
                .map(|lasted| backoff.after(seconds(*lasted)).as_secs())
                .collect();
            assert_eq!(waits, expected, "{case}");
        }
    }

    #[test]
    fn the_next_session_goes_where_a_goaway_says_at_once_once_and_else_after_the_wait() {
        let moved = |to: &str| Ended::Moved(Some(to.to_string()));
        let failed = Ended::Failed { began: true };
        let cases: [Case; 5] = [
            (
                "a GOAWAY with a URI, then failures there",
                vec![
                    (moved("https://b/"), 5),
                    (failed.clone(), 0),
                    (failed.clone(), 0),
                ],
                &[("https://b/", 0), ("https://b/", 1), ("https://b/", 2)],
            ),
            (
                "a GOAWAY without one",
                vec![(Ended::Moved(None), 5)],
                &[("https://a/", 0)],
            ),
            (
                "GOAWAYs in a row: at once once, then after the wait",
                vec![
                    (moved("https://b/"), 0),
                    (moved("https://c/"), 0),
                    (moved("https://d/"), 0),
                ],
                &[("https://b/", 0), ("https://c/", 1), ("https://d/", 0)],
            ),
            (
                "a failure, then a GOAWAY",
                vec![(failed.clone(), 0), (moved("https://b/"), 0)],
                &[("https://a/", 1), ("https://b/", 0)],
            ),
            (
                "a failure after a session that went well",
                vec![(failed.clone(), 0), (failed.clone(), 60)],
                &[("https://a/", 1), ("https://a/", 1)],
            ),
        ];
        for (case, ends, expected) in cases {
            let mut next = NextTry::default();
            assert_eq!(next.first("https://a/"), "https://a/", "{case}");
            let seen: Vec<(String, u64)> = ends
                .into_iter()
                .map(|(ended, lasted)| {
                    let (url, wait) = next.after("https://a/", ended, Duration::from_secs(lasted));
                    (url, wait.as_secs())
                })
                .collect();
            let expected: Vec<(String, u64)> = expected
                .iter()
                .map(|(url, wait)| ((*url).to_string(), *wait))
                .collect();
            assert_eq!(seen, expected, "{case}");
        }
    }
}
