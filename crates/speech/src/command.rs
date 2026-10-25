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
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use rustix::process::{Pid, Signal, kill_process_group};

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

/// How long the reading of a stopped program's output is waited for. Once
/// its process group is stopped nothing writes to the pipe, and the read
/// ends at once; it can only go on where the program put a process of its
/// out of the group (`setsid`), which then keeps the pipe and the thread.
const STOPPED_READ_WAIT: Duration = Duration::from_secs(1);

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

/// The programs that run for the readers, each the leader of a process
/// group of its own, so that what they started is stopped with them.
///
/// WHY a group: a program with arguments that have spaces is put in a
/// script (`--engine-command`), and the program that reads is then the
/// script's child; stopping the script alone would leave it running, and
/// holding the pipe of the output. WHY shared: the transcriber stops them
/// when it ends ([`Running::stop_all`]), and they are no longer in its own
/// group, to which a terminal sends Ctrl-C.
#[derive(Clone, Debug, Default)]
pub struct Running(Arc<Mutex<Programs>>);

#[derive(Debug, Default)]
struct Programs {
    pids: Vec<Pid>,
    /// Set by [`Running::stop_all`]: no program is started after it.
    stopped: bool,
}

impl Running {
    fn lock(&self) -> MutexGuard<'_, Programs> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops every program that runs, with the processes it started, and
    /// starts none after.
    pub fn stop_all(&self) {
        let mut running = self.lock();
        running.stopped = true;
        for pid in running.pids.drain(..) {
            let _ = kill_process_group(pid, Signal::KILL);
        }
    }

    /// Stops `child`'s group, and waits for `child`.
    fn stop(&self, child: &mut Child) {
        let pid = Pid::from_child(child);
        {
            let mut running = self.lock();
            if running.pids.contains(&pid) {
                let _ = kill_process_group(pid, Signal::KILL);
                running.pids.retain(|other| *other != pid);
            }
        }
        let _ = child.wait();
    }

    /// `child`'s exit status, once it has exited.
    ///
    /// WHY under the lock, and the program let go of with it: once waited
    /// for, its ID may be given to another process, which a later
    /// [`Running::stop_all`] would stop.
    fn exited(&self, child: &mut Child) -> std::io::Result<Option<ExitStatus>> {
        let mut running = self.lock();
        let status = child.try_wait();
        if !matches!(status, Ok(None)) {
            let pid = Pid::from_child(child);
            running.pids.retain(|other| *other != pid);
        }
        status
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
    running: Running,
}

impl Command {
    /// A reader that runs `program` (the program, then its arguments), and
    /// writes its pieces to `wav`, a file of its own. The program, while it
    /// runs, is in `running`.
    ///
    /// # Errors
    /// When `program` is empty.
    pub fn new(
        program: Vec<String>,
        language: &str,
        limits: CommandLimits,
        wav: PathBuf,
        running: Running,
    ) -> Result<Self> {
        if program.is_empty() {
            bail!("no program to run");
        }
        Ok(Self {
            program,
            language: language.to_owned(),
            limits,
            wav,
            running,
        })
    }

