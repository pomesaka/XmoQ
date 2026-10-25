//! The flags of the reader.
//!
//! WHY its own, beside those of `xmoq-watch`: most of the viewer's are about
//! playing (the buffer, the sound, the terminal), and the two would be one
//! parser of flags that each program uses half of.

use xmoq_picture::frames::Settings;
use xmoq_viewer::chat_track::{CATCH_UP_LINES, CATCH_UP_LINES_MOST, room_of_address};
use xmoq_viewer::meeting::Limits;
use xmoq_viewer::room::certificate_hash;

/// Where the broadcast is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A relay, the hash of its certificate, and the Track Namespace.
    Relay {
        /// `--url`.
        url: String,
        /// `--cert-sha256`.
        cert_sha256: [u8; 32],
        /// `--namespace`, its fields.
        namespace: Vec<String>,
    },
    /// A room that says where the broadcast is and gives tokens.
    Room {
        /// `--room`: `http://host:port/rooms/<name>`.
        address: String,
        /// `--token no` asks the room for none.
        ask_token: bool,
    },
}

/// What the reader was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Where the broadcast is.
    pub source: Source,
    /// The room of the chat: with `--room`, the last segment of its
    /// address; with `--url`, `--chat-room`.
    pub room: String,
    /// `--name`: the reader's name in the chat.
    pub name: String,
    /// `--out`: the directory of `events.jsonl` and what it names.
    pub out: String,
    /// `--chat-catch-up-lines`: how many of the lines missed while the
    /// subscription to `lines` was away are fetched, 1 to 1,000.
    pub catch_up: u64,
    /// `--answer-wait-ms`.
    pub answer_wait_ms: u64,
    /// `--setup-wait-ms`.
    pub setup_wait_ms: u64,
    /// `--open-wait-ms`.
    pub open_wait_ms: u64,
    /// `--seconds`: the reader ends after so long.
    pub seconds: Option<u64>,
    /// `--frames off`: the video is not subscribed to, and `/frame` gets
    /// no picture.
    pub frames: bool,
    /// `--frame-every-ms`, `--frame-longest`, `--frames-kept`; the defaults
    /// are `xmoq-picture`'s.
    pub pictures: Settings,
    /// `--speech off`: no transcript is followed.
    pub speech: bool,
    /// `--transcript`: the prefix the transcriber listens under, whose
    /// transcript `xmoq/transcript/<prefix>` is followed, its fields. `None`:
    /// the broadcast's namespace.
    pub transcript: Option<Vec<String>>,
    /// `--meeting`: the room of a meeting, whose participants publish under
    /// `xmoq/meet/<room>/<id>`, in place of one broadcast; with the limits
    /// of `--meeting-most` and `--meeting-video-linger-ms`.
    pub meeting: Option<(String, Limits)>,
}

/// What is wrong with the flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// The value of `xmoq_moqt::params::ANSWER_WAIT_MS`, which this library
/// does not depend on, as `xmoq-watch` has it. **Provisional.**
const DEFAULT_WAIT_MS: u64 = 10_000;

/// The name in the chat where `--name` does not say.
pub const DEFAULT_NAME: &str = "agent";

