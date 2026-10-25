//! A reader in a meeting: every participant publishes a broadcast of its
//! own under `xmoq/meet/<room>/<id>`, and the reader is told of them with
//! one `SUBSCRIBE_NAMESPACE` for `xmoq/meet/<room>` (draft 18 §6.1). It
//! follows nobody's sound: the transcriber listens to every broadcast under
//! the prefix and says who spoke (`from` of `speech`). It subscribes to one
//! participant's video at a time, the one a picture is asked for.
//!
//! **Proposed** (`docs/scale-and-meetings.md`, Part 2); how the participants
//! find each other is undecided (Q57), and this is the side that reads the
//! namespaces of the relay. WHY one video at a time, and not one for each:
//! a reader that asks for a picture now and then would receive 48 videos
//! for it, and keep the pictures of each apart (`--frames-kept` bounds
//! them together through one `Frames` of `xmoq-picture`). WHY a video is
//! kept a while after a picture: the next picture of the same participant
//! needs no new subscription, and no key frame to wait for.

use crate::presence::{Change, Presence};

/// The first two fields of a meeting's namespace.
pub const MEET: [&str; 2] = ["xmoq", "meet"];

/// What the reader of a meeting is bounded by. **Provisional**, each set by
/// a flag of the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The participants kept; one more is told of as left out. At least 1.
    pub most_participants: usize,
    /// How long a participant's video stays subscribed after the last
    /// picture of it was asked for, in milliseconds.
    pub video_linger_ms: u64,
}

impl Default for Limits {
    /// WHY 49: the most participants of a meeting (decided on 2026-10-25).
    /// WHY 30 s: an agent that looks at somebody often asks again within
    /// it, and one video of a meeting's small version is about 0.5 Mbit/s
    /// (`docs/scale-and-meetings.md`).
    fn default() -> Self {
        Self {
            most_participants: 49,
            video_linger_ms: 30_000,
        }
    }
}

/// The prefix of a meeting's broadcasts: `xmoq/meet/<room>`.
#[must_use]
pub fn prefix(room: &str) -> Vec<String> {
    MEET.iter()
        .map(ToString::to_string)
        .chain(std::iter::once(room.to_string()))
        .collect()
}

/// The participant a namespace is the broadcast of: the field after the
/// meeting's prefix, where the namespace has exactly one more. `None` for
/// any other, the prefix itself and a deeper one among them.
#[must_use]
pub fn participant_of<'a>(room: &str, namespace: &'a [String]) -> Option<&'a str> {
    match namespace {
        [xmoq, meet, of_room, id] if [xmoq.as_str(), meet.as_str()] == MEET && of_room == room => {
            Some(id.as_str())
        }
        _ => None,
    }
}

/// Why no participant is the one a picture is asked of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoTarget {
    /// `/frame <id>` for somebody who is not there.
    NotHere {
        /// The id asked for.
        asked: String,
        /// Who is there.
        here: Vec<String>,
    },
    /// `/frame` alone, and nobody of the meeting has spoken yet, or the one
    /// who spoke last has left.
    NobodySpoke {
        /// Who is there.
        here: Vec<String>,
    },
}

impl std::fmt::Display for NoTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (said, here) = match self {
            Self::NotHere { asked, here } => (format!("{asked} is not in the meeting"), here),
            Self::NobodySpoke { here } => ("nobody has spoken; /frame <id>".to_string(), here),
        };
        if here.is_empty() {
            write!(f, "{said}, and nobody is there")
        } else {
            write!(f, "{said}, and there are {}", here.join(", "))
        }
    }
}

/// What to do with the video subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Video {
    /// The participant's video is subscribed: nothing to do.
    Keep,
    /// Subscribe to the participant's video, after the subscription to
    /// `cancel`'s where there is one (and begin the pictures again).
    Subscribe {
        /// Whose video is subscribed now, to be cancelled first.
        cancel: Option<String>,
        /// Whose to subscribe to.
        to: String,
    },
}

/// A meeting as the reader has it.
#[derive(Debug)]
pub struct Meeting {
    room: String,
    limits: Limits,
    presence: Presence,
    /// The participant whose voice was read last.
    last_speaker: Option<String>,
    /// Whose video is subscribed, and when a picture of it was last asked
    /// for (ms, the caller's clock).
    watched: Option<(String, u64)>,
}

impl Meeting {
    /// Nobody yet in the meeting `room`.
    #[must_use]
    pub fn new(room: &str, limits: Limits) -> Self {
        Self {
            room: room.to_string(),
            limits,
            presence: Presence::new(limits.most_participants),
            last_speaker: None,
            watched: None,
        }
    }

    /// The relay told of a namespace under the prefix (NAMESPACE).
    pub fn named(&mut self, namespace: &[String]) -> Option<Change> {
        let id = participant_of(&self.room, namespace)?;
        self.presence.named(id)
    }

