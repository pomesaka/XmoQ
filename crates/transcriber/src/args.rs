//! The flags of the transcriber.

use std::path::PathBuf;

use std::time::Duration;
use xmoq_moqt::name::TrackNamespace;
use xmoq_moqt_io::wt::certificate_hash;

use xmoq_speech::command::CommandLimits;
use xmoq_speech::engine::Engine;
use xmoq_subscriber::kept::{TIMES, TIMES_MOST};

/// How the transcriber is started without a flag it does not know.
pub const USAGE: &str = "\
xmoq-transcriber --url <https://host:port/> --cert-sha256 <64 hex digits> --namespace <a/b>...
                 [--engine whisper|command] [--model <file>] [--language <code>]
                 [--engine-command '<program> <args>'] [--engine-timeout-ms <n>] [--engine-most-output <n>]
                 [--voice vad|loudness] [--vad-model <file>]
                 [--keep <n>] [--waiting <n>] [--readers <n>] [--tracks <n>]
                 [--ask-again-times <n>] [--auth-key-hex <hex>]

--namespace     a prefix whose broadcasts are listened to, its fields between slashes, 30 at the most;
                up to 8, none the same as another or a prefix of it. Its transcript is the track speech of xmoq/transcript/<prefix>
--engine        what reads the pieces: whisper.cpp with --model, or a program run for each piece (whisper)
--engine-command      with --engine command: the program and its arguments, between spaces; the piece's
                      WAV file is its last argument, the language in XMOQ_LANGUAGE, and its stdout the words
--engine-timeout-ms   with --engine command: how long it may take for a piece, or it is killed
                      (30000; 1 to 600000)
--engine-most-output  with --engine command: the most bytes of its stdout (65536; 1 to 1048576)
--model         the file of a model of whisper.cpp (~/.cache/xmoq/whisper/ggml-small-q5_1.bin;
                mise run speech-model fetches it)
--language      a language code of whisper's, or auto (auto). A code is twice as fast as auto, and has
                whisper.cpp read every voice in that language: English read with ja comes out in Japanese
--voice         how a voice is told: by the model of voice activity (vad), or by loudness (vad)
--vad-model     the file of the model of voice activity (~/.cache/xmoq/whisper/ggml-silero-v5.1.2.bin;
                mise run speech-model -- silero-v5.1.2 fetches it)
--keep          the latest pieces of each transcript kept for FETCH (1000; 1 to 100000)
--waiting       pieces that wait to be read; one more is dropped and counted (16; 1 to 256)
--readers       threads that read pieces at once: of whisper, each a state of its own over the one model;
                of command, each runs the program for a piece of its own (1; 1 to 8)
--tracks        broadcasts listened to at once; one more is passed over (16; 1 to 100)
--ask-again-times   how many times in a row the catalog of a broadcast is asked again, without a
                    subscription between, or found malformed, before it is asked again only when
                    the broadcast's namespace comes again (30; 0 to 10000)
--auth-key-hex  the relay's key, of which tokens are made: where the relay shares its key with the transcriber";

/// The most `--namespace` flags: as many `SUBSCRIBE_NAMESPACE` requests as
/// the own relay lets one session have open
/// (`MAX_NAMESPACE_SUBSCRIPTIONS_PER_SESSION`), as for the gatherer's
/// `--room`. WHY refused at the start: a ninth would be refused for good,
/// and its broadcasts never read while the transcriber looked as if it ran.
pub const PREFIXES_MOST: usize = 8;

/// The most `--keep`. WHY a bound: the pieces are held in memory; 100,000
/// of 200 bytes is 20 MB a prefix.
pub const KEEP_MOST: usize = 100_000;

/// The most `--waiting`. Each waiting piece holds up to 15 s of samples,
/// 960 KB.
pub const WAITING_MOST: usize = 256;

/// The most `--engine-timeout-ms`: 10 minutes. A piece is 15 s at the
/// most, and a reader that takes longer has the pieces dropped behind it.
pub const ENGINE_TIMEOUT_MOST_MS: usize = 600_000;

/// The most `--engine-most-output`: 1 MiB, the most the words of a piece
/// are held at.
pub const ENGINE_OUTPUT_MOST: usize = 1024 * 1024;

/// The most `--readers`. Each holds a state of whisper.cpp of its own, tens
/// of MiB beside the model, which they share.
pub const READERS_MOST: usize = 8;

/// The most `--tracks`: the own relay keeps 256 requests of one session,
/// two for each broadcast (the catalog and the audio).
pub const TRACKS_MOST: usize = 100;