/// The flags, each of which takes a value.
pub const USAGE: &str = "\
xmoq-agent --room http://host:port/rooms/<name> --out <dir> [--name agent] [--token no]
xmoq-agent --url https://host:port/ --cert-sha256 <64 hex digits> [--namespace xmoq/demo]
           (--chat-room <room> | --meeting <room>) --out <dir> [--name agent]
  --out <dir>                       where events.jsonl is written (made where it is not)
  --chat-catch-up-lines <n>         the lines missed while away that are fetched (16; 1 to 1000)
  --answer-wait-ms <ms>             how long the relay gets to answer a request (10000)
  --setup-wait-ms <ms>              how long the relay gets for its SETUP (10000)
  --open-wait-ms <ms>               how long the relay gets to allow a stream for a request (10000)
  --seconds <n>                     end after so long
  --frames on|off                   off: the video is not subscribed to, and /frame gets no picture (on)
  --frame-every-ms <ms>             a picture this often by the broadcast's clock, unasked (none)
  --frame-longest <n>               the longest side of a picture, in pixels (1280)
  --frames-kept <n>                 the pictures kept in <dir>/frames; the oldest goes (500)
  --meeting <room>                  a meeting's participants under xmoq/meet/<room>, in place of
                                    --namespace; /frame <id> asks for one's picture
  --meeting-most <n>                the participants kept (49; 1 or more)
  --meeting-video-linger-ms <ms>    how long a participant's video stays after its picture (30000)
  --speech on|off                   off: no transcript is followed (on)
  --transcript <prefix>             the prefix the transcriber listens under (the broadcast's namespace)
Each event is a line of JSON on stdout and in <dir>/events.jsonl. A line of stdin is said in the
chat; /frame asks for a picture; a line that begins with // is said with one / taken off.";

fn number(flag: &str, value: &str) -> Result<u64, Invalid> {
    value
        .parse()
        .map_err(|_| Invalid(format!("{flag} {value}: not a number it takes")))
}

fn positive(flag: &str, value: &str) -> Result<u64, Invalid> {
    let number = number(flag, value)?;
    if number == 0 {
        return Err(Invalid(format!("{flag} 0: 1 or more")));
    }
    Ok(number)
}

fn size(flag: &str, value: &str) -> Result<usize, Invalid> {
    usize::try_from(positive(flag, value)?)
        .map_err(|_| Invalid(format!("{flag} {value}: not a number it takes")))
}

/// Reads the flags, without the name of the program.
///
/// # Errors
///
/// [`Invalid`] for a flag that is not one, a flag without its value, a
/// value a flag does not take, neither or both of `--url` and `--room`,
/// `--url` without `--cert-sha256` or `--chat-room`, `--chat-room` with
/// `--room`, a room's address without the room's name, and no `--out`.
#[expect(
    clippy::too_many_lines,
    reason = "one arm a flag: cut in two, the list of the flags would be in two places"
)]
pub fn parse(mut given: impl Iterator<Item = String>) -> Result<Args, Invalid> {
    let (mut url, mut cert_sha256, mut namespace) = (None, None, "xmoq/demo".to_string());
    let (mut room, mut chat_room, mut ask_token) = (None, None, true);
    let (mut name, mut out) = (DEFAULT_NAME.to_string(), None);
    let mut catch_up = CATCH_UP_LINES;
    let (mut answer_wait_ms, mut setup_wait_ms, mut open_wait_ms) =
        (DEFAULT_WAIT_MS, DEFAULT_WAIT_MS, DEFAULT_WAIT_MS);
    let mut seconds = None;
    let (mut frames, mut pictures) = (true, Settings::default());
    let (mut speech, mut transcript) = (true, None);
    let (mut meeting, mut limits, mut named_namespace) = (None, Limits::default(), false);
    while let Some(flag) = given.next() {
        let value = given
            .next()
            .ok_or_else(|| Invalid(format!("{flag} needs a value")))?;
        match flag.as_str() {
            "--url" => url = Some(value),
            "--cert-sha256" => {
                cert_sha256 = Some(
                    certificate_hash(&value)
                        .map_err(|error| Invalid(format!("--cert-sha256: {error}")))?,
                );
            }
            "--namespace" => {
                namespace = value;
                named_namespace = true;
            }
            "--meeting" => {
                if value.is_empty() || value.contains('/') {
                    return Err(Invalid(format!("--meeting {value}: one field, not empty")));
                }
                meeting = Some(value);
            }
            "--meeting-most" => limits.most_participants = size(&flag, &value)?,
            "--meeting-video-linger-ms" => limits.video_linger_ms = number(&flag, &value)?,
            "--room" => room = Some(value),
            "--chat-room" => chat_room = Some(value),
            "--token" => {
                ask_token = match value.as_str() {
                    "yes" => true,
                    "no" => false,
                    _ => return Err(Invalid(format!("--token {value}: yes or no"))),
                };
            }
            "--name" => name = value,
            "--out" => out = Some(value),
            "--chat-catch-up-lines" => {
                catch_up = value
                    .parse()
                    .ok()
                    .filter(|lines| (1..=CATCH_UP_LINES_MOST).contains(lines))
                    .ok_or_else(|| {
                        Invalid(format!(
                            "--chat-catch-up-lines is a whole number from 1 to {CATCH_UP_LINES_MOST}, not {value}"
                        ))
                    })?;
            }
            "--answer-wait-ms" => answer_wait_ms = number(&flag, &value)?,
            "--setup-wait-ms" => setup_wait_ms = number(&flag, &value)?,
            "--open-wait-ms" => open_wait_ms = number(&flag, &value)?,
            "--seconds" => seconds = Some(number(&flag, &value)?),
            "--frames" => {
                frames = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(Invalid(format!("--frames {value}: on or off"))),
                };
            }
            "--frame-every-ms" => {
                pictures.every_us = Some(
                    positive(&flag, &value)?
                        .checked_mul(1000)
                        .ok_or_else(|| Invalid(format!("{flag} {value}: too long")))?,
                );
            }
            "--speech" => {
                speech = match value.as_str() {
                    "on" => true,
                    "off" => false,
                    _ => return Err(Invalid(format!("--speech {value}: on or off"))),
                };
            }
            "--transcript" => {
                let fields: Vec<String> = value.split('/').map(str::to_string).collect();
                if fields.iter().any(String::is_empty) {
                    return Err(Invalid(format!(
                        "--transcript {value}: fields between slashes, none empty"
                    )));
                }
                transcript = Some(fields);
            }
            "--frame-longest" => pictures.longest_side = size(&flag, &value)?,
            "--frames-kept" => pictures.most_kept = size(&flag, &value)?,
            other => return Err(Invalid(format!("unknown flag {other}"))),
        }
    }
    let out = out.ok_or_else(|| Invalid("--out <dir> is needed".to_string()))?;
    let (source, room) = match (url, room) {
        (Some(url), None) => (
            Source::Relay {
                url,
                cert_sha256: cert_sha256
                    .ok_or_else(|| Invalid("--url needs --cert-sha256".to_string()))?,
                // The flag has the fields between slashes.
                namespace: namespace.split('/').map(str::to_string).collect(),
            },
            // A meeting's chat is the meeting's room unless it is named.
            chat_room
                .or_else(|| meeting.clone())
                .ok_or_else(|| Invalid("--url needs --chat-room or --meeting".to_string()))?,
        ),
        (None, Some(address)) => {
            if chat_room.is_some() {
                return Err(Invalid(
                    "--chat-room is for --url; with --room the chat is the room's".to_string(),
                ));
            }
            let room = room_of_address(&address)
                .ok_or_else(|| {
                    Invalid(format!(
                        "--room {address}: http://host:port/rooms/<name> is needed"
                    ))
                })?
                .to_string();
            (Source::Room { address, ask_token }, room)
        }
        (None, None) => return Err(Invalid("--url or --room is needed".to_string())),
        (Some(_), Some(_)) => return Err(Invalid("--url or --room, not both".to_string())),
    };
    // WHY not with a room yet: how a room lets the participants of a
    // meeting in, and gives their tokens, waits for how they find each other
    // (Q57).
    if meeting.is_some() && matches!(source, Source::Room { .. }) {
        return Err(Invalid(
            "--meeting is for --url, not yet for --room".to_string(),
        ));
    }
    if meeting.is_some() && named_namespace {
        return Err(Invalid(
            "--meeting or --namespace: a meeting has a broadcast for each participant".to_string(),
        ));
    }
    Ok(Args {
        meeting: meeting.map(|room| (room, limits)),
        source,
        room,
        name,
        out,
        catch_up,
        answer_wait_ms,
        setup_wait_ms,
        open_wait_ms,
        seconds,
        frames,
        pictures,
        speech,
        transcript,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Args, Invalid> {
        parse(line.split_whitespace().map(str::to_string))
    }

    fn in_room(room: &str, name: &str) -> Args {
        Args {
            source: Source::Room {
                address: format!("http://h/rooms/{room}"),
                ask_token: true,
            },
            room: room.to_string(),
            name: name.to_string(),
            out: "o".to_string(),
            catch_up: 16,
            answer_wait_ms: 10_000,
            setup_wait_ms: 10_000,
            open_wait_ms: 10_000,
            seconds: None,
            frames: true,
            pictures: Settings {
                every_us: None,
                longest_side: 1280,
                most_kept: 500,
            },
            speech: true,
            transcript: None,
            meeting: None,
        }
    }

    #[test]
    fn the_flags_of_a_meeting_are_read() {
        let hash = "0f".repeat(32);
        let relay = format!("--url https://h:1/ --cert-sha256 {hash} --out o");
        let at_relay = |room: &str, meeting: Option<(&str, Limits)>| Args {
            source: Source::Relay {
                url: "https://h:1/".to_string(),
                cert_sha256: [0x0f; 32],
                namespace: vec!["xmoq".to_string(), "demo".to_string()],
            },
            room: room.to_string(),
            meeting: meeting.map(|(room, limits)| (room.to_string(), limits)),
            ..in_room(room, "agent")
        };
        // (the flags after `relay`; what they are read as)
        let cases = [
            (
                "--meeting m",
                Ok(at_relay("m", Some(("m", Limits::default())))),
            ),
            // The chat of another room than the meeting's.
            (
                "--meeting m --chat-room c",
                Ok(at_relay("c", Some(("m", Limits::default())))),
            ),
            (
                "--meeting m --meeting-most 3 --meeting-video-linger-ms 0",
                Ok(at_relay(
                    "m",
                    Some((
                        "m",
                        Limits {
                            most_participants: 3,
                            video_linger_ms: 0,
                        },
                    )),
                )),
            ),
            // The participants cannot be unbounded.
            (
                "--meeting m --meeting-most 0",
                Err(Invalid("--meeting-most 0: 1 or more".to_string())),
            ),
            (
                "--meeting m --namespace xmoq/demo",
                Err(Invalid(
                    "--meeting or --namespace: a meeting has a broadcast for each participant"
                        .to_string(),
                )),
            ),
            (
                "--meeting m/n",
                Err(Invalid("--meeting m/n: one field, not empty".to_string())),
            ),
            (
                "",
                Err(Invalid("--url needs --chat-room or --meeting".to_string())),
            ),
        ];
        assert_eq!(
            parsed("--room http://h/rooms/a --out o --meeting m"),
            Err(Invalid(
                "--meeting is for --url, not yet for --room".to_string()
            ))
        );
        for (flags, expected) in cases {
            assert_eq!(parsed(&format!("{relay} {flags}")), expected, "{flags}");
        }
    }

    #[test]
    fn the_flags_of_the_pictures_are_read() {
        let room = "--room http://h/rooms/a --out o";
        // (the flags after `room`; what they are read as)
        let cases = [
            (
                "--frames off --frame-every-ms 5000 --frame-longest 640 --frames-kept 20",
                Ok(Args {
                    frames: false,
                    pictures: Settings {
                        every_us: Some(5_000_000),
                        longest_side: 640,
                        most_kept: 20,
                    },
                    ..in_room("a", "agent")
                }),
            ),
            (
                "--frames-kept 0",
                Err(Invalid("--frames-kept 0: 1 or more".to_string())),
            ),
            (
                "--frame-every-ms 0",
                Err(Invalid("--frame-every-ms 0: 1 or more".to_string())),
            ),
            (
                "--frame-every-ms 18446744073709552",
                Err(Invalid(
                    "--frame-every-ms 18446744073709552: too long".to_string(),
                )),
            ),
            (
                "--frames maybe",
                Err(Invalid("--frames maybe: on or off".to_string())),
            ),
            (
                "--speech off --transcript xmoq/voice",
                Ok(Args {
                    speech: false,
                    transcript: Some(vec!["xmoq".to_string(), "voice".to_string()]),
                    ..in_room("a", "agent")
                }),
            ),
            (
                "--transcript xmoq//voice",
                Err(Invalid(
                    "--transcript xmoq//voice: fields between slashes, none empty".to_string(),
                )),
            ),
            (
                "--speech maybe",
                Err(Invalid("--speech maybe: on or off".to_string())),
            ),
        ];
        for (flags, expected) in cases {
            assert_eq!(parsed(&format!("{room} {flags}")), expected, "{flags}");
        }
    }

    #[test]
    fn the_flags_are_read() {
        let hash = "0f".repeat(32);
        // (the flags; what they are read as)
        let cases = [
            (
                "--room http://h/rooms/a --out o".to_string(),
                Ok(in_room("a", "agent")),
            ),
            (
                "--room http://h/rooms/a --out o --name claude".to_string(),
                Ok(in_room("a", "claude")),
            ),
            (
                "--room http://h/rooms/a --out o --token no --chat-catch-up-lines 1000 \
                 --answer-wait-ms 1 --setup-wait-ms 2 --open-wait-ms 3 --seconds 4"
                    .to_string(),
                Ok(Args {
                    source: Source::Room {
                        address: "http://h/rooms/a".to_string(),
                        ask_token: false,
                    },
                    catch_up: 1000,
                    answer_wait_ms: 1,
                    setup_wait_ms: 2,
                    open_wait_ms: 3,
                    seconds: Some(4),
                    ..in_room("a", "agent")
                }),
            ),
            (
                format!("--url https://h:1/ --cert-sha256 {hash} --chat-room r --out o"),
                Ok(Args {
                    source: Source::Relay {
                        url: "https://h:1/".to_string(),
                        cert_sha256: [0x0f; 32],
                        namespace: vec!["xmoq".to_string(), "demo".to_string()],
                    },
                    room: "r".to_string(),
                    ..in_room("a", "agent")
                }),
            ),
            (
                "--room http://h/rooms/a".to_string(),
                Err(Invalid("--out <dir> is needed".to_string())),
            ),
            (
                "--room http://h --out o".to_string(),
                Err(Invalid(
                    "--room http://h: http://host:port/rooms/<name> is needed".to_string(),
                )),
            ),
            (
                "--room http://h/rooms/a --chat-room r --out o".to_string(),
                Err(Invalid(
                    "--chat-room is for --url; with --room the chat is the room's".to_string(),
                )),
            ),
            (
                format!("--url https://h:1/ --cert-sha256 {hash} --out o"),
                Err(Invalid("--url needs --chat-room or --meeting".to_string())),
            ),
            (
                "--url https://h:1/ --chat-room r --out o".to_string(),
                Err(Invalid("--url needs --cert-sha256".to_string())),
            ),
            (
                "--out o".to_string(),
                Err(Invalid("--url or --room is needed".to_string())),
            ),
            (
                "--room http://h/rooms/a --out o --chat-catch-up-lines 0".to_string(),
                Err(Invalid(
                    "--chat-catch-up-lines is a whole number from 1 to 1000, not 0".to_string(),
                )),
            ),
            (
                "--room http://h/rooms/a --out o --token maybe".to_string(),
                Err(Invalid("--token maybe: yes or no".to_string())),
            ),
            (
                "--room http://h/rooms/a --out".to_string(),
                Err(Invalid("--out needs a value".to_string())),
            ),
            (
                "--room http://h/rooms/a --out o --say hi".to_string(),
                Err(Invalid("unknown flag --say".to_string())),
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parsed(&line), expected, "{line}");
        }
    }
}
