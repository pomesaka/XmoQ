//! The flags of the gatherer, and the limits of the chat they set.

/// How the gatherer is started without a flag it does not know.
pub const USAGE: &str = "\
xmoq-chat --url <https://host:port/> --cert-sha256 <64 hex digits> --store <file>
          [--room <kind>/<room>]... [--text-bytes <n>] [--lines-per-second <n>] [--fetch-lines <n>]
          [--ask-again-times <n>] [--ask-again-ms <ms>] [--auth-key-hex <hex>]
          [--reaction-batch-ms <ms>] [--reactions-per-second <n>] [--reaction-kinds <n>]

--store             the SQLite file of the history, this gatherer's alone; made where it is not there
--room              a room to gather, broadcast/<room> or meeting/<room>, up to 8; every room of both
                    kinds without it
--text-bytes        the most bytes of UTF-8 the text of a line may have (512; 1 to 8192)
--lines-per-second  the most lines one person may say in any second; the line over it is dropped (5; 1 to 1000)
--fetch-lines       the most lines one FETCH gets, the latest of a longer range (1000; 1 to 100000)
--ask-again-times   how many times in a row a participant whose subscription ended is asked again
                    without one that was established (30; 0 never; up to 10000)
--ask-again-ms      the pause before it is, after an end or a refusal that names no time or 0
                    (1000; 100 to 60000)
--reaction-batch-ms     the while a room's reactions are counted over before they are published on
                        `reactions` (100; 10 to 60000)
--reactions-per-second  the most reactions one person may send in any second; the one over it is
                        dropped (5; 1 to 1000)
--reaction-kinds        the most emoji one while of a room counts; a reaction with one more is
                        dropped (16; 1 to 256)
--auth-key-hex      the key the relay checks tokens with; the gatherer makes its own tokens (where the relay shares its key with it)";

use xmoq_chat_tracks::names::Room;
use xmoq_moqt_io::wt::certificate_hash;

use crate::said_again::{PAUSE_MS_LEAST, PAUSE_MS_MOST, Settings, TIMES_MOST};

pub use xmoq_chat_tracks::limits::{
    FETCH_LINES_MOST, LINES_PER_SECOND_MOST, Limits, REACTION_BATCH_MS_LEAST,
    REACTION_BATCH_MS_MOST, REACTION_KINDS_MOST, REACTIONS_PER_SECOND_MOST, TEXT_BYTES_MOST,
};

/// The most bytes the gatherer holds of one stream of `said` before it has
/// decoded the Object on it; a stream with more is stopped, and its line is
/// lost. Not a flag: it bounds what any participant can have the gatherer
/// hold, 64 KiB a stream. **Provisional.**
pub const PAYLOAD_MOST: usize = 64 * 1024;

/// The most `--room` flags: as many `SUBSCRIBE_NAMESPACE` requests as the own
/// relay lets one session have open (`MAX_NAMESPACE_SUBSCRIPTIONS_PER_SESSION`
/// of `crates/relay-core/src/discovery.rs`, 8, decided on 2026-10-24). WHY
/// refused at the start and not left to the relay: a ninth request would be
/// refused with `EXCESSIVE_LOAD` for good, and its room would never be gathered
/// while the gatherer looked as if it ran.
pub const ROOMS_MOST: usize = 8;

/// Which rooms the gatherer takes, publishes and answers FETCH for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rooms {
    /// Every room of both kinds: one `SUBSCRIBE_NAMESPACE` for `xmoq/chat`.
    Every,
    /// These (`--room`), one `SUBSCRIBE_NAMESPACE` for
    /// `xmoq/chat/<kind>/<room>` each, in the order given (decided on
    /// 2026-10-24: a gatherer for each room).
    These(Vec<Room>),
}

impl Rooms {
    /// Whether the gatherer takes this room.
    #[must_use]
    pub fn has(&self, room: &Room) -> bool {
        match self {
            Self::Every => true,
            Self::These(rooms) => rooms.iter().any(|each| each == room),
        }
    }
}

