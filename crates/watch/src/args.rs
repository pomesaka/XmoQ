//! The flags of the viewer.

use xmoq_viewer::reconnect::{CONNECT_WAIT_MS, ROOM_WAIT_MS};
use xmoq_viewer::room::certificate_hash;

use crate::tracks::VideoVersion;

/// How a frame gets to the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Medium {
    /// Its pixels in the escape codes: a terminal anywhere.
    Direct,
    /// A temporary file the terminal reads and removes: the same machine.
    File,
}

/// How large a frame may be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// As large as the terminal's window, where the terminal says how
    /// many pixels it has; as it is where it does not.
    Terminal,
    /// As it is.
    AsItIs,
    /// No more than so many pixels.
    Most(usize, usize),
}

/// Where the broadcast is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A relay, the hash of its certificate, and the Track Namespace.
    Relay {
        /// `--url`.
        url: String,
        /// `--cert-sha256`.
        cert_sha256: [u8; 32],
        /// `--namespace`, its fields between slashes.
        namespace: String,
    },
    /// A room that says where the broadcast is and gives a token.
    Room {
        /// `--room`: `http://host:port/rooms/<name>`.
        address: String,
        /// `--name`: what the viewer is called in the chat.
        name: String,
        /// `--token no` asks the room for none.
        ask_token: bool,
    },
}

/// The chat on `MoQ` tracks ([`xmoq_viewer::chat_track`]): the room and the
/// viewer's name in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    /// With `--room`, the last segment of its address; with `--url`,
    /// `--chat-room`.
    pub room: String,
    /// `--name`.
    pub name: String,
    /// `--say`: a line said on `said` once the chat on tracks has begun.
    pub say: Option<String>,
    /// `--chat-catch-up-lines`: how many of the lines it missed are fetched
    /// when the subscription to `lines` is made again
    /// ([`xmoq_viewer::chat_track::history_to_fetch`]); 1 to
    /// [`xmoq_viewer::chat_track::CATCH_UP_LINES_MOST`].
    pub catch_up: u64,
}

