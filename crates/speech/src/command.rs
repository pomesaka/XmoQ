//! A reader that runs a program for each piece: the piece is written as a
//! WAV file, whose path is the program's last argument, and the program's
//! standard output is its words. The language is in `XMOQ_LANGUAGE`.
//!
//! WHY a program and a file: whatever reads the words (a script around
//! another model, an agent) is put in place without a change here.
//! WHY NOT a service over the network: where the sound of the people
//! present may be sent is undecided (Q35, `docs/open-questions.md`); a
//! program that sends it is the person's who runs it.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::Words;
use crate::decode::RATE;

/// How long a program may take for a piece: 30 s, twice the longest piece
/// (`Cut::longest_ms`). **Provisional**.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most bytes of standard output taken as a piece's words: 64 KiB, as
/// a line of the chat. 15 s of speech is under 1 KB of text.
/// **Provisional**.
pub const DEFAULT_MOST_OUTPUT: usize = 64 * 1024;

/// How often a running program is looked at.
const POLL: Duration = Duration::from_millis(10);

/// What a program is given before it is stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandLimits {
    /// How long it may take for a piece, from its start to its exit.
    pub timeout: Duration,
    /// The most bytes of its standard output; more is a failure.
    pub most_output: usize,
}

impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            most_output: DEFAULT_MOST_OUTPUT,
        }
    }
}

/// A reader that runs `program` for each piece.
pub struct Command {
    /// The program and its arguments, before the path of the WAV file.
    program: Vec<String>,
    language: String,
    limits: CommandLimits,
    /// Where this reader writes its pieces, one at a time.
    wav: PathBuf,
}

impl Command {
    /// A reader that runs `program` (the program, then its arguments), and
    /// writes its pieces to `wav`, a file of its own.
    ///
    /// # Errors
    /// When `program` is empty.
    pub fn new(
        program: Vec<String>,
        language: &str,
        limits: CommandLimits,
        wav: PathBuf,
    ) -> Result<Self> {
        if program.is_empty() {
            bail!("no program to run");
        }
        Ok(Self {
            program,
            language: language.to_owned(),
            limits,
            wav,
        })
    }

    /// Runs the program on the file at `wav`, and takes its output.
    fn run(&self, wav: &Path) -> Result<String> {
        let (program, args) = self.program.split_first().context("no program to run")?;
        let mut child = Process::new(program)
            .args(args)
            .arg(wav)
            .env("XMOQ_LANGUAGE", &self.language)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .with_context(|| format!("starting {program}"))?;
        let deadline = Instant::now() + self.limits.timeout;
        let mut stdout = child.stdout.take().context("the program's output")?;
        let most = self.limits.most_output;
        // WHY a thread: a read of the pipe waits for as long as the program
        // writes nothing, and the deadline has to be kept meanwhile. One
        // byte past the most is read, to tell a longer output.
        let reading = std::thread::spawn(move || {
            let mut output = Vec::new();
            let read = stdout
                .by_ref()
                .take(u64::try_from(most).unwrap_or(u64::MAX).saturating_add(1))
                .read_to_end(&mut output);
            read.map(|_| output)
        });
        while !reading.is_finished() {
            if Instant::now() >= deadline {
                stop(&mut child);
                bail!(
                    "{program} took longer than {} ms",
                    self.limits.timeout.as_millis()
                );
            }
            std::thread::sleep(POLL);
        }
        let output = match reading.join() {
            Ok(read) => read.context("reading the program's output")?,
            Err(_) => bail!("reading the program's output"),
        };
        if output.len() > most {
            stop(&mut child);
            bail!("{program} wrote more than {most} bytes");
        }
        let status = exit(&mut child, deadline).with_context(|| {
            format!(
                "{program} took longer than {} ms",
                self.limits.timeout.as_millis()
            )
        })?;
        if !status.success() {
            bail!("{program} exited with {status}");
        }
        let text = String::from_utf8(output).context("the program's output is not UTF-8")?;
        Ok(text.trim().to_owned())
    }
}

impl Words for Command {
    fn text(&mut self, samples: &[f32]) -> Result<String> {
        std::fs::write(&self.wav, wav(samples))
            .with_context(|| format!("writing {}", self.wav.display()))?;
        let text = self.run(&self.wav);
        // The piece is the voice of somebody present: it is not left behind.
        let _ = std::fs::remove_file(&self.wav);
        text
    }
}

/// Waits for `child` to exit until `deadline`; `None` where it did not,
/// and was stopped.
fn exit(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                stop(child);
                return None;
            }
        }
    }
}

/// Kills `child` and waits for it, so that it is not left running.
fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// `samples` as a WAV file: PCM, 16 bits, one channel, at [`RATE`].
#[must_use]
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // one channel
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes a second
    bytes.extend_from_slice(&2u16.to_le_bytes()); // bytes a sample
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        // A float becomes an integer by `as` alone in Rust, which
        // saturates; clamped to [-1, 1], the value is within i16 anyway.
        #[allow(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            reason = "no other conversion from f32 to i16 without unsafe"
        )]
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_has_the_header_of_16_bit_mono_at_16_khz() {
        let bytes = wav(&[0.0, 1.0, -1.0]);
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(bytes.get(..4), Some(&b"RIFF"[..]));
        assert_eq!(bytes.get(24..28), Some(&16_000u32.to_le_bytes()[..]));
        assert_eq!(bytes.get(40..44), Some(&6u32.to_le_bytes()[..]));
        assert_eq!(bytes.get(44..), Some(&[0, 0, 0xff, 0x7f, 0x01, 0x80][..]));
    }

    #[test]
    fn a_program_is_run_for_a_piece() {
        // The name, the script given to `sh -c` (its $1 is the WAV file),
        // the limits, and the words, or what the error says.
        type Case = (
            &'static str,
            &'static str,
            CommandLimits,
            Result<&'static str, &'static str>,
        );
        let limits = CommandLimits::default();
        let short = CommandLimits {
            timeout: Duration::from_millis(300),
            most_output: 8,
        };
        let cases: [Case; 7] = [
            (
                "its output is the words, trimmed",
                "echo ' hello '",
                limits,
                Ok("hello"),
            ),
            (
                "the file is the piece",
                "wc -c < \"$1\" | tr -d ' '",
                limits,
                Ok("54"),
            ),
            (
                "the language is told",
                "printf %s \"$XMOQ_LANGUAGE\"",
                limits,
                Ok("ja"),
            ),
            ("no words", "true", limits, Ok("")),
            (
                "an exit that is not 0",
                "echo partly; exit 3",
                limits,
                Err("exited with"),
            ),
            ("too long", "sleep 5", short, Err("took longer than 300 ms")),
            (
                "too much",
                "echo 123456789",
                short,
                Err("wrote more than 8 bytes"),
            ),
        ];
        let dir = std::env::temp_dir().join(format!("xmoq-speech-command-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, script, limits, expected) in cases {
            let program = ["sh", "-c", script, "sh"].map(str::to_owned).to_vec();
            let wav = dir.join("piece.wav");
            let mut command = Command::new(program, "ja", limits, wav.clone()).unwrap();
            let read = command.text(&[0.0; 5]);
            match expected {
                Ok(words) => assert_eq!(read.unwrap(), words, "{name}"),
                Err(says) => {
                    let error = format!("{:#}", read.unwrap_err());
                    assert!(error.contains(says), "{name}: {error}");
                }
            }
            assert!(!wav.exists(), "{name}: the piece is left behind");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
