//! The flags of the pet.

use xmoq_moqt_io::wt::certificate_hash;
use xmoq_subscriber::kept::{Keeping, TIMES, TIMES_MOST, Tries};

/// How the pet is started without a flag it does not know.
pub const USAGE: &str = "\
xmoq-pet --url <https://host:port/> --cert-sha256 <64 hex digits>
         [--auth-key-hex <hex>] [--ask-again-times <n>] [--ask-again-ms <ms>]
         [--decisions-url <url>] [--decisions-per-minute <n>]

--auth-key-hex          the key the relay checks tokens with; the pet makes its own tokens
                        (where the relay shares its key with it)
--ask-again-times       how many times in a row a room's `garden` whose subscription ended, or
                        the pet's announcement of the room that ended, is asked again
                        without one that was established (30; 0 never; up to 10000)
--ask-again-ms          the pause before it is, after an end or a refusal that names no time
                        or 0 (1000; 100 to 60000)
--decisions-url         the decisions API that decides what the pet does, such as
                        http://127.0.0.1:3333/v1/decisions; its bearer token is the
                        environment variable DECISIONS_API_TOKEN. Without it, the table does
--decisions-per-minute  the most calls of the decisions API in any minute; a batch over it is
                        decided by the table (10; 0 never calls; up to 600)";

/// The pause after an end by default: the rule's.
pub const PAUSE_MS: u64 = xmoq_again::ASK_AGAIN_MS;

/// The least `--ask-again-ms`: the rule's shortest pause after a refusal.
pub const PAUSE_MS_LEAST: u64 = xmoq_again::SHORTEST_MS;

/// The most `--ask-again-ms`.
pub const PAUSE_MS_MOST: u64 = 60_000;

/// `--decisions-per-minute` by default. WHY 10: a call of four questions
/// makes four calls upstream (the API's owner, 2026-10-28), and a batch
/// every six seconds is about as often as a few people play with the pet.
/// **Provisional.**
pub const DECISIONS_PER_MINUTE: usize = 10;

/// The most `--decisions-per-minute`: ten a second.
pub const DECISIONS_PER_MINUTE_MOST: usize = 600;

/// When a room's `garden`, or the pet's announcement in the room, is asked
/// for again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AskAgain {
    /// `--ask-again-times`.
    pub times: u32,
    /// `--ask-again-ms`.
    pub pause_ms: u64,
}

impl Default for AskAgain {
    fn default() -> Self {
        Self {
            times: TIMES,
            pause_ms: PAUSE_MS,
        }
    }
}

impl AskAgain {
    /// How `garden` and the announcement are kept (`kept::decide` of
    /// `xmoq-subscriber`), as the chat's gatherer keeps `said`. WHY not
    /// optional: the garden that announces a room publishes `garden`.
    #[must_use]
    pub fn keeping(self) -> Keeping {
        Keeping {
            again_after_failure: true,
            times: Some(self.times),
            optional: false,
        }
    }

    /// What is kept of a room followed anew, or whose NAMESPACE came again.
    #[must_use]
    pub fn tries(self) -> Tries {
        Tries::new(xmoq_again::Pauses {
            after_end_ms: self.pause_ms,
            ..xmoq_again::Pauses::default()
        })
    }
}