/// What the viewer was started with.
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// Where the broadcast is.
    pub source: Source,
    /// The chat on `MoQ` tracks. `None` with `--url` without `--chat-room`.
    pub chat: Option<Chat>,
    /// `--video` and `--audio`: the names of the tracks. `None`: what the
    /// catalog of the broadcast has.
    pub tracks: Option<(String, String)>,
    /// `--video-version`: which version of the video is played where the
    /// catalog has several. Not with `--video`, which names the track.
    pub video_version: VideoVersion,
    /// `--buffer-ms`: how long an Object is held beyond the fastest way;
    /// with `auto`, where the buffer begins.
    pub buffer_ms: u64,
    /// `--buffer-ms auto`: the buffer follows how late Objects come
    /// ([`crate::clock::Clock::automatic`]).
    pub buffer_auto: bool,
    /// `--video-timeout-ms`: the `SUBGROUP_DELIVERY_TIMEOUT` the SUBSCRIBE
    /// of the video track carries (§10.2.3 of
    /// `draft-ietf-moq-transport-18`): the relay gives up a Group the
    /// viewer does not have that long after its last Object, and goes on
    /// with the next. `None`: the parameter is left out, and every Group is
    /// delivered however late.
    pub video_timeout_ms: Option<u64>,
    /// `--answer-wait-ms`: how long the answer to a request is waited for
    /// before the parameters of the request are looked at
    /// (`xmoq_moqt::params::answer_wait_ms`); 10,000 where it is not said.
    pub answer_wait_ms: u64,
    /// `--setup-wait-ms`: how long the relay's SETUP is waited for; 10,000
    /// where it is not said.
    pub setup_wait_ms: u64,
    /// `--open-wait-ms`: how long a request may wait for its stream to be
    /// opened, for the relay to allow one more; 10,000 where it is not
    /// said.
    pub open_wait_ms: u64,
    /// `--room-wait-ms`: how long the room gets for each of its answers
    /// (`xmoq_viewer::reconnect::ROOM_WAIT_MS` where it is not said).
    pub room_wait_ms: u64,
    /// `--connect-wait-ms`: how long the relay gets to be connected to
    /// (`xmoq_viewer::reconnect::CONNECT_WAIT_MS` where it is not said).
    pub connect_wait_ms: u64,
    /// `--rendezvous-timeout-ms`: the `RENDEZVOUS_TIMEOUT` of every
    /// SUBSCRIBE the viewer sends (§10.2.6 of
    /// `draft-ietf-moq-transport-18`): how long the relay may hold one for
    /// a publisher to appear, so that the viewer can come before the
    /// broadcast. `None`: the parameter is left out, and a broadcast that
    /// is not there is refused at once.
    pub rendezvous_ms: Option<u64>,
    /// `--volume`: 0 to 1.
    pub volume: f32,
    /// `--scale`: how many times its pixels the picture takes of the
    /// terminal.
    pub scale: usize,
    /// `--medium`.
    pub medium: Medium,
    /// `--fit`.
    pub fit: Fit,
    /// `--out`: a file for what would go to the terminal.
    pub out: Option<String>,
    /// `--seconds`: the viewer ends after so long.
    pub seconds: Option<u64>,
    /// `--log`: a file for the lines about the room and the keys.
    pub log: Option<String>,
    /// `--sound off`: the sound output of the machine is not opened. The
    /// sound is decoded and its packets are taken from the queue at the
    /// moments an output would take them, so that everything that is
    /// counted is counted; nothing is heard. For a machine without an
    /// output.
    pub sound: bool,
    /// `--measure-demo-marks on`: for measuring with the demo's
    /// publisher, whose picture has a white square and whose sound a beep
    /// at the same moments; how far apart the two leave the viewer is
    /// reported at the end.
    pub measure_demo_marks: bool,
    /// `--clock-skew-ppm`: for measuring. The viewer's clock runs so many
    /// parts in a million faster than the machine's (slower with a
    /// negative number), as that of a viewer on another machine does
    /// against its publisher's. 0 where it is not said.
    pub clock_skew_ppm: i64,
    /// `--cut-sound-s`: for measuring. The viewer cancels its subscription
    /// to the sound so long after it was made, once, and the sound is asked
    /// for again as when a relay ends it; the watching page has the same
    /// (`/watch?cutSound=N`).
    pub cut_sound_s: Option<u64>,
}

/// The value of `xmoq_moqt::params::ANSWER_WAIT_MS`, which this library
/// does not depend on. **Provisional.**
const DEFAULT_ANSWER_WAIT_MS: u64 = 10_000;

/// The default of `--buffer-ms`, the watching page's default
/// (`DEFAULT_PLAYOUT_BUFFER_MS` of `@xmoq/media`, where the reasons are).
/// Decided on 2026-10-21.
pub const DEFAULT_BUFFER_MS: u64 = 150;

/// What is wrong with the flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// The flags, each of which takes a value.
pub const USAGE: &str = "\
xmoq-watch --url https://host:port/ --cert-sha256 <64 hex digits> [--namespace xmoq/demo]
           [--chat-room <room> [--name terminal] [--say <a line>]] [--chat-catch-up-lines 16]
