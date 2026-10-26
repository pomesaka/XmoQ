//! When and where a client connects again after a session ended: the rule
//! the terminal viewer (`xmoq-watch`), the agent (`xmoq-agent`), the
//! gatherer (`xmoq-chat`) and the transcriber (`xmoq-transcriber`) go by.
//! Time comes in as milliseconds from the caller's clock.
//!
//! The rows of `fixtures/ask-again/next-session.tsv` are read in the test;
//! `createNextSession` of `@xmoq/moqt` goes by the same rule and table: the
//! wait counts from the try before, not from the end of the session.

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
    /// A track the session could not do without broke (the terminal
    /// viewer's video was malformed, §2.4.2): a new session at the same
    /// place, after the usual wait.
    ///
    /// WHY NOT `Moved(None)`: that would hold the place as one left, and the
    /// relay is not going away. WHY NOT `Failed`: a viewer without a room
    /// would end, for one Group a publisher got wrong.
    Broken,
    /// The viewer is to end.
    Quit,
}

/// Where the New Session URI of a GOAWAY says to go: `None` for the URL of
/// the session (§10.4: "If the URI is zero bytes long, the current URI is
/// reused instead"). Bytes that are not UTF-8 are replaced, as a URL that
/// cannot be connected to.
#[must_use]
pub fn new_url(new_session_uri: &[u8]) -> Option<String> {
    (!new_session_uri.is_empty()).then(|| String::from_utf8_lossy(new_session_uri).into_owned())
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

/// The pause from one try at a session to the next, where the next is not
/// the one made at once after a GOAWAY: [`Self::first_ms`] at first, twice
/// the one before after each try, up to [`Self::most_ms`], and
/// [`Self::first_ms`] again after a session that lasted
/// [`Self::went_well_ms`] from its try. It counts from the try before, not
/// from the end of the session. Each role has its own (`fixtures/ask-again/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pause {
    /// The first pause, in milliseconds.
    pub first_ms: u64,
    /// The longest, in milliseconds.
    pub most_ms: u64,
    /// How long a session has to have lasted from its try for the pause to
    /// begin again from [`Self::first_ms`]; `None` where it never grows.
    pub went_well_ms: Option<u64>,
}

impl Pause {
    /// The viewers' and the agent's: 1 s from one try to the next, however
    /// often a room names another place. **Provisional.** WHY it does not
    /// grow: a person waits for the picture.
    pub const VIEWER: Self = Self {
        first_ms: 1_000,
        most_ms: 1_000,
        went_well_ms: None,
    };

    /// The gatherer's and the transcriber's: 1 s, doubling to 30 s, and 1 s
    /// again after a session of a minute. **Provisional**: long against a
    /// relay that is being started again, short against a person waiting
    /// for the chat. WHY it grows: a relay that ends every session at once
    /// would have such a client connect every second for good, and nobody
    /// watches it. WHY it comes back to the first: after a session that went
    /// well for a while, the next end is one of its own and not the same
    /// one again.
    pub const SERVING: Self = Self {
        first_ms: 1_000,
        most_ms: 30_000,
        went_well_ms: Some(60_000),
    };
}

/// The pauses of a [`Pause`] one after the other: the gatherer's and the
/// transcriber's wait before a `SUBSCRIBE_NAMESPACE` that ended is asked
/// again, and the wait of [`NextSession`].
#[derive(Debug, Clone)]
pub struct Growing {
    pause: Pause,
    next_ms: u64,
}

impl Growing {
    /// At the first pause.
    #[must_use]
    pub fn new(pause: Pause) -> Self {
        Self {
            pause,
            next_ms: pause.first_ms,
        }
    }

