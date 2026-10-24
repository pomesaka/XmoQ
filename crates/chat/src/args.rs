//! The flags of the gatherer, and the limits of the chat they set.

/// How the gatherer is started without a flag it does not know.
pub const USAGE: &str = "\
xmoq-chat --url <https://host:port/> --cert-sha256 <64 hex digits> --store <file>
          [--room <room>]... [--text-bytes <n>] [--lines-per-second <n>] [--fetch-lines <n>]

--store             the SQLite file of the history, this gatherer's alone; made where it is not there
--room              a room to gather, up to 8; every room without it
--text-bytes        the most bytes of UTF-8 the text of a line may have (512)
--lines-per-second  the most lines one person may say in any second (5)
--fetch-lines       the most lines one FETCH gets (1000)";

pub use xmoq_chat_tracks::limits::{
    FETCH_LINES_MOST, LINES_PER_SECOND_MOST, Limits, TEXT_BYTES_MOST,
};

/// The most bytes the gatherer holds of one stream of `said` before it has
/// decoded the Object on it; a stream with more is stopped, and its line is
/// lost. Not a flag: it bounds what any participant can have the gatherer
/// hold, 64 KiB a stream. **Provisional.**
pub const PAYLOAD_MOST: usize = 64 * 1024;

/// The most `--room` flags: as many `SUBSCRIBE_NAMESPACE` requests as the own
/// relay lets one session have open (`MAX_NAMESPACE_WATCHES_PER_SESSION` of
/// `crates/relay-core/src/discovery.rs`, 8, decided on 2026-10-24: Q45). WHY refused at
/// the start and not left to the relay: a ninth request would be refused
/// with `EXCESSIVE_LOAD` for good, and its room would never be gathered
/// while the gatherer looked as if it ran.
pub const ROOMS_MOST: usize = 8;

/// Which rooms the gatherer takes, publishes and answers FETCH for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rooms {
    /// Every room: one `SUBSCRIBE_NAMESPACE` for `xmoq/chat`.
    Every,
    /// These (`--room`), one `SUBSCRIBE_NAMESPACE` for `xmoq/chat/<room>`
    /// each, in the order given (Q48, decided on 2026-10-24: a gatherer for
    /// each room).
    These(Vec<String>),
}

impl Rooms {
    /// Whether the gatherer takes this room.
    #[must_use]
    pub fn has(&self, room: &str) -> bool {
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
}

/// Reads 64 hexadecimal digits.
fn certificate_hash(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut hash = [0; 32];
    for (index, byte) in hash.iter_mut().enumerate() {
        let digits = text.get(index * 2..index * 2 + 2)?;
        *byte = u8::from_str_radix(digits, 16).ok()?;
    }
    Some(hash)
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

/// Reads the flags.
///
/// # Errors
///
/// What is wrong with them.
pub fn parse(mut flags: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut url, mut cert_sha256, mut store) = (None, None, None);
    let mut limits = Limits::default();
    let mut rooms: Vec<String> = Vec::new();
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
                if xmoq_chat_tracks::names::room_namespace(&value).is_err() {
                    return Err(format!("--room {value:?} is no field of a namespace"));
                }
                if rooms.contains(&value) {
                    return Err(format!("--room {value:?} is given twice"));
                }
                rooms.push(value);
            }
            "--text-bytes" => limits.text_bytes = number_within(&flag, &value, TEXT_BYTES_MOST)?,
            "--lines-per-second" => {
                limits.lines_per_second = number_within(&flag, &value, LINES_PER_SECOND_MOST)?;
            }
            "--fetch-lines" => limits.fetch_lines = number_within(&flag, &value, FETCH_LINES_MOST)?,
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
        }
    }

    fn these(rooms: &[&str]) -> Args {
        Args {
            rooms: Rooms::These(rooms.iter().map(ToString::to_string).collect()),
            ..with(Limits::default())
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
            "--room", "1", "--room", "2", "--room", "3", "--room", "4", "--room", "5", "--room",
            "6", "--room", "7", "--room", "8",
        ];
        let nine: Vec<&str> = eight.iter().copied().chain(["--room", "9"]).collect();
        let cases: [(&str, &[&str], Result<Args, String>); 6] = [
            ("none: every room", &[], Ok(with(Limits::default()))),
            ("one", &["--room", "r"], Ok(these(&["r"]))),
            ("eight, the most", &eight, Ok(these(&["1", "2", "3", "4", "5", "6", "7", "8"]))),
            (
                "nine",
                &nine,
                Err("--room is given 9 times: a session at the own relay may have 8 SUBSCRIBE_NAMESPACE requests; run another gatherer, with a store of its own, for the rest".to_string()),
            ),
            ("twice the same", &["--room", "r", "--room", "r"], Err("--room \"r\" is given twice".to_string())),
            ("an empty one", &["--room", ""], Err("--room \"\" is no field of a namespace".to_string())),
        ];
        for (case, extra, expected) in cases {
            let flags: Vec<&str> = base.iter().chain(extra).copied().collect();
            assert_eq!(args(&flags), expected, "{case}");
        }
    }

    #[test]
    fn which_rooms_are_taken() {
        let cases: [(&str, Rooms, &str, bool); 4] = [
            ("every room", Rooms::Every, "r", true),
            (
                "a room given",
                Rooms::These(vec!["r".to_string(), "s".to_string()]),
                "s",
                true,
            ),
            (
                "a room not given",
                Rooms::These(vec!["r".to_string()]),
                "s",
                false,
            ),
            (
                "a prefix of a room given",
                Rooms::These(vec!["room".to_string()]),
                "roo",
                false,
            ),
        ];
        for (case, rooms, room, expected) in cases {
            assert_eq!(rooms.has(room), expected, "{case}");
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
        let cases: [(&str, &[&str], Result<Args, String>); 9] = [
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
                ],
                Ok(with(Limits {
                    text_bytes: 100,
                    lines_per_second: 2,
                    fetch_lines: 3,
                })),
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