    /// The relay told that a namespace under the prefix is gone
    /// (`NAMESPACE_DONE`). Whose video is watched is let go of with it: the
    /// caller cancels that subscription (`watching` is `None` then).
    pub fn done(&mut self, namespace: &[String]) -> Option<Change> {
        let id = participant_of(&self.room, namespace)?;
        let change = self.presence.done(id)?;
        self.forget(id);
        Some(change)
    }

    /// The session ended; see [`Presence::session_ended`]. No video is
    /// subscribed on the next one until a picture is asked for.
    pub fn session_ended(&mut self) {
        self.presence.session_ended();
        self.watched = None;
    }

    /// [`crate::presence::SETTLE_MS`] after the `SUBSCRIBE_NAMESPACE` of a
    /// session was answered.
    pub fn settled(&mut self) -> Vec<Change> {
        let changes = self.presence.settled();
        for change in &changes {
            if let Change::Left(id) = change {
                self.forget(id);
            }
        }
        changes
    }

    fn forget(&mut self, id: &str) {
        if self.last_speaker.as_deref() == Some(id) {
            self.last_speaker = None;
        }
        if self
            .watched
            .as_ref()
            .is_some_and(|(watched, _)| watched == id)
        {
            self.watched = None;
        }
    }

    /// A piece of the transcript was of the broadcast `from`.
    pub fn spoke(&mut self, from: &[String]) {
        if let Some(id) = participant_of(&self.room, from)
            && self.presence.has(id)
        {
            self.last_speaker = Some(id.to_string());
        }
    }

    /// The video of [`Video::Subscribe`] could not be subscribed to: none
    /// is, and the next picture asks for it anew.
    pub fn video_failed(&mut self) {
        self.watched = None;
    }

    /// Whose video is subscribed.
    #[must_use]
    pub fn watching(&self) -> Option<&str> {
        self.watched.as_ref().map(|(id, _)| id.as_str())
    }

    /// A picture is asked for at `now_ms`, of `asked` (`/frame <id>`) or of
    /// the one who spoke last (`/frame`): whose it is, and what to do with
    /// the video subscription for it.
    ///
    /// # Errors
    ///
    /// [`NoTarget`] where that participant is not there, or nobody spoke.
    pub fn asked(&mut self, asked: Option<&str>, now_ms: u64) -> Result<(String, Video), NoTarget> {
        let here = || self.presence.names().map(ToString::to_string).collect();
        let id = match asked {
            Some(id) if self.presence.has(id) => id.to_string(),
            Some(id) => {
                return Err(NoTarget::NotHere {
                    asked: id.to_string(),
                    here: here(),
                });
            }
            None => self
                .last_speaker
                .clone()
                .ok_or_else(|| NoTarget::NobodySpoke { here: here() })?,
        };
        let video = match self.watched.take() {
            Some((watched, _)) if watched == id => Video::Keep,
            other => Video::Subscribe {
                cancel: other.map(|(watched, _)| watched),
                to: id.clone(),
            },
        };
        self.watched = Some((id.clone(), now_ms));
        Ok((id, video))
    }