/// What the transcriber was started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `--url`: the relay.
    pub url: String,
    /// `--cert-sha256`: the hash of the relay's certificate.
    pub cert_sha256: [u8; 32],
    /// `--namespace`, in the order given.
    pub prefixes: Vec<TrackNamespace>,
    /// `--engine`.
    pub engine: Engine,
    /// `--engine-command`, split at spaces; empty but with `--engine
    /// command`.
    pub command: Vec<String>,
    /// `--engine-timeout-ms` and `--engine-most-output`.
    pub command_limits: CommandLimits,
    /// `--model`; `None` for the default in the cache
    /// (`xmoq_speech::model`).
    pub model: Option<PathBuf>,
    /// `--language`.
    pub language: String,
    /// `--voice`: whether a voice is told by the model of voice activity.
    pub vad: bool,
    /// `--vad-model`; `None` for the default in the cache.
    pub vad_model: Option<PathBuf>,
    /// `--keep`.
    pub keep: usize,
    /// `--waiting`.
    pub waiting: usize,
    /// `--readers`.
    pub readers: usize,
    /// `--tracks`.
    pub tracks: usize,
    /// `--ask-again-times`.
    pub ask_again_times: u32,
    /// `--auth-key-hex`: the key the relay checks tokens with, of which the
    /// transcriber makes its own, as the gatherer of the chat does: for a
    /// relay that shares its key with the transcriber (provisional).
    pub auth_key: Option<Vec<u8>>,
}

/// A number from 1 to `most`.
fn number_within(flag: &str, value: &str, most: usize) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(number) if (1..=most).contains(&number) => Ok(number),
        _ => Err(format!(
            "{flag} is a number from 1 to {most}, not {value:?}"
        )),
    }
}

/// A namespace written with its fields between slashes.
fn prefix(value: &str) -> Result<TrackNamespace, String> {
    let fields = value
        .split('/')
        .map(|field| field.as_bytes().to_vec())
        .collect();
    let namespace = TrackNamespace::new(fields)
        .map_err(|_| format!("--namespace {value:?} is no namespace"))?;
    // The transcript's namespace has two fields more.
    xmoq_transcript_tracks::names::transcript_namespace(&namespace)
        .map_err(|_| format!("--namespace {value:?} has too many fields for its transcript"))?;
    Ok(namespace)
}

/// `--ask-again-times`: a number from 0 to [`TIMES_MOST`].
fn times(value: &str) -> Result<u32, String> {
    value
        .parse()
        .ok()
        .filter(|times| *times <= TIMES_MOST)
        .ok_or_else(|| {
            format!("--ask-again-times is a number from 0 to {TIMES_MOST}, not {value:?}")
        })
}

/// Adds the prefix of a `--namespace` to `prefixes`.
fn add_prefix(prefixes: &mut Vec<TrackNamespace>, value: &str) -> Result<(), String> {
    let namespace = prefix(value)?;
    // WHY none under another: a broadcast under both would be
    // read twice, into two transcripts.
    if prefixes
        .iter()
        .any(|other| is_prefix(other, &namespace) || is_prefix(&namespace, other))
    {
        return Err(format!(
            "--namespace {value:?} is the same as another, or a prefix of it, or under it"
        ));
    }
    prefixes.push(namespace);
    Ok(())
}

/// Whether `prefix` is a prefix of `of`, or the same.
fn is_prefix(prefix: &TrackNamespace, of: &TrackNamespace) -> bool {
    of.fields().starts_with(prefix.fields())
}

/// The limits of `--engine command`, where the flags of the engine agree
/// with it.
///
/// WHY refused and not passed over: a flag that does nothing would have the
/// transcriber look as if it read with what it names.
fn command_limits(
    engine: Engine,
    command: &[String],
    timeout: Option<usize>,
    most_output: Option<usize>,
    model: bool,
) -> Result<CommandLimits, String> {
    match engine {
        Engine::Whisper => {
            if !command.is_empty() || timeout.is_some() || most_output.is_some() {
                return Err("--engine-command, --engine-timeout-ms and --engine-most-output are for --engine command".to_string());
            }
        }
        Engine::Command => {
            if command.is_empty() {
                return Err("--engine command needs --engine-command".to_string());
            }
            if model {
                return Err("--model is for --engine whisper".to_string());
            }
        }
    }
    let defaults = CommandLimits::default();
    Ok(CommandLimits {
        timeout: timeout.map_or(defaults.timeout, |ms| {
            Duration::from_millis(u64::try_from(ms).unwrap_or(u64::MAX))
        }),
        most_output: most_output.unwrap_or(defaults.most_output),
    })
}

