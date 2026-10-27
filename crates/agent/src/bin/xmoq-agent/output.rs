//! Where the events go: one line each on stdout at once, and the lines of
//! each second in a file of `<DIR>/events/` of their own
//! (`xmoq_agent::event_files`), on a thread of its own.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use std::io::Write;
use xmoq_agent::event_files::{EVENTS_DIR, name_of, next_number, number_of};
use xmoq_agent::events::Event;

/// How long a line waits to be written to a file of `events/`, with the
/// lines after it. **Provisional.** WHY a second: each wait is a file, and
/// the page of a meeting waits as long (`FLUSH_EVERY_MS` of
/// `@xmoq/meeting`); an agent that reads the files every second reads a
/// line at most two seconds after it was written.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// The two places events are written to.
pub struct Output {
    /// The lines for the thread that writes the files; `None` once it is
    /// told to end.
    lines: Option<mpsc::Sender<String>>,
    writer: Option<JoinHandle<()>>,
}

/// The machine's clock, in milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

impl Output {
    /// Makes `dir/events/` where it is not, and starts the thread that
    /// writes the files in it.
    ///
    /// WHY the files there are kept: an agent that started the reader
    /// again goes on reading the same directory, with what came before
    /// still in it, and the next file is named after the newest there.
    pub fn open(dir: &Path) -> Result<Self> {
        let events = dir.join(EVENTS_DIR);
        std::fs::create_dir_all(&events).with_context(|| format!("--out {}", events.display()))?;
        let newest = std::fs::read_dir(&events)
            .with_context(|| format!("{}", events.display()))?
            .filter_map(|entry| number_of(entry.ok()?.file_name().to_str()?))
            .max();
        let (lines, received) = mpsc::channel();
        let writer = std::thread::spawn(move || write_files(&events, newest, &received));
        Ok(Self {
            lines: Some(lines),
            writer: Some(writer),
        })
    }

    /// Writes `event` as one line to both: on stdout at once, as an agent
    /// that reads stdout waits for it, and in the next file of `events/`.
    pub fn write(&mut self, event: &Event) {
        let line = event.line(now_ms());
        // WHY nothing is done where stdout fails: an agent that reads only
        // the files may have closed it, and the files are written all the
        // same.
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
        if let Some(lines) = &self.lines {
            // The thread ends only when this is dropped.
            let _ = lines.send(format!("{line}\n"));
        }
    }
}

impl Drop for Output {
    /// Writes the lines that wait, before the process ends.
    fn drop(&mut self) {
        self.lines = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

/// Writes the lines that come into files of `events`, those of each
/// [`FLUSH_EVERY`] together, until the sender is dropped.
fn write_files(events: &Path, mut newest: Option<u64>, lines: &mpsc::Receiver<String>) {
    let mut failed = false;
    while let Ok(first) = lines.recv() {
        let mut text = first;
        let until = Instant::now() + FLUSH_EVERY;
        let mut ended = false;
        loop {
            match lines.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(line) => text.push_str(&line),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    ended = true;
                    break;
                }
            }
        }
        let number = next_number(now_ms(), newest);
        newest = Some(number);
        if let Err(error) = put(events, &name_of(number), &text)
            && !failed
        {
            failed = true;
            eprintln!("XMOQ_AGENT_FILE_FAILED {error:#}");
        }
        if ended {
            return;
        }
    }
}

/// Puts a file of `text` at `events/name`, whole: written under another
/// name first and moved there.
///
/// WHY moved: a reader that lists the directory while the file is written
/// would read a part of it, and never the rest. The other name begins with
/// `.` and does not end with `.jsonl`, which a reader passes over.
fn put(events: &Path, name: &str, text: &str) -> Result<()> {
    let written: PathBuf = events.join(format!(".{name}.tmp"));
    std::fs::write(&written, text).with_context(|| format!("{}", written.display()))?;
    let path = events.join(name);
    std::fs::rename(&written, &path).with_context(|| format!("{}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names and the texts of the files of `events/`, in the order of
    /// their names.
    fn files_of(dir: &Path) -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = std::fs::read_dir(dir.join(EVENTS_DIR))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                (name, std::fs::read_to_string(entry.path()).unwrap())
            })
            .collect();
        files.sort();
        files
    }

    #[test]
    fn the_lines_of_a_second_are_one_file_and_a_reader_started_again_names_its_files_after_them() {
        let dir = std::env::temp_dir().join(format!("xmoq-agent-output-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let error = |message: &str| Event::Error {
            message: message.to_string(),
        };
        let mut output = Output::open(&dir).unwrap();
        output.write(&error("a"));
        output.write(&error("b"));
        drop(output);
        let first = files_of(&dir);
        let [(name, text)] = &first[..] else {
            panic!("{first:?}");
        };
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.len() == 2
                && lines[0].contains(r#""message":"a""#)
                && lines[1].contains(r#""message":"b""#),
            "{text}"
        );
        // A file named in the future, as a clock that went back would find.
        let later = number_of(name).unwrap() + 3_600_000;
        std::fs::write(dir.join(EVENTS_DIR).join(name_of(later)), "").unwrap();
        let mut output = Output::open(&dir).unwrap();
        output.write(&error("c"));
        drop(output);
        let names: Vec<String> = files_of(&dir).into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            [name.clone(), name_of(later), name_of(later + 1)],
            "no file written over, and none put before one there"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
