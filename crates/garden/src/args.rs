//! The flags of the garden.

use xmoq_moqt_io::wt::certificate_hash;
use xmoq_subscriber::kept::{Keeping, TIMES, TIMES_MOST, Tries};

/// How the garden is started without a flag it does not know.
pub const USAGE: &str = "\
xmoq-garden --url <https://host:port/> --cert-sha256 <64 hex digits>
            [--auth-key-hex <hex>] [--ask-again-times <n>] [--ask-again-ms <ms>]

--auth-key-hex     the key the relay checks tokens with; the garden makes its own tokens
                   (where the relay shares its key with it)
--ask-again-times  how many times in a row an actor whose `acted` ended is asked again
                   without one that was established (30; 0 never; up to 10000)
--ask-again-ms     the pause before it is, after an end or a refusal that names no time
                   or 0 (1000; 100 to 60000)";

/// The pause after an end by default: the rule's.
pub const PAUSE_MS: u64 = xmoq_again::ASK_AGAIN_MS;

/// The least `--ask-again-ms`: the rule's shortest pause after a refusal.
pub const PAUSE_MS_LEAST: u64 = xmoq_again::SHORTEST_MS;

/// The most `--ask-again-ms`.
pub const PAUSE_MS_MOST: u64 = 60_000;

/// When an actor's `acted` is asked for again.
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
    /// How `acted` is kept (`kept::decide` of `xmoq-subscriber`), as the
    /// chat's gatherer keeps `said`. WHY not optional: an actor is there to
    /// act on the garden, and a page that announces publishes `acted`.
    #[must_use]
    pub fn keeping(self) -> Keeping {
        Keeping {
            again_after_failure: true,
            times: Some(self.times),
            optional: false,
        }
    }

    /// What is kept of an actor followed anew, or whose NAMESPACE came
    /// again.
    #[must_use]
    pub fn tries(self) -> Tries {
        Tries::new(xmoq_again::Pauses {
            after_end_ms: self.pause_ms,
            ..xmoq_again::Pauses::default()
        })
    }
}

/// What the garden was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `--url`: the relay.
    pub url: String,
    /// `--cert-sha256`: the hash of the relay's certificate.
    pub cert_sha256: [u8; 32],
    /// `--auth-key-hex`: the key of the relay, with which the garden makes
    /// tokens of its own, as the chat's gatherer does (provisional).
    pub auth_key: Option<Vec<u8>>,
    /// `--ask-again-times`, `--ask-again-ms`.
    pub ask_again: AskAgain,
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
    let (mut url, mut cert_sha256, mut auth_key) = (None, None, None);
    let mut ask_again = AskAgain::default();
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
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Args {
        url: url.ok_or("--url is needed")?,
        cert_sha256: cert_sha256.ok_or("--cert-sha256 is needed")?,
        auth_key,
        ask_again,
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
        }
    }

    #[test]
    fn the_flags_and_their_bounds() {
        let cases: [(&str, &[&str], Result<Args, String>); 9] = [
            ("the defaults", &[], Ok(base())),
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
                "a flag of the pet",
                &["--decisions-url", "http://127.0.0.1:3333/"],
                Err("unknown flag --decisions-url".to_string()),
            ),
            (
                "a flag without its value",
                &["--auth-key-hex"],
                Err("--auth-key-hex needs a value".to_string()),
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