xmoq-watch --room http://host:port/rooms/<name> [--name terminal] [--say <a line>] [--token no]
  --video <track> --audio <track>   the tracks by name; without them, those of the catalog
  --video-version first|name:<track>|height:<n>|bitrate:<n>
                                    which version of the video of the catalog: the first (first), the
                                    one of that name, or the largest of at most so many pixels of
                                    height or bits a second (the smallest where none is within it)
  --buffer-ms <ms>|auto             how long an Object is held beyond the fastest way (150);
                                    auto: it follows how late Objects come, from 150 up to 400 (a trial)
  --answer-wait-ms <ms>             how long the relay gets to answer a request (10000)
  --setup-wait-ms <ms>              how long the relay gets for its SETUP (10000)
  --rendezvous-timeout-ms <ms>      ask the relay to hold the viewer's SUBSCRIBEs that long for a
                                    broadcast that is not there yet (not asked; the relay holds 30 s at most)
  --open-wait-ms <ms>               how long the relay gets to allow a stream for a request (10000)
  --room-wait-ms <ms>               how long the room gets for each of its answers (5000)
  --connect-wait-ms <ms>            how long the relay gets to be connected to (10000)
  --video-timeout-ms <ms>           ask the relay to give up a Group of the video that is not delivered
                                    that long after its last Object (not asked; 0 is not asking)
  --volume <0..1>                   (1)
  --scale <n>                       the picture at n times its pixels (1)
  --medium file|direct              a file the terminal reads (file), or the pixels in the escape codes
  --fit terminal|none|<w>x<h>       the most pixels that are written (terminal: the window's)
  --out <file>                      what would go to the terminal, to a file
  --seconds <n>                     end after so long
  --log <file>                      the lines about the room and the keys
  --sound on|off                    off: the sound is decoded and counted, and no output is opened (on)
  --measure-demo-marks on|off       for measuring with the demo's publisher: how far apart the
                                    white square and the beep leave the viewer (off)
  --clock-skew-ppm <n>              for measuring: the viewer's clock runs n parts in a million
                                    faster than the machine's (0)
  --cut-sound-s <n>                 for measuring: the sound's subscription is cancelled once, n s
                                    after it was made, and asked for again
Keys: q or Ctrl-C ends, + and - the volume, m mutes, t or Enter begins a line for the chat.";

fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, Invalid> {
    value
        .parse()
        .map_err(|_| Invalid(format!("{flag} {value}: not a number it takes")))
}

fn one_of<T: Copy>(flag: &str, value: &str, choices: &[(&str, T)]) -> Result<T, Invalid> {
    choices
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, choice)| *choice)
        .ok_or_else(|| {
            let names: Vec<&str> = choices.iter().map(|(name, _)| *name).collect();
            Invalid(format!("{flag} {value}: {}", names.join(" or ")))
        })
}

fn video_version(value: &str) -> Result<VideoVersion, Invalid> {
    let flag = "--video-version";
    match value.split_once(':') {
        None if value == "first" => Ok(VideoVersion::First),
        Some(("name", name)) if !name.is_empty() => Ok(VideoVersion::Named(name.to_string())),
        // The message has the value as it was typed.
        Some(("height", most)) => most
            .parse()
            .map(VideoVersion::HeightAtMost)
            .map_err(|_| Invalid(format!("{flag} {value}: not a number it takes"))),
        Some(("bitrate", most)) => most
            .parse()
            .map(VideoVersion::BitrateAtMost)
            .map_err(|_| Invalid(format!("{flag} {value}: not a number it takes"))),
        _ => Err(Invalid(format!(
            "{flag} {value}: first, name:<track>, height:<n> or bitrate:<n>"
        ))),
    }
}

fn fit(value: &str) -> Result<Fit, Invalid> {
    match value {
        "terminal" => Ok(Fit::Terminal),
        "none" => Ok(Fit::AsItIs),
        size => {
            let invalid = || Invalid(format!("--fit {size}: terminal, none or <width>x<height>"));
            let (width, height) = size.split_once('x').ok_or_else(invalid)?;
            let (width, height): (usize, usize) = (
                width.parse().map_err(|_| invalid())?,
                height.parse().map_err(|_| invalid())?,
            );
            if width == 0 || height == 0 {
                return Err(invalid());
            }
            Ok(Fit::Most(width, height))
        }
    }
}

