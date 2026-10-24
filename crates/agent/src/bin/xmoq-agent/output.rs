//! Where the events go: one line each on stdout and at the end of
//! `<DIR>/events.jsonl`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use xmoq_agent::events::Event;

/// The name of the file of events in `--out`.
pub const EVENTS_FILE: &str = "events.jsonl";

/// The two places events are written to.
pub struct Output {
    file: File,
    /// Whether a write to the file failed, which is said once.
    file_failed: bool,
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
    /// Makes `dir` where it is not, and opens its file of events to be
    /// added to.
    ///
    /// WHY added to and not begun anew: an agent that started the reader
    /// again goes on reading the same file, with what came before still in
    /// it.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("--out {}", dir.display()))?;
        let path = dir.join(EVENTS_FILE);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("{}", path.display()))?;
        Ok(Self {
            file,
            file_failed: false,
        })
    }

    /// Writes `event` as one line to both. Each line is flushed as it is
    /// written: an agent waits for it.
    pub fn write(&mut self, event: &Event) {
        let line = event.line(now_ms());
        // WHY nothing is done where stdout fails: an agent that reads only
        // the file may have closed it, and the file is written all the same.
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
        let written = writeln!(self.file, "{line}").and_then(|()| self.file.flush());
        if let Err(error) = written
            && !self.file_failed
        {
            self.file_failed = true;
            eprintln!("XMOQ_AGENT_FILE_FAILED {error}");
        }
    }
}