    /// At `now_ms`: the participant whose video is to be cancelled, no
    /// picture of it having been asked for in [`Limits::video_linger_ms`].
    pub fn linger_over(&mut self, now_ms: u64) -> Option<String> {
        let (_, asked_at) = self.watched.as_ref()?;
        if now_ms.saturating_sub(*asked_at) < self.limits.video_linger_ms {
            return None;
        }
        self.watched.take().map(|(id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(path: &str) -> Vec<String> {
        path.split('/').map(ToString::to_string).collect()
    }

    #[test]
    fn a_participant_is_the_field_after_the_prefix() {
        // (the namespace; the participant of the meeting `r`)
        let cases = [
            ("xmoq/meet/r/a1", Some("a1")),
            ("xmoq/meet/r", None),
            ("xmoq/meet/r/a1/more", None),
            ("xmoq/meet/other/a1", None),
            ("xmoq/chat/r/a1", None),
            ("xmoq/demo", None),
        ];
        for (namespace, expected) in cases {
            assert_eq!(
                participant_of("r", &fields(namespace)),
                expected,
                "{namespace}"
            );
        }
        assert_eq!(prefix("r"), fields("xmoq/meet/r"));
    }

    #[derive(Debug, Clone, Copy)]
    enum Step {
        Named(&'static str),
        Done(&'static str),
        Spoke(&'static str),
        /// `/frame`, with an id or without, at the time in ms.
        Asked(Option<&'static str>, u64),
        Linger(u64),
        VideoFailed,
        SessionEnded,
    }

    /// What a step gave, in a line.
    fn outcome(meeting: &mut Meeting, step: Step) -> Option<String> {
        match step {
            Step::Named(path) => meeting
                .named(&fields(path))
                .map(|change| format!("{change:?}")),
            Step::Done(path) => meeting
                .done(&fields(path))
                .map(|change| format!("{change:?}")),
            Step::Spoke(path) => {
                meeting.spoke(&fields(path));
                None
            }
            Step::Asked(id, now) => Some(match meeting.asked(id, now) {
                Ok((id, video)) => format!("{id} {video:?}"),
                Err(why) => format!("error: {why}"),
            }),
            Step::Linger(now) => meeting.linger_over(now).map(|id| format!("cancel {id}")),
            Step::VideoFailed => {
                meeting.video_failed();
                None
            }
            Step::SessionEnded => {
                meeting.session_ended();
                None
            }
        }
    }

    type Case = (&'static [Step], &'static [&'static str]);

    /// (what happens in the meeting `r`, a video kept 30 s; what each step
    /// gave, where it gave something)
    #[expect(
        clippy::too_many_lines,
        reason = "a table, one row a case: cut in two, a reader would look for a case in two places"
    )]
    fn cases() -> [Case; 10] {
        use Step::{Asked, Done, Linger, Named, SessionEnded, Spoke, VideoFailed};
        const A: &str = "xmoq/meet/r/a";
        const B: &str = "xmoq/meet/r/b";
        [
            (
                &[Named(A), Asked(Some("a"), 0)],
                &[r#"Came("a")"#, r#"a Subscribe { cancel: None, to: "a" }"#],
            ),
            // The same again: kept; another: the first is cancelled.
            (
                &[
                    Named(A),
                    Named(B),
                    Asked(Some("a"), 0),
                    Asked(Some("a"), 10),
                    Asked(Some("b"), 20),
                ],
                &[
                    r#"Came("a")"#,
                    r#"Came("b")"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                    "a Keep",
                    r#"b Subscribe { cancel: Some("a"), to: "b" }"#,
                ],
            ),
            // `/frame` alone: the one who spoke last.
            (
                &[Named(A), Named(B), Spoke(B), Spoke(A), Asked(None, 0)],
                &[
                    r#"Came("a")"#,
                    r#"Came("b")"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                ],
            ),
            (
                &[Named(A), Asked(None, 0)],
                &[
                    r#"Came("a")"#,
                    "error: nobody has spoken; /frame <id>, and there are a",
                ],
            ),
            // The voice of somebody not there, or of another broadcast.
            (
                &[Spoke(A), Spoke("xmoq/demo"), Asked(None, 0)],
                &["error: nobody has spoken; /frame <id>, and nobody is there"],
            ),
            (
                &[Named(A), Asked(Some("c"), 0)],
                &[
                    r#"Came("a")"#,
                    "error: c is not in the meeting, and there are a",
                ],
            ),
            // 30 s after the last picture, not before.
            (
                &[
                    Named(A),
                    Asked(Some("a"), 1_000),
                    Linger(30_999),
                    Linger(31_000),
                    Asked(Some("a"), 31_001),
                ],
                &[
                    r#"Came("a")"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                    "cancel a",
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                ],
            ),
            // Whose video is watched leaves: nothing to cancel later, and
            // the one who spoke last is gone with it.
            (
                &[
                    Named(A),
                    Spoke(A),
                    Asked(None, 0),
                    Done(A),
                    Linger(60_000),
                    Asked(None, 60_000),
                ],
                &[
                    r#"Came("a")"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                    r#"Left("a")"#,
                    "error: nobody has spoken; /frame <id>, and nobody is there",
                ],
            ),
            // A video that could not be subscribed to is asked for again.
            (
                &[
                    Named(A),
                    Asked(Some("a"), 0),
                    VideoFailed,
                    Asked(Some("a"), 10),
                ],
                &[
                    r#"Came("a")"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                ],
            ),
            // A new session subscribes anew.
            (
                &[
                    Named(A),
                    Asked(Some("a"), 0),
                    SessionEnded,
                    Named(A),
                    Asked(Some("a"), 10),
                ],
                &[
                    r#"Came("a")"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                    r#"a Subscribe { cancel: None, to: "a" }"#,
                ],
            ),
        ]
    }

    #[test]
    fn whose_picture_is_taken_and_which_video_is_subscribed() {
        for (steps, expected) in cases() {
            let mut meeting = Meeting::new("r", Limits::default());
            let got: Vec<String> = steps
                .iter()
                .filter_map(|step| outcome(&mut meeting, *step))
                .collect();
            assert_eq!(got, expected, "{steps:?}");
        }
    }

    #[test]
    fn participants_beyond_the_most_are_left_out() {
        let mut meeting = Meeting::new(
            "r",
            Limits {
                most_participants: 1,
                ..Limits::default()
            },
        );
        assert_eq!(
            meeting.named(&fields("xmoq/meet/r/a")),
            Some(Change::Came("a".to_string()))
        );
        assert_eq!(
            meeting.named(&fields("xmoq/meet/r/b")),
            Some(Change::TooMany("b".to_string()))
        );
        assert_eq!(
            meeting.asked(Some("b"), 0),
            Err(NoTarget::NotHere {
                asked: "b".to_string(),
                here: vec!["a".to_string()],
            })
        );
    }
}