    /// The pause after something that lasted `lasted_ms` from its try ended:
    /// the first after one that lasted [`Pause::went_well_ms`], else twice
    /// the one before, up to [`Pause::most_ms`].
    pub fn after(&mut self, lasted_ms: u64) -> u64 {
        if self
            .pause
            .went_well_ms
            .is_some_and(|well| lasted_ms >= well)
        {
            self.next_ms = self.pause.first_ms;
        }
        let pause = self.next_ms;
        self.next_ms = pause.saturating_mul(2).min(self.pause.most_ms);
        pause
    }
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
    /// The pause of each try that is not made at once.
    pauses: Growing,
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
    /// The rule for a client that pauses by `pause` from one try to the
    /// next, and that `follows` a room: one whose room says what comes next
    /// connects again after a broadcast ended or broke; one without ends.
    /// `first_failure_ends`: a first try that fails before anything came of
    /// it ends the client, though it follows a room.
    #[must_use]
    pub fn new(pause: Pause, follows: bool, first_failure_ends: bool) -> Self {
        Self {
            pauses: Growing::new(pause),
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
        let wait_ms = match self.tried_at_ms {
            _ if at_once => 0,
            None => 0,
            Some(tried) => {
                let lasted = now_ms.saturating_sub(tried);
                self.pauses.after(lasted).saturating_sub(lasted)
            }
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
            Ended::Quit => After::End,
            Ended::Again if !self.follows => After::End,
            Ended::Failed { began } if !self.follows || (failure_ends && !began) => {
                After::EndWithError
            }
            // A broken track connects again with a room or without.
            Ended::Again | Ended::Broken | Ended::Failed { .. } => After::Connect,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pause that does not grow.
    fn fixed(ms: u64) -> Pause {
        Pause {
            first_ms: ms,
            most_ms: ms,
            went_well_ms: None,
        }
    }

    /// The second column of a row of `next-session.tsv`:
    /// `<first>,<most>,<went well>`, `-` for a pause that does not begin
    /// again.
    fn pause_of(text: &str) -> Pause {
        let [first, most, went_well] = text.split(',').collect::<Vec<_>>()[..] else {
            panic!("a pause of three values: {text}");
        };
        Pause {
            first_ms: first.parse().expect("the first pause"),
            most_ms: most.parse().expect("the longest"),
            went_well_ms: (went_well != "-").then(|| went_well.parse().expect("how long")),
        }
    }

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
        assert_eq!(rows.len(), 9, "{path}");
        assert!(
            table.ends_with('\n'),
            "{path}: its last row ends with a line feed"
        );
        for row in rows {
            let [name, pause, ends] = row.as_slice() else {
                panic!("a row of three columns: {row:?}");
            };
            // A room is followed: no end ends the client.
            let mut rule = NextSession::new(pause_of(pause), true, false);
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
    fn the_serving_pause_doubles_up_to_its_most_and_begins_again_after_a_long_run() {
        // (case, how long each lasted, the pauses after each, in seconds)
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
            let mut pauses = Growing::new(Pause::SERVING);
            let got: Vec<u64> = lasted
                .iter()
                .map(|lasted| pauses.after(lasted * 1_000) / 1_000)
                .collect();
            assert_eq!(got, expected, "{case}");
        }
        // The viewers' does not grow.
        let mut pauses = Growing::new(Pause::VIEWER);
        let got: Vec<u64> = (0..4).map(|_| pauses.after(0)).collect();
        assert_eq!(got, [1_000; 4]);
    }

    #[test]
    fn a_goaway_names_where_the_next_sessions_go() {
        let mut rule = NextSession::new(fixed(1000), true, true);
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
            (false, vec![Ended::Broken], After::Connect),
            (true, vec![Ended::Broken], After::Connect),
        ];
        for (follows, ends, expected) in cases {
            let mut rule = NextSession::new(fixed(1000), follows, true);
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
            let mut rule = NextSession::new(fixed(1000), follows, false);
            rule.next("https://a/", 0);
            assert_eq!(
                rule.ended(Ended::Failed { began: false }),
                expected,
                "{follows}"
            );
        }
    }
}