/// What the gatherer was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `--url`: the relay.
    pub url: String,
    /// `--cert-sha256`: the hash of the relay's certificate.
    pub cert_sha256: [u8; 32],
    /// `--store`: the file of the history.
    pub store: String,
    /// The limits.
    pub limits: Limits,
    /// `--room`.
    pub rooms: Rooms,
    /// `--auth-key-hex`: the key the relay checks the tokens of the room
    /// with (its flag of that name), with which the gatherer makes tokens
    /// of its own for its SETUP and its requests: for a relay that shares
    /// its key with the gatherer (provisional). A gatherer of a room's real
    /// service would get its tokens from where the room's are made.
    pub auth_key: Option<Vec<u8>>,
    /// `--ask-again-times`, `--ask-again-ms`.
    pub ask_again: Settings,
}

/// A number between 1 and `most`.
fn number_within<T>(flag: &str, value: &str, most: T) -> Result<T, String>
where
    T: std::str::FromStr + PartialOrd + From<u8> + std::fmt::Display + Copy,
{
    match value.parse::<T>() {
        Ok(number) if number >= T::from(1) && number <= most => Ok(number),
        _ => Err(format!(
            "{flag} is a number from 1 to {most}, not {value:?}"
        )),
    }
}

/// A number between `least` and `most`.
fn number_from<T>(flag: &str, value: &str, least: T, most: T) -> Result<T, String>
where
    T: std::str::FromStr + PartialOrd + std::fmt::Display + Copy,
{
    match value.parse::<T>() {
        Ok(number) if number >= least && number <= most => Ok(number),
        _ => Err(format!(
            "{flag} is a number from {least} to {most}, not {value:?}"
        )),
    }
}