/// Reads the flags, without the name of the program.
///
/// # Errors
///
/// [`Invalid`] for a flag that is not one, a flag without its value, a
/// value a flag does not take, neither or both of `--url` and `--room`,
/// `--url` without `--cert-sha256`, and one of `--video` and `--audio`
/// without the other.
#[allow(
    clippy::too_many_lines,
    reason = "one arm a flag: cut in two, the list of the flags would be in two places"
)]
pub fn parse(mut given: impl Iterator<Item = String>) -> Result<Args, Invalid> {
    let (mut url, mut cert_sha256, mut namespace) = (None, None, "xmoq/demo".to_string());
    let (mut room, mut name, mut say, mut ask_token) = (None, "terminal".to_string(), None, true);
    let (mut video, mut audio) = (None, None);
    let mut chat_room = None;
    let mut catch_up = xmoq_viewer::chat_track::CATCH_UP_LINES;
    let mut args = Args {
        // Replaced below: a source is asked for.
        source: Source::Room {
            address: String::new(),
            name: String::new(),
            ask_token: true,
        },
        chat: None,
        tracks: None,
        video_version: VideoVersion::First,
        buffer_ms: DEFAULT_BUFFER_MS,
        buffer_auto: false,
        video_timeout_ms: None,
        answer_wait_ms: DEFAULT_ANSWER_WAIT_MS,
        setup_wait_ms: DEFAULT_ANSWER_WAIT_MS,
        open_wait_ms: DEFAULT_ANSWER_WAIT_MS,
        room_wait_ms: ROOM_WAIT_MS,
        connect_wait_ms: CONNECT_WAIT_MS,
        rendezvous_ms: None,
        volume: 1.0,
        scale: 1,
        medium: Medium::File,
        fit: Fit::Terminal,
        out: None,
        seconds: None,
        log: None,
        sound: true,
        measure_demo_marks: false,
        clock_skew_ppm: 0,
        cut_sound_s: None,
    };
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
            "--namespace" => namespace = value,
            "--chat-room" => chat_room = Some(value),
            "--chat-catch-up-lines" => {
                catch_up = value
                    .parse()
                    .ok()
                    .filter(|lines| {
                        (1..=xmoq_viewer::chat_track::CATCH_UP_LINES_MOST).contains(lines)
                    })
                    .ok_or_else(|| {
                        Invalid(format!(
                            "--chat-catch-up-lines is a whole number from 1 to {}, not {value}",
                            xmoq_viewer::chat_track::CATCH_UP_LINES_MOST
                        ))
                    })?;
            }
            "--room" => room = Some(value),
            "--name" => name = value,
            "--say" => say = Some(value),
            "--token" => ask_token = one_of(&flag, &value, &[("yes", true), ("no", false)])?,
            "--video" => video = Some(value),
            "--audio" => audio = Some(value),
            "--video-version" => args.video_version = video_version(&value)?,
            "--buffer-ms" if value == "auto" => {
                args.buffer_ms = DEFAULT_BUFFER_MS;
                args.buffer_auto = true;
            }
            "--buffer-ms" => {
                args.buffer_auto = false;
                args.buffer_ms = number(&flag, &value)?;
                // More than the clock takes an Object to be of it.
                if args.buffer_ms > crate::clock::MOST_FROM_THE_LEAST_US / 1000 {
                    return Err(Invalid(format!("--buffer-ms {value}: 10000 at most")));
                }
            }
            "--video-timeout-ms" => {
                // §8: "a value of 0 means that there is no timeout set".
                let timeout: u64 = number(&flag, &value)?;
                args.video_timeout_ms = (timeout > 0).then_some(timeout);
            }
            "--answer-wait-ms" => args.answer_wait_ms = number(&flag, &value)?,
            "--setup-wait-ms" => args.setup_wait_ms = number(&flag, &value)?,
            "--open-wait-ms" => args.open_wait_ms = number(&flag, &value)?,
            "--room-wait-ms" => args.room_wait_ms = number(&flag, &value)?,
            "--connect-wait-ms" => args.connect_wait_ms = number(&flag, &value)?,
            "--rendezvous-timeout-ms" => {
                // §10.2.6: 0 is not wanting to wait, as without it.
                let wait: u64 = number(&flag, &value)?;
                args.rendezvous_ms = (wait > 0).then_some(wait);
            }
            "--volume" => {
                let volume: f32 = number(&flag, &value)?;
                if !(0.0..=1.0).contains(&volume) {
                    return Err(Invalid(format!("--volume {value}: from 0 to 1")));
                }
                args.volume = volume;
            }
            "--scale" => {
                args.scale = number(&flag, &value)?;
                if args.scale == 0 {
                    return Err(Invalid("--scale 0: 1 or more".to_string()));
                }
            }
            "--medium" => {
                args.medium = one_of(
                    &flag,
                    &value,
                    &[("file", Medium::File), ("direct", Medium::Direct)],
                )?;
            }
            "--fit" => args.fit = fit(&value)?,
            "--out" => args.out = Some(value),
            "--seconds" => args.seconds = Some(number(&flag, &value)?),
            "--cut-sound-s" => args.cut_sound_s = Some(number(&flag, &value)?),
            "--log" => args.log = Some(value),
            "--sound" => args.sound = one_of(&flag, &value, &[("on", true), ("off", false)])?,
            "--measure-demo-marks" => {
                args.measure_demo_marks = one_of(&flag, &value, &[("on", true), ("off", false)])?;
            }
            "--clock-skew-ppm" => {
                args.clock_skew_ppm = number(&flag, &value)?;
                // WHY a limit: at a million slower the clock stands, and
                // beyond it would run backwards.
                if !(-999_999..=1_000_000).contains(&args.clock_skew_ppm) {
                    return Err(Invalid(format!(
                        "--clock-skew-ppm {value}: from -999999 to 1000000"
                    )));
                }
            }
            other => return Err(Invalid(format!("unknown flag {other}"))),
        }
    }
    args.chat = match (&room, chat_room) {
        (Some(address), None) => {
            xmoq_viewer::chat_track::room_of_address(address).map(|room| Chat {
                room: room.to_string(),
                name: name.clone(),
                say: say.clone(),
                catch_up,
            })
        }
        (None, Some(room)) => Some(Chat {
            room,
            name: name.clone(),
            say: say.clone(),
            catch_up,
        }),
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(Invalid(
                "--chat-room is for --url; with --room the chat is the room's".to_string(),
            ));
        }
    };
    args.source = match (url, room) {
        (Some(url), None) => Source::Relay {
            url,
            cert_sha256: cert_sha256
                .ok_or_else(|| Invalid("--url needs --cert-sha256".to_string()))?,
            namespace,
        },
        (None, Some(address)) => Source::Room {
            address,
            name,
            ask_token,
        },
        (None, None) => return Err(Invalid("--url or --room is needed".to_string())),
        (Some(_), Some(_)) => return Err(Invalid("--url or --room, not both".to_string())),
    };
    if video.is_some() && args.video_version != VideoVersion::First {
        return Err(Invalid(
            "--video names the track; --video-version chooses one of the catalog".to_string(),
        ));
    }
    args.tracks = match (video, audio) {
        (Some(video), Some(audio)) => Some((video, audio)),
        (None, None) => None,
        _ => {
            return Err(Invalid(
                "--video and --audio are given together, or neither".to_string(),
            ));
        }
    };
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Result<Args, Invalid> {
        parse(line.split_whitespace().map(str::to_string))
    }

    /// The defaults with `source`; with a room, the chat of the room
    /// `a` as `terminal`.
    fn defaults(source: Source) -> Args {
        let chat = matches!(source, Source::Room { .. }).then(|| Chat {
            room: "a".to_string(),
            name: "terminal".to_string(),
            say: None,
            catch_up: 16,
        });
        Args {
            source,
            chat,
            tracks: None,
            video_version: VideoVersion::First,
            buffer_ms: 150,
            buffer_auto: false,
            video_timeout_ms: None,
            answer_wait_ms: 10_000,
            setup_wait_ms: 10_000,
            open_wait_ms: 10_000,
            room_wait_ms: 5_000,
            connect_wait_ms: 10_000,
            rendezvous_ms: None,
            volume: 1.0,
            scale: 1,
            medium: Medium::File,
            fit: Fit::Terminal,
            out: None,
            seconds: None,
            log: None,
            sound: true,
            measure_demo_marks: false,
            clock_skew_ppm: 0,
            cut_sound_s: None,
        }
    }

    #[test]
    fn the_chat_on_tracks_is_of_the_room_or_of_chat_room() {
        let hash = "0f".repeat(32);
        let url = format!("--url https://h:1/ --cert-sha256 {hash}");
        let chat = |room: &str, name: &str| {
            Some(Chat {
                room: room.to_string(),
                name: name.to_string(),
                say: None,
                catch_up: 16,
            })
        };
        // (the flags; the chat, or what is wrong)
        let cases = [
            (url.clone(), Ok(None)),
            (format!("{url} --chat-room r"), Ok(chat("r", "terminal"))),
            (
                format!("{url} --chat-room r --name me"),
                Ok(chat("r", "me")),
            ),
            (
                format!("{url} --chat-room r --say hi"),
                Ok(Some(Chat {
                    room: "r".to_string(),
                    name: "terminal".to_string(),
                    say: Some("hi".to_string()),
                    catch_up: 16,
                })),
            ),
            (
                format!("{url} --chat-room r --chat-catch-up-lines 1000"),
                Ok(Some(Chat {
                    room: "r".to_string(),
                    name: "terminal".to_string(),
                    say: None,
                    catch_up: 1000,
                })),
            ),
            (
                format!("{url} --chat-room r --chat-catch-up-lines 0"),
                Err(Invalid(
                    "--chat-catch-up-lines is a whole number from 1 to 1000, not 0".to_string(),
                )),
            ),
            (
                "--room http://h/rooms/b --name me".to_string(),
                Ok(chat("b", "me")),
            ),
            // An address without the room's name has no chat on tracks.
            ("--room http://h".to_string(), Ok(None)),
            (
                "--room http://h/rooms/b --chat-room r".to_string(),
                Err(Invalid(
                    "--chat-room is for --url; with --room the chat is the room's".to_string(),
                )),
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(parsed(&line).map(|args| args.chat), expected, "{line}");
        }
    }

    fn room() -> Source {
        Source::Room {
            address: "http://h/rooms/a".to_string(),
            name: "terminal".to_string(),
            ask_token: true,
        }
    }

    #[test]
    fn the_flags_are_read() {
        let hash = "0f".repeat(32);
        assert_eq!(
            parsed(&format!("--url https://h:1/ --cert-sha256 {hash}")),
            Ok(defaults(Source::Relay {
                url: "https://h:1/".to_string(),
                cert_sha256: [0x0f; 32],
                namespace: "xmoq/demo".to_string(),
            }))
        );
        assert_eq!(parsed("--room http://h/rooms/a"), Ok(defaults(room())));
        assert_eq!(
            parsed(
                "--room http://h/rooms/a --name me --say hello --token no --video v --audio a \
                 --buffer-ms 150 --video-timeout-ms 100 --answer-wait-ms 1500 --setup-wait-ms 2500 --open-wait-ms 3500 --room-wait-ms 600 --connect-wait-ms 700 --rendezvous-timeout-ms 4500 --volume 0.3 --scale 2 --medium direct --fit 960x540 --out o \
                 --seconds 20 --log l --sound off --measure-demo-marks on --cut-sound-s 5"
            ),
            Ok(Args {
                source: Source::Room {
                    address: "http://h/rooms/a".to_string(),
                    name: "me".to_string(),
                    ask_token: false,
                },
                chat: Some(Chat {
                    room: "a".to_string(),
                    name: "me".to_string(),
                    say: Some("hello".to_string()),
                    catch_up: 16,
                }),
                tracks: Some(("v".to_string(), "a".to_string())),
                video_version: VideoVersion::First,
                buffer_ms: 150,
                buffer_auto: false,
                video_timeout_ms: Some(100),
                answer_wait_ms: 1500,
                setup_wait_ms: 2500,
                open_wait_ms: 3500,
                room_wait_ms: 600,
                connect_wait_ms: 700,
                rendezvous_ms: Some(4500),
                volume: 0.3,
                scale: 2,
                medium: Medium::Direct,
                fit: Fit::Most(960, 540),
                out: Some("o".to_string()),
                seconds: Some(20),
                log: Some("l".to_string()),
                sound: false,
                measure_demo_marks: true,
                clock_skew_ppm: 0,
                cut_sound_s: Some(5),
            })
        );
        assert_eq!(
            parsed("--room http://h/rooms/a --fit none").map(|args| args.fit),
            Ok(Fit::AsItIs)
        );
        // (the value of `--video-version`; what it is read as)
        let versions = [
            ("first", VideoVersion::First),
            (
                "name:video-low",
                VideoVersion::Named("video-low".to_string()),
            ),
            ("height:360", VideoVersion::HeightAtMost(360)),
            ("bitrate:500000", VideoVersion::BitrateAtMost(500_000)),
        ];
        for (value, expected) in versions {
            assert_eq!(
                parsed(&format!("--room http://h/rooms/a --video-version {value}"))
                    .map(|args| args.video_version),
                Ok(expected),
                "{value}"
            );
        }
        assert_eq!(
            parsed("--room http://h/rooms/a --buffer-ms 300 --buffer-ms auto")
                .map(|args| (args.buffer_ms, args.buffer_auto)),
            Ok((150, true))
        );
        assert_eq!(
            parsed("--room http://h/rooms/a --buffer-ms auto --buffer-ms 300")
                .map(|args| (args.buffer_ms, args.buffer_auto)),
            Ok((300, false))
        );
        // 0 is the draft's way of asking for no timeout.
        assert_eq!(
            parsed("--room http://h/rooms/a --video-timeout-ms 0")
                .map(|args| args.video_timeout_ms),
            Ok(None)
        );
    }

    #[test]
    fn flags_that_are_wrong_are_turned_down() {
        let hash = "0f".repeat(32);
        // (the flags; what is said of them)
        let cases = [
            (String::new(), "--url or --room is needed"),
            ("--url https://h/".to_string(), "--url needs --cert-sha256"),
            (
                format!("--url https://h/ --cert-sha256 {hash} --room http://h/rooms/a"),
                "--url or --room, not both",
            ),
            (
                "--url https://h/ --cert-sha256 0f".to_string(),
                "--cert-sha256: the hash of a certificate is 64 hex digits",
            ),
            ("--room".to_string(), "--room needs a value"),
            ("--room r --nothing 1".to_string(), "unknown flag --nothing"),
            (
                "--room r --video v".to_string(),
                "--video and --audio are given together, or neither",
            ),
            (
                "--room r --buffer-ms soon".to_string(),
                "--buffer-ms soon: not a number it takes",
            ),
            (
                "--room r --buffer-ms -1".to_string(),
                "--buffer-ms -1: not a number it takes",
            ),
            (
                "--room r --buffer-ms 10001".to_string(),
                "--buffer-ms 10001: 10000 at most",
            ),
            (
                "--room r --clock-skew-ppm -1000000".to_string(),
                "--clock-skew-ppm -1000000: from -999999 to 1000000",
            ),
            (
                "--room r --volume 1.5".to_string(),
                "--volume 1.5: from 0 to 1",
            ),
            (
                "--room r --volume NaN".to_string(),
                "--volume NaN: from 0 to 1",
            ),
            ("--room r --scale 0".to_string(), "--scale 0: 1 or more"),
            (
                "--room r --video-version low".to_string(),
                "--video-version low: first, name:<track>, height:<n> or bitrate:<n>",
            ),
            (
                "--room r --video-version name:".to_string(),
                "--video-version name:: first, name:<track>, height:<n> or bitrate:<n>",
            ),
            (
                "--room r --video-version height:tall".to_string(),
                "--video-version height:tall: not a number it takes",
            ),
            (
                "--room r --video v --audio a --video-version height:360".to_string(),
                "--video names the track; --video-version chooses one of the catalog",
            ),
            (
                "--room r --medium paper".to_string(),
                "--medium paper: file or direct",
            ),
            (
                "--room r --fit 0x10".to_string(),
                "--fit 0x10: terminal, none or <width>x<height>",
            ),
            (
                "--room r --fit wide".to_string(),
                "--fit wide: terminal, none or <width>x<height>",
            ),
            (
                "--room r --token maybe".to_string(),
                "--token maybe: yes or no",
            ),
            (
                "--room r --sound loud".to_string(),
                "--sound loud: on or off",
            ),
            (
                "--room r --measure-demo-marks 1".to_string(),
                "--measure-demo-marks 1: on or off",
            ),
        ];
        for (flags, said) in cases {
            assert_eq!(parsed(&flags), Err(Invalid(said.to_string())), "{flags}");
        }
    }
}
