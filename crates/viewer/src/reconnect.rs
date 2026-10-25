//! When and where a viewer connects again after a session ended: the rule
//! `xmoq-watch` and `xmoq-agent` both go by. Time comes in as milliseconds
//! from the caller's clock.
//!
//! The rows of `fixtures/ask-again/next-session.tsv` are read in the test;
//! `createNextSession` of `@xmoq/moqt` goes by the same rule and table: the
//! wait counts from the try before, not from the end of the session.

/// How long a room gets for each of its answers (a token, a seat), by
/// default; `--room-wait-ms` of both programs. **Provisional.** WHY a flag:
/// a room can stand far from the viewer.
pub const ROOM_WAIT_MS: u64 = 5_000;

/// How long a relay gets to be connected to, by default; `--connect-wait-ms`
/// of both programs. **Provisional.**
pub const CONNECT_WAIT_MS: u64 = 10_000;

/// The least time from one try at a session to the next, however often a
/// room names another place ([`NextSession`]). **Provisional.**
pub const BETWEEN_TRIES_MS: u64 = 1_000;

/// How a session ended, as the rule needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// The relay said GOAWAY, with the URL to go to, or none for the same.
    Moved(Option<String>),
    /// The broadcast ended; a room says what comes next.
    Again,
    /// The session could not be made, or broke. `began`: something was
    /// played or read on it.
    Failed {
        /// Whether anything came of the session.
        began: bool,
    },
    /// The viewer is to end.
    Quit,
}

/// What the caller does after a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum After {
    /// Connects again ([`NextSession::next`]).
    Connect,
    /// Ends.
    End,
    /// Ends with the session's error.
    EndWithError,
}

/// Where the next session goes, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Next {
    /// The URL to connect to.
    pub url: String,
    /// How long to wait first, in milliseconds.
    pub wait_ms: u64,
}

/// How the last try was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Try {
    /// After the wait, or the first.
    Waited,
    /// At once, after a GOAWAY.
    AtOnce,
}

/// The state of the rule from one session to the next.
#[derive(Debug, Clone)]
pub struct NextSession {
    between_ms: u64,
    follows: bool,
    /// The last GOAWAY: the URL the viewer was told to leave, and where to
    /// go instead. It holds for every try while the place is still the one
    /// left. WHY not for the next try alone: the relay that sent it is going
    /// away and refuses what comes (`GOING_AWAY`); a viewer that went back
    /// to it after one failure at the new URL was left there (`keepPlaying`
    /// of `@xmoq/media`, an interop run, 2026-10-24). WHY not where the
    /// place is another: a room knows where the broadcast went.
    moved: Option<(String, String)>,
    just_moved: bool,
    last_try: Try,
    tried_at_ms: Option<u64>,
    /// The URL of the session that ends next.
    connected: Option<String>,
    /// Whether a failure ends the viewer though it follows a room: the
    /// first try's, where it was asked for, and no later one's.
    failure_ends: bool,
}

impl NextSession {
    /// The rule for a viewer that waits `between_ms` from one try to the
    /// next, and that `follows` a room: one whose room says what comes next
    /// connects again after a broadcast ended or broke; one without ends.
    #[must_use]
    /// `first_failure_ends`: a first try that fails before anything came of
    /// it ends the viewer, though it follows a room.
    pub fn new(between_ms: u64, follows: bool, first_failure_ends: bool) -> Self {
        Self {
            between_ms,
            follows,
            moved: None,
            just_moved: false,
            last_try: Try::Waited,
            tried_at_ms: None,
            connected: None,
            failure_ends: first_failure_ends,
        }
    }

    /// Where to connect for a place at `url` (as the room or the flags give
    /// it), and how long to wait first, at `now_ms`.
    ///
    /// §10.4: "The client MUST use this URI for the new session if
    /// provided." WHY not at once again: a room that names one broadcast
    /// after the other, or one that is gone, would have the viewer connect
    /// as fast as it can. WHY at once after a GOAWAY, and only once in a
    /// row: the picture stands still until the next session plays, and a
    /// relay that sends GOAWAY on every session would have it connect as
    /// fast as it can. `createNextSession` of `@xmoq/moqt` does the same.
    pub fn next(&mut self, url: &str, now_ms: u64) -> Next {
        let url = match &self.moved {
            Some((left, to)) if left == url => to.clone(),
            Some(_) => {
                self.moved = None;
                url.to_string()
            }
            None => url.to_string(),
        };
        let at_once = self.just_moved && self.last_try == Try::Waited;
        self.just_moved = false;
        self.last_try = if at_once { Try::AtOnce } else { Try::Waited };
        let wait_ms = if at_once {
            0
        } else {
            self.tried_at_ms.map_or(0, |tried| {
                self.between_ms.saturating_sub(now_ms.saturating_sub(tried))
            })
        };
        self.tried_at_ms = Some(now_ms.saturating_add(wait_ms));
        self.connected = Some(url.clone());
        Next { url, wait_ms }
    }