/// Reads the flags.
///
/// # Errors
///
/// What is wrong with them.
pub fn parse(mut flags: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut url, mut cert_sha256, mut store) = (None, None, None);
    let mut limits = Limits::default();
    let mut rooms: Vec<Room> = Vec::new();
    let mut auth_key = None;
    let mut ask_again = Settings::default();
    while let Some(flag) = flags.next() {
        let value = flags
            .next()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--url" => url = Some(value),
            "--cert-sha256" => {
                cert_sha256 = Some(
                    certificate_hash(&value)
                        .ok_or_else(|| format!("--cert-sha256 is 64 hex digits, not {value:?}"))?,
                );
            }
            "--store" => store = Some(value),
            "--room" => {
                let room = Room::parse(&value)
                    .filter(|room| xmoq_chat_tracks::names::room_namespace(room).is_ok())
                    .ok_or_else(|| {
                        format!("--room {value:?} is no broadcast/<room> or meeting/<room>")
                    })?;
                if rooms.contains(&room) {
                    return Err(format!("--room {value:?} is given twice"));
                }
                rooms.push(room);
            }
            "--text-bytes" => limits.text_bytes = number_within(&flag, &value, TEXT_BYTES_MOST)?,
            "--lines-per-second" => {
                limits.lines_per_second = number_within(&flag, &value, LINES_PER_SECOND_MOST)?;
            }
            "--fetch-lines" => limits.fetch_lines = number_within(&flag, &value, FETCH_LINES_MOST)?,
            "--reaction-batch-ms" => {
                limits.reaction_batch_ms = number_from(
                    &flag,
                    &value,
                    REACTION_BATCH_MS_LEAST,
                    REACTION_BATCH_MS_MOST,
                )?;
            }
            "--reactions-per-second" => {
                limits.reactions_per_second =
                    number_within(&flag, &value, REACTIONS_PER_SECOND_MOST)?;
            }
            "--reaction-kinds" => {
                limits.reaction_kinds = number_within(&flag, &value, REACTION_KINDS_MOST)?;
            }
            "--ask-again-times" => ask_again.times = number_from(&flag, &value, 0, TIMES_MOST)?,
            "--ask-again-ms" => {
                ask_again.pause_ms = number_from(&flag, &value, PAUSE_MS_LEAST, PAUSE_MS_MOST)?;
            }
            "--auth-key-hex" => {
                auth_key = Some(
                    xmoq_room_token::key_of_hex(&value)
                        .ok_or_else(|| format!("--auth-key-hex is hex digits, not {value:?}"))?,
                );
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if rooms.len() > ROOMS_MOST {
        return Err(format!(
            "--room is given {} times: a session at the own relay may have {ROOMS_MOST} SUBSCRIBE_NAMESPACE requests; run another gatherer, with a store of its own, for the rest",
            rooms.len()
        ));
    }
    Ok(Args {
        url: url.ok_or("--url is needed")?,
        cert_sha256: cert_sha256.ok_or("--cert-sha256 is needed")?,
        store: store.ok_or("--store is needed")?,
        limits,
        rooms: if rooms.is_empty() {
            Rooms::Every
        } else {
            Rooms::These(rooms)
        },
        auth_key,
        ask_again,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";

    fn args(flags: &[&str]) -> Result<Args, String> {
        parse(flags.iter().map(ToString::to_string))
    }

    fn with(limits: Limits) -> Args {
        let mut cert_sha256 = [0; 32];
        for (index, byte) in cert_sha256.iter_mut().enumerate() {
            *byte = [
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ][index % 16];
        }
        Args {
            url: "https://h:1/".to_string(),
            cert_sha256,
            store: "s".to_string(),
            limits,
            rooms: Rooms::Every,
            auth_key: None,
            ask_again: Settings::default(),
        }
    }

    fn these(rooms: &[&str]) -> Args {
        Args {
            rooms: Rooms::These(rooms.iter().map(|room| broadcast(room)).collect()),
            ..with(Limits::default())
        }
    }

    /// The room of a broadcast that `name` names.
    fn broadcast(name: &str) -> Room {
        Room {
            kind: xmoq_chat_tracks::names::Kind::Broadcast,
            name: name.to_string(),
        }
    }

    #[test]
    fn the_rooms_given_are_the_ones_gathered() {
        let base = [
            "--url",
            "https://h:1/",
            "--cert-sha256",
            HASH,
            "--store",
            "s",
        ];
        let eight = [
            "--room",
            "broadcast/1",
            "--room",
            "broadcast/2",
            "--room",
            "broadcast/3",
            "--room",
            "broadcast/4",
            "--room",
            "broadcast/5",
            "--room",
            "broadcast/6",
            "--room",
            "broadcast/7",
            "--room",
            "broadcast/8",
        ];
        let nine: Vec<&str> = eight
            .iter()
            .copied()
            .chain(["--room", "broadcast/9"])
            .collect();
        let meeting = Args {
            rooms: Rooms::These(vec![Room {
                kind: xmoq_chat_tracks::names::Kind::Meeting,
                name: "m".to_string(),
            }]),
            ..with(Limits::default())
        };
        let cases: [(&str, &[&str], Result<Args, String>); 9] = [
            ("none: every room", &[], Ok(with(Limits::default()))),
            ("one", &["--room", "broadcast/r"], Ok(these(&["r"]))),
            ("one of a meeting", &["--room", "meeting/m"], Ok(meeting)),
            ("eight, the most", &eight, Ok(these(&["1", "2", "3", "4", "5", "6", "7", "8"]))),
            (
                "nine",
                &nine,
                Err("--room is given 9 times: a session at the own relay may have 8 SUBSCRIBE_NAMESPACE requests; run another gatherer, with a store of its own, for the rest".to_string()),
            ),
            ("twice the same", &["--room", "broadcast/r", "--room", "broadcast/r"], Err("--room \"broadcast/r\" is given twice".to_string())),
            ("an empty one", &["--room", ""], Err("--room \"\" is no broadcast/<room> or meeting/<room>".to_string())),
            ("one without its kind", &["--room", "r"], Err("--room \"r\" is no broadcast/<room> or meeting/<room>".to_string())),
            ("a kind there is not", &["--room", "talk/r"], Err("--room \"talk/r\" is no broadcast/<room> or meeting/<room>".to_string())),
        ];
        for (case, extra, expected) in cases {
            let flags: Vec<&str> = base.iter().chain(extra).copied().collect();
            assert_eq!(args(&flags), expected, "{case}");
        }
    }

    #[test]
    fn which_rooms_are_taken() {
        let meeting = |name: &str| Room {
            kind: xmoq_chat_tracks::names::Kind::Meeting,
            name: name.to_string(),
        };
        let cases: [(&str, Rooms, Room, bool); 5] = [
            ("every room", Rooms::Every, broadcast("r"), true),
            (
                "a room given",
                Rooms::These(vec![broadcast("r"), broadcast("s")]),
                broadcast("s"),
                true,
            ),
            (
                "a room not given",
                Rooms::These(vec![broadcast("r")]),
                broadcast("s"),
                false,
            ),
            (
                "a prefix of a room given",
                Rooms::These(vec![broadcast("room")]),
                broadcast("roo"),
                false,
            ),
            (
                "the same name of the other kind",
                Rooms::These(vec![broadcast("r")]),
                meeting("r"),
                false,
            ),
        ];
        for (case, rooms, room, expected) in cases {
            assert_eq!(rooms.has(&room), expected, "{case}");
        }
    }

    /// What `--auth-key-hex` comes to.
    type Key = Result<Option<Vec<u8>>, String>;

    #[test]
    fn the_key_of_the_relay_is_hex_digits() {
        let base = [
            "--url",
            "https://h:1/",
            "--cert-sha256",
            HASH,
            "--store",
            "s",
        ];
        let cases: [(&str, Key); 4] = [
            ("00ff10", Ok(Some(vec![0x00, 0xff, 0x10]))),
            (
                "0",
                Err("--auth-key-hex is hex digits, not \"0\"".to_string()),
            ),
            (
                "zz",
                Err("--auth-key-hex is hex digits, not \"zz\"".to_string()),
            ),
            (
                "",
                Err("--auth-key-hex is hex digits, not \"\"".to_string()),
            ),
        ];
        for (hex, expected) in cases {
            let flags: Vec<&str> = base
                .iter()
                .copied()
                .chain(["--auth-key-hex", hex])
                .collect();
            assert_eq!(args(&flags).map(|args| args.auth_key), expected, "{hex}");
        }
    }

    #[test]
    fn the_flags_set_the_limits_within_their_bounds() {
        let base = [
            "--url",
            "https://h:1/",
            "--cert-sha256",
            HASH,
            "--store",
            "s",
        ];
        let cases: [(&str, &[&str], Result<Args, String>); 12] = [
            ("the defaults", &[], Ok(with(Limits::default()))),
            (
                "every limit",
                &[
                    "--text-bytes",
                    "100",
                    "--lines-per-second",
                    "2",
                    "--fetch-lines",
                    "3",
                    "--reaction-batch-ms",
                    "250",
                    "--reactions-per-second",
                    "3",
                    "--reaction-kinds",
                    "8",
                ],
                Ok(with(Limits {
                    text_bytes: 100,
                    lines_per_second: 2,
                    fetch_lines: 3,
                    reaction_batch_ms: 250,
                    reactions_per_second: 3,
                    reaction_kinds: 8,
                })),
            ),
            (
                "a while too short",
                &["--reaction-batch-ms", "9"],
                Err("--reaction-batch-ms is a number from 10 to 60000, not \"9\"".to_string()),
            ),
            (
                "no reactions at all",
                &["--reactions-per-second", "0"],
                Err("--reactions-per-second is a number from 1 to 1000, not \"0\"".to_string()),
            ),
            (
                "too many kinds",
                &["--reaction-kinds", "257"],
                Err("--reaction-kinds is a number from 1 to 256, not \"257\"".to_string()),
            ),
            (
                "the largest text",
                &["--text-bytes", "8192"],
                Ok(with(Limits {
                    text_bytes: 8_192,
                    ..Limits::default()
                })),
            ),
            (
                "a text too large",
                &["--text-bytes", "8193"],
                Err("--text-bytes is a number from 1 to 8192, not \"8193\"".to_string()),
            ),
            (
                "no lines at all",
                &["--lines-per-second", "0"],
                Err("--lines-per-second is a number from 1 to 1000, not \"0\"".to_string()),
            ),
            (
                "a FETCH of none",
                &["--fetch-lines", "0"],
                Err("--fetch-lines is a number from 1 to 100000, not \"0\"".to_string()),
            ),
            (
                "not a number",
                &["--fetch-lines", "-1"],
                Err("--fetch-lines is a number from 1 to 100000, not \"-1\"".to_string()),
            ),
            (
                "a flag it does not know",
                &["--rooms", "3"],
                Err("unknown flag --rooms".to_string()),
            ),
            (
                "a flag without its value",
                &["--fetch-lines"],
                Err("--fetch-lines needs a value".to_string()),
            ),
        ];
        for (case, extra, expected) in cases {
            let flags: Vec<&str> = base.iter().chain(extra).copied().collect();
            assert_eq!(args(&flags), expected, "{case}");
        }
    }

    #[test]
    fn the_flags_set_when_a_participant_is_asked_again() {
        let base = [
            "--url",
            "https://h:1/",
            "--cert-sha256",
            HASH,
            "--store",
            "s",
        ];
        let asked = |times, pause_ms| {
            Ok(Args {
                ask_again: Settings { times, pause_ms },
                ..with(Limits::default())
            })
        };
        let cases: [(&str, &[&str], Result<Args, String>); 8] = [
            ("the defaults", &[], asked(30, 1000)),
            (
                "both",
                &["--ask-again-times", "3", "--ask-again-ms", "2000"],
                asked(3, 2000),
            ),
            ("never", &["--ask-again-times", "0"], asked(0, 1000)),
            (
                "the most times",
                &["--ask-again-times", "10000"],
                asked(10_000, 1000),
            ),
            (
                "too many times",
                &["--ask-again-times", "10001"],
                Err("--ask-again-times is a number from 0 to 10000, not \"10001\"".to_string()),
            ),
            (
                "the shortest pause",
                &["--ask-again-ms", "100"],
                asked(30, 100),
            ),
            (
                "a pause too short",
                &["--ask-again-ms", "99"],
                Err("--ask-again-ms is a number from 100 to 60000, not \"99\"".to_string()),
            ),
            (
                "a pause too long",
                &["--ask-again-ms", "60001"],
                Err("--ask-again-ms is a number from 100 to 60000, not \"60001\"".to_string()),
            ),
        ];
        for (case, extra, expected) in cases {
            let flags: Vec<&str> = base.iter().chain(extra).copied().collect();
            assert_eq!(args(&flags), expected, "{case}");
        }
    }

    #[test]
    fn what_has_to_be_given() {
        let cases: [(&str, &[&str], &str); 4] = [
            (
                "no url",
                &["--cert-sha256", HASH, "--store", "s"],
                "--url is needed",
            ),
            (
                "no hash",
                &["--url", "u", "--store", "s"],
                "--cert-sha256 is needed",
            ),
            (
                "no store",
                &["--url", "u", "--cert-sha256", HASH],
                "--store is needed",
            ),
            (
                "a short hash",
                &["--url", "u", "--cert-sha256", "0011", "--store", "s"],
                "--cert-sha256 is 64 hex digits, not \"0011\"",
            ),
        ];
        for (case, flags, expected) in cases {
            assert_eq!(args(flags), Err(expected.to_string()), "{case}");
        }
    }
}