/// What the pet was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `--url`: the relay.
    pub url: String,
    /// `--cert-sha256`: the hash of the relay's certificate.
    pub cert_sha256: [u8; 32],
    /// `--auth-key-hex`: the key of the relay, with which the pet makes
    /// tokens of its own, as the chat's gatherer does (provisional).
    pub auth_key: Option<Vec<u8>>,
    /// `--ask-again-times`, `--ask-again-ms`.
    pub ask_again: AskAgain,
    /// `--decisions-url`.
    pub decisions_url: Option<String>,
    /// `--decisions-per-minute`.
    pub decisions_per_minute: usize,
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
    let (mut url, mut cert_sha256, mut auth_key, mut decisions_url) = (None, None, None, None);
    let mut ask_again = AskAgain::default();
    let mut decisions_per_minute = DECISIONS_PER_MINUTE;
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
            "--auth-key-hex" => {
                auth_key = Some(
                    xmoq_room_token::key_of_hex(&value)
                        .ok_or_else(|| format!("--auth-key-hex is hex digits, not {value:?}"))?,
                );
            }
            "--ask-again-times" => ask_again.times = number_from(&flag, &value, 0, TIMES_MOST)?,
            "--ask-again-ms" => {
                ask_again.pause_ms = number_from(&flag, &value, PAUSE_MS_LEAST, PAUSE_MS_MOST)?;
            }
            "--decisions-url" => {
                if !(value.starts_with("http://") || value.starts_with("https://")) {
                    return Err(format!(
                        "--decisions-url is an http:// or https:// URL, not {value:?}"
                    ));
                }
                decisions_url = Some(value);
            }
            "--decisions-per-minute" => {
                decisions_per_minute = number_from(&flag, &value, 0, DECISIONS_PER_MINUTE_MOST)?;
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Args {
        url: url.ok_or("--url is needed")?,
        cert_sha256: cert_sha256.ok_or("--cert-sha256 is needed")?,
        auth_key,
        ask_again,
        decisions_url,
        decisions_per_minute,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";

    fn args(extra: &[&str]) -> Result<Args, String> {
        parse(
            ["--url", "https://h:1/", "--cert-sha256", HASH]
                .iter()
                .chain(extra)
                .map(ToString::to_string),
        )
    }

    fn base() -> Args {
        Args {
            url: "https://h:1/".to_string(),
            cert_sha256: certificate_hash(HASH).unwrap(),
            auth_key: None,
            ask_again: AskAgain::default(),
            decisions_url: None,
            decisions_per_minute: 10,
        }
    }

    #[test]
    fn the_flags_and_their_bounds() {
        let cases: [(&str, &[&str], Result<Args, String>); 12] = [
            ("the defaults", &[], Ok(base())),
            (
                "the API",
                &[
                    "--decisions-url",
                    "http://127.0.0.1:3333/v1/decisions",
                    "--decisions-per-minute",
                    "0",
                ],
                Ok(Args {
                    decisions_url: Some("http://127.0.0.1:3333/v1/decisions".to_string()),
                    decisions_per_minute: 0,
                    ..base()
                }),
            ),
            (
                "an API that is no http URL",
                &["--decisions-url", "127.0.0.1:3333"],
                Err(
                    "--decisions-url is an http:// or https:// URL, not \"127.0.0.1:3333\""
                        .to_string(),
                ),
            ),
            (
                "too many calls",
                &["--decisions-per-minute", "601"],
                Err("--decisions-per-minute is a number from 0 to 600, not \"601\"".to_string()),
            ),
            (
                "the key",
                &["--auth-key-hex", "00ff"],
                Ok(Args {
                    auth_key: Some(vec![0x00, 0xff]),
                    ..base()
                }),
            ),
            (
                "a key that is no hex",
                &["--auth-key-hex", "zz"],
                Err("--auth-key-hex is hex digits, not \"zz\"".to_string()),
            ),
            (
                "asked again",
                &["--ask-again-times", "0", "--ask-again-ms", "100"],
                Ok(Args {
                    ask_again: AskAgain {
                        times: 0,
                        pause_ms: 100,
                    },
                    ..base()
                }),
            ),
            (
                "a pause too short",
                &["--ask-again-ms", "99"],
                Err("--ask-again-ms is a number from 100 to 60000, not \"99\"".to_string()),
            ),
            (
                "too many times",
                &["--ask-again-times", "10001"],
                Err("--ask-again-times is a number from 0 to 10000, not \"10001\"".to_string()),
            ),
            (
                "the token is no flag",
                &["--decisions-token", "x"],
                Err("unknown flag --decisions-token".to_string()),
            ),
            (
                "a flag without its value",
                &["--decisions-url"],
                Err("--decisions-url needs a value".to_string()),
            ),
            (
                "a short hash",
                &["--cert-sha256", "0011"],
                Err("--cert-sha256 is 64 hex digits, not \"0011\"".to_string()),
            ),
        ];
        for (case, extra, expected) in cases {
            assert_eq!(args(extra), expected, "{case}");
        }
    }

    #[test]
    fn what_has_to_be_given() {
        let cases: [(&[&str], &str); 2] = [
            (&["--cert-sha256", HASH], "--url is needed"),
            (&["--url", "u"], "--cert-sha256 is needed"),
        ];
        for (flags, expected) in cases {
            assert_eq!(
                parse(flags.iter().map(ToString::to_string)),
                Err(expected.to_string())
            );
        }
    }
}