    /// The session [`Self::next`] gave ended so.
    ///
    /// WHY a first try that failed before anything came ends the viewer,
    /// and a later one does not: a relay that turns the viewer down is to
    /// be seen, and a broadcast that broke off may come back.
    pub fn ended(&mut self, end: Ended) -> After {
        let failure_ends = std::mem::replace(&mut self.failure_ends, false);
        match end {
            Ended::Moved(to) => {
                debug_assert!(self.connected.is_some(), "ended before next");
                let connected = self.connected.clone().unwrap_or_default();
                let left = self
                    .moved
                    .as_ref()
                    .map_or_else(|| connected.clone(), |(left, _)| left.clone());
                self.moved = Some((left, to.unwrap_or(connected)));
                self.just_moved = true;
                After::Connect
            }
            Ended::Again if self.follows => After::Connect,
            Ended::Again | Ended::Quit => After::End,
            Ended::Failed { began } if !self.follows || (failure_ends && !began) => {
                After::EndWithError
            }
            Ended::Failed { .. } => After::Connect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The third column of a row of `next-session.tsv`: each end, how long
    /// its session lasted from its try (`@N`, 0 without it), and the wait it
    /// is to be followed by.
    fn waits_of(ends: &str) -> Vec<(Ended, u64, u64)> {
        ends.split(' ')
            .map(|step| {
                let (end, wait) = step.split_once('=').expect("end=wait");
                let (end, lasted) = end.split_once('@').unwrap_or((end, "0"));
                let end = match end {
                    "moved" => Ended::Moved(None),
                    "ended" => Ended::Again,
                    "failed" => Ended::Failed { began: true },
                    other => panic!("an end the test does not know: {other}"),
                };
                (
                    end,
                    lasted.parse().expect("how long it lasted"),
                    wait.parse().expect("a wait"),
                )
            })
            .collect()
    }

    #[test]
    fn the_waits_are_as_the_shared_table_says() {
        let path = format!(
            "{}/../../fixtures/ask-again/next-session.tsv",
            env!("CARGO_MANIFEST_DIR")
        );
        let table =
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let rows: Vec<Vec<&str>> = table.lines().map(|row| row.split('\t').collect()).collect();
        // As many as the table has: a row that is added there and not read
        // here is counted, as `@xmoq/moqt` counts them.
        assert_eq!(rows.len(), 6, "{path}");
        assert!(
            table.ends_with('\n'),
            "{path}: its last row ends with a line feed"
        );
        for row in rows {
            let [name, between, ends] = row.as_slice() else {
                panic!("a row of three columns: {row:?}");
            };
            let between: u64 = between.parse().expect("the usual wait");
            // A room is followed: no end ends the viewer.
            let mut rule = NextSession::new(between, true, false);
            // The first try at 0; then for each end, the session lasts as
            // the row says from its try, ends, and the next try waits.
            let mut now = 0;
            rule.next("https://a/", now);
            for (step, (end, lasted, wait)) in waits_of(ends).into_iter().enumerate() {
                now += lasted;
                assert_eq!(rule.ended(end), After::Connect, "{name}, step {step}");
                let next = rule.next("https://a/", now);
                assert_eq!(next.wait_ms, wait, "{name}, step {step}");
                now += next.wait_ms;
            }
        }
    }

    #[test]
    fn a_goaway_names_where_the_next_sessions_go() {
        let mut rule = NextSession::new(1000, true, true);
        assert_eq!(rule.next("https://a/", 0).url, "https://a/");
        rule.ended(Ended::Moved(Some("https://b/".to_string())));
        assert_eq!(rule.next("https://a/", 10).url, "https://b/");
        // It holds while the place is the one left, also after a failure.
        rule.ended(Ended::Failed { began: true });
        assert_eq!(rule.next("https://a/", 2000).url, "https://b/");
        // Another GOAWAY, without a URL: the same place it came from.
        rule.ended(Ended::Moved(None));
        assert_eq!(rule.next("https://a/", 3000).url, "https://b/");
        // A place that is another forgets it.
        rule.ended(Ended::Again);
        assert_eq!(rule.next("https://c/", 5000).url, "https://c/");
        rule.ended(Ended::Again);
        assert_eq!(rule.next("https://a/", 7000).url, "https://a/");
    }

    #[test]
    fn what_ends_the_viewer() {
        // (follows a room, the ends in order; what comes after the last),
        // for a viewer whose first failure ends it, as `xmoq-watch`
        let cases = [
            (false, vec![Ended::Again], After::End),
            (true, vec![Ended::Again], After::Connect),
            (true, vec![Ended::Quit], After::End),
            (
                false,
                vec![Ended::Failed { began: true }],
                After::EndWithError,
            ),
            (
                true,
                vec![Ended::Failed { began: false }],
                After::EndWithError,
            ),
            (true, vec![Ended::Failed { began: true }], After::Connect),
            (
                true,
                vec![Ended::Again, Ended::Failed { began: false }],
                After::Connect,
            ),
            (false, vec![Ended::Moved(None)], After::Connect),
        ];
        for (follows, ends, expected) in cases {
            let mut rule = NextSession::new(1000, follows, true);
            let mut after = After::Connect;
            for (at, end) in ends.iter().enumerate() {
                rule.next("https://a/", u64::try_from(at).unwrap() * 1000);
                after = rule.ended(end.clone());
            }
            assert_eq!(after, expected, "{follows} {ends:?}");
        }
        // One whose first failure does not end it, as `xmoq-agent`, goes on
        // where it follows a room, and ends where it does not.
        for (follows, expected) in [(true, After::Connect), (false, After::EndWithError)] {
            let mut rule = NextSession::new(1000, follows, false);
            rule.next("https://a/", 0);
            assert_eq!(
                rule.ended(Ended::Failed { began: false }),
                expected,
                "{follows}"
            );
        }
    }
}