    /// Runs the program on the file at `wav`, and takes its output.
    fn run(&self, wav: &Path) -> Result<String> {
        let (program, args) = self.program.split_first().context("no program to run")?;
        let mut child = {
            // Under the lock: the program is stopped by `stop_all` from the
            // moment it starts.
            let mut running = self.running.lock();
            if running.stopped {
                bail!("the readers are stopped");
            }
            let child = Process::new(program)
                .args(args)
                .arg(wav)
                .env("XMOQ_LANGUAGE", &self.language)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .process_group(0)
                .spawn()
                .with_context(|| format!("starting {program}"))?;
            running.pids.push(Pid::from_child(&child));
            child
        };
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
                self.running.stop(&mut child);
                join_stopped(reading);
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
            self.running.stop(&mut child);
            bail!("{program} wrote more than {most} bytes");
        }
        let status = self.exit(&mut child, deadline).with_context(|| {
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

    /// Waits for `child` to exit until `deadline`; `None` where it did not,
    /// and was stopped.
    fn exit(&self, child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
        loop {
            match self.running.exited(child) {
                Ok(Some(status)) => return Some(status),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
                _ => {
                    self.running.stop(child);
                    return None;
                }
            }
        }
    }
}

/// Waits for the reading of a stopped program's output to end, for
/// [`STOPPED_READ_WAIT`] at the most.
fn join_stopped<T>(reading: JoinHandle<T>) {
    let deadline = Instant::now() + STOPPED_READ_WAIT;
    while !reading.is_finished() && Instant::now() < deadline {
        std::thread::sleep(POLL);
    }
    if reading.is_finished() {
        let _ = reading.join();
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
            let mut command =
                Command::new(program, "ja", limits, wav.clone(), Running::default()).unwrap();
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

    /// A script whose child (the program that reads, say) runs past the
    /// end of the script: `sleep`, which holds the pipe of the output and
    /// writes its process ID next to the piece.
    fn a_script_with_a_child(dir: &Path) -> Vec<String> {
        let script = format!("sleep 30 & echo $! > {}/child; wait", dir.display());
        ["sh", "-c", &script, "sh"].map(str::to_owned).to_vec()
    }

    /// The process ID the script wrote, once it is there.
    fn child_of(dir: &Path) -> Pid {
        let file = dir.join("child");
        for _ in 0..500 {
            if let Some(pid) = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
                .and_then(Pid::from_raw)
            {
                return pid;
            }
            std::thread::sleep(POLL);
        }
        panic!("the script wrote no process ID");
    }

    /// Whether `pid` is gone. A process that was killed is a zombie until
    /// the process that took it in (init) waits for it, so it is looked at
    /// for a while.
    fn gone(pid: Pid) -> bool {
        (0..200).any(|_| {
            let gone = rustix::process::test_kill_process(pid).is_err();
            if !gone {
                std::thread::sleep(POLL);
            }
            gone
        })
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xmoq-speech-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_program_out_of_time_is_stopped_with_what_it_started() {
        let dir = test_dir("out-of-time");
        let limits = CommandLimits {
            timeout: Duration::from_millis(300),
            most_output: 8,
        };
        let mut command = Command::new(
            a_script_with_a_child(&dir),
            "ja",
            limits,
            dir.join("piece.wav"),
            Running::default(),
        )
        .unwrap();
        let started = Instant::now();
        let error = format!("{:#}", command.text(&[0.0; 5]).unwrap_err());
        assert!(error.contains("took longer than 300 ms"), "{error}");
        // The output was read to its end, which comes once nothing holds the
        // pipe: the thread that read it ended, and was not left waiting for
        // the 30 s of `sleep`.
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(gone(child_of(&dir)), "the script's child is left running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_programs_that_run_are_stopped_with_what_they_started_and_none_starts_after() {
        let dir = test_dir("stop-all");
        let running = Running::default();
        let mut command = Command::new(
            a_script_with_a_child(&dir),
            "ja",
            CommandLimits::default(),
            dir.join("piece.wav"),
            running.clone(),
        )
        .unwrap();
        let reading = std::thread::spawn(move || command.text(&[0.0; 5]));
        let child = child_of(&dir);
        running.stop_all();
        let error = format!("{:#}", reading.join().unwrap().unwrap_err());
        assert!(error.contains("exited with"), "{error}");
        assert!(gone(child), "the script's child is left running");
        let mut later = Command::new(
            ["true".to_owned()].to_vec(),
            "ja",
            CommandLimits::default(),
            dir.join("later.wav"),
            running,
        )
        .unwrap();
        let error = format!("{:#}", later.text(&[0.0; 5]).unwrap_err());
        assert!(
            error.contains("stopped"),
            "nothing is started after: {error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