/// Reads the flags.
///
/// # Errors
///
/// What is wrong with them.
pub fn parse(mut flags: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut url, mut cert_sha256, mut model, mut auth_key) = (None, None, None, None);
    let mut prefixes: Vec<TrackNamespace> = Vec::new();
    let mut language = "auto".to_string();
    let mut engine = Engine::Whisper;
    let mut command: Vec<String> = Vec::new();
    let (mut timeout, mut most_output) = (None, None);
    let (mut vad, mut vad_model) = (true, None);
    let (mut keep, mut waiting, mut tracks, mut readers) = (1000, 16, 16, 1);
    let mut ask_again_times = TIMES;
    while let Some(flag) = flags.next() {
        let value = flags
            .next()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--url" => url = Some(value),
            "--cert-sha256" => {
                let hash = certificate_hash(&value)
                    .ok_or_else(|| format!("--cert-sha256 is 64 hex digits, not {value:?}"))?;
                cert_sha256 = Some(hash);
            }
            "--namespace" => add_prefix(&mut prefixes, &value)?,
            "--engine" => {
                engine = Engine::named(&value).ok_or_else(|| {
                    format!("--engine is one of {}, not {value:?}", Engine::NAMES)
                })?;
            }
            "--engine-command" => {
                command = value.split_whitespace().map(str::to_string).collect();
                if command.is_empty() {
                    return Err("--engine-command names no program".to_string());
                }
            }
            "--engine-timeout-ms" => {
                timeout = Some(number_within(&flag, &value, ENGINE_TIMEOUT_MOST_MS)?);
            }
            "--engine-most-output" => {
                most_output = Some(number_within(&flag, &value, ENGINE_OUTPUT_MOST)?);
            }
            "--model" => model = Some(PathBuf::from(value)),
            "--language" => language = value,
            "--voice" => {
                vad = match value.as_str() {
                    "vad" => true,
                    "loudness" => false,
                    _ => return Err(format!("--voice is vad or loudness, not {value:?}")),
                };
            }
            "--vad-model" => vad_model = Some(PathBuf::from(value)),
            "--keep" => keep = number_within(&flag, &value, KEEP_MOST)?,
            "--waiting" => waiting = number_within(&flag, &value, WAITING_MOST)?,
            "--tracks" => tracks = number_within(&flag, &value, TRACKS_MOST)?,
            "--readers" => readers = number_within(&flag, &value, READERS_MOST)?,
            "--ask-again-times" => ask_again_times = times(&value)?,
            "--auth-key-hex" => {
                auth_key = Some(
                    xmoq_room_token::key_of_hex(&value)
                        .ok_or_else(|| format!("--auth-key-hex is hex digits, not {value:?}"))?,
                );
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if prefixes.is_empty() {
        return Err("--namespace is needed".to_string());
    }
    let command_limits = command_limits(engine, &command, timeout, most_output, model.is_some())?;
    if prefixes.len() > PREFIXES_MOST {
        return Err(format!(
            "--namespace is given {} times: a session at the own relay may have {PREFIXES_MOST} SUBSCRIBE_NAMESPACE requests; run another transcriber for the rest",
            prefixes.len()
        ));
    }
    Ok(Args {
        url: url.ok_or("--url is needed")?,
        cert_sha256: cert_sha256.ok_or("--cert-sha256 is needed")?,
        prefixes,
        engine,
        command,
        command_limits,
        model,
        language,
        vad,
        vad_model,
        keep,
        waiting,
        readers,
        tracks,
        ask_again_times,
        auth_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";

    fn args(flags: &[&str]) -> Result<Args, String> {
        let mut all = vec!["--url", "https://r/", "--cert-sha256", HASH];
        all.extend_from_slice(flags);
        parse(all.into_iter().map(str::to_string))
    }

    fn namespace(text: &str) -> TrackNamespace {
        prefix(text).unwrap()
    }

    #[test]
    fn the_defaults() {
        let read = args(&["--namespace", "xmoq/demo"]).unwrap();
        assert_eq!(read.prefixes, [namespace("xmoq/demo")]);
        assert_eq!(read.cert_sha256[31], 0xff);
        assert_eq!(
            (
                read.model,
                read.language.as_str(),
                read.keep,
                read.waiting,
                read.tracks,
                read.auth_key
            ),
            (None, "auto", 1000, 16, 16, None)
        );
        assert_eq!(
            (read.vad, read.vad_model, read.readers, read.ask_again_times),
            (true, None, 1, 30)
        );
        let never = args(&["--namespace", "a", "--ask-again-times", "0"]).unwrap();
        assert_eq!(never.ask_again_times, 0);
        assert_eq!(
            (read.engine, read.command.is_empty(), read.command_limits),
            (Engine::Whisper, true, CommandLimits::default())
        );
        let command = args(&[
            "--namespace",
            "a",
            "--engine",
            "command",
            "--engine-command",
            "say-it  --fast",
            "--engine-timeout-ms",
            "500",
            "--engine-most-output",
            "10",
        ])
        .unwrap();
        assert_eq!(
            (command.engine, command.command, command.command_limits),
            (
                Engine::Command,
                vec!["say-it".to_string(), "--fast".to_string()],
                CommandLimits {
                    timeout: Duration::from_millis(500),
                    most_output: 10
                }
            )
        );
        let loudness = args(&["--namespace", "a", "--voice", "loudness"]).unwrap();
        assert!(!loudness.vad);
    }

    #[test]
    fn flags_that_are_refused() {
        let cases: [(&[&str], &str); 18] = [
            (
                &["--namespace", "a", "--ask-again-times", "10001"],
                "--ask-again-times is a number from 0 to 10000, not \"10001\"",
            ),
            (
                &["--namespace", "a", "--engine", "claude"],
                "--engine is one of whisper, command",
            ),
            (
                &["--namespace", "a", "--engine", "command"],
                "--engine command needs --engine-command",
            ),
            (
                &["--namespace", "a", "--engine-command", "x"],
                "are for --engine command",
            ),
            (
                &[
                    "--namespace",
                    "a",
                    "--engine",
                    "command",
                    "--engine-command",
                    "x",
                    "--model",
                    "m",
                ],
                "--model is for --engine whisper",
            ),
            (
                &[
                    "--namespace",
                    "a",
                    "--engine",
                    "command",
                    "--engine-command",
                    " ",
                ],
                "--engine-command names no program",
            ),
            (
                &[
                    "--namespace",
                    "a",
                    "--engine",
                    "command",
                    "--engine-command",
                    "x",
                    "--engine-timeout-ms",
                    "0",
                ],
                "--engine-timeout-ms is a number from 1 to 600000",
            ),
            (
                &["--namespace", "a", "--readers", "9"],
                "--readers is a number from 1 to 8",
            ),
            (
                &["--namespace", "a", "--voice", "quiet"],
                "--voice is vad or loudness",
            ),
            (&[], "--namespace is needed"),
            (&["--namespace", "a//b"], "is no namespace"),
            (
                &["--namespace", "a/b", "--namespace", "a/b"],
                "the same as another",
            ),
            (
                &["--namespace", "a/b", "--namespace", "a"],
                "a prefix of it",
            ),
            (&["--namespace", "a", "--namespace", "a/b"], "under it"),
            (
                &["--namespace", "a", "--keep", "0"],
                "--keep is a number from 1 to 100000",
            ),
            (
                &["--namespace", "a", "--waiting", "257"],
                "--waiting is a number from 1 to 256",
            ),
            (
                &["--namespace", "a", "--cert-sha256", "00"],
                "--cert-sha256 is 64 hex digits",
            ),
            (&["--namespace", "a", "--loud", "1"], "unknown flag --loud"),
        ];
        for (flags, expected) in cases {
            let error = args(flags).unwrap_err();
            assert!(error.contains(expected), "{flags:?}: {error}");
        }
    }

    #[test]
    fn eight_prefixes_and_not_nine() {
        let mut flags: Vec<String> = Vec::new();
        for n in 0..9 {
            flags.push("--namespace".to_string());
            flags.push(format!("p{n}"));
        }
        let given: Vec<&str> = flags.iter().map(String::as_str).collect();
        assert!(args(&given[..16]).is_ok());
        assert!(args(&given).unwrap_err().contains("given 9 times"));
    }

    #[test]
    fn a_prefix_of_31_fields_has_no_room_for_its_transcript() {
        let fields: Vec<String> = (0..31).map(|n| n.to_string()).collect();
        let error = args(&["--namespace", &fields.join("/")]).unwrap_err();
        assert!(error.contains("too many fields"), "{error}");
    }
}
