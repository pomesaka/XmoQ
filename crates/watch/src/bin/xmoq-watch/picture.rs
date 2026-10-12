//! The picture: frames decoded as they come, each written for the terminal
//! at the moment it is due, on a thread of its own.

use std::collections::VecDeque;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use xmoq_watch::args::Medium;
use xmoq_watch::marks::Recent;
use xmoq_watch::{fit, kitty, marks};

use crate::sound::micros_since;
use crate::video_decoder::{Decoded, VideoDecoder};

/// Lines under the picture: the last of the chat, and one for the keys.
pub const LINES: usize = 4;

/// The most frames that wait to be decoded and drawn, as the watching page
/// holds 30. One more is left out, and the picture goes on at the next
/// Group: a terminal that takes its frames slowly must not have them kept
/// for it without end.
pub const MOST_FRAMES_WAITING: usize = 30;

/// The most values of a series that is counted (`Recent`): about two
/// minutes of frames.
pub const MOST_COUNTED: usize = 4096;

/// The most files of pixels of this viewer that are there at once
/// (`--medium file`): a second of frames. A terminal removes each when it
/// has read it; where none does, the oldest is removed for the newest.
const MOST_FILES: usize = 30;

/// How long the thread sleeps at a time while a frame is not due, so that
/// it hears of the end.
const A_NAP: Duration = Duration::from_millis(20);

/// What stands under the picture.
#[derive(Default)]
pub struct Under {
    pub chat: Vec<String>,
    /// The keys, or the line that is being typed.
    pub status: String,
    /// Counted up with every change.
    pub version: u64,
}

/// A frame of the video track.
pub struct Frame {
    pub group: u64,
    pub object: u64,
    /// The frame as a byte stream with start codes.
    pub annex_b: Vec<u8>,
    pub due: Instant,
}

/// What the picture did.
#[derive(Default)]
pub struct Seen {
    pub came: u64,
    /// Frames before the first key frame, and the rest of a Group after a
    /// frame that did not come.
    pub left_out: u64,
    /// Frames whose picture has more pixels than are handled, and the size
    /// of the last of them.
    pub too_large: u64,
    pub too_large_size: (usize, usize),
    pub decoded: u32,
    pub drawn: u32,
    pub decoding: Duration,
    pub drawing: Duration,
    /// For the last frames drawn: how long after it was due each was
    /// written, in microseconds.
    pub after_due_us: Vec<i64>,
    /// With `--measure-demo-marks`: when a frame with the white square was
    /// written after one without.
    pub marks_us: Vec<u64>,
    /// Bytes written to the terminal, and to the files it reads.
    pub bytes: u64,
    /// The size of the last frame, and by how much it was made smaller.
    pub size: (usize, usize),
    pub by: usize,
    pub shown_for: Duration,
}

/// Where the picture goes.
pub struct Screen {
    pub to: Box<dyn Write + Send>,
    pub medium: Medium,
    pub scale: usize,
    /// The most pixels that are written; `None`: the frame as it is.
    pub fit: Option<(usize, usize)>,
    /// The pixels of a cell of the terminal.
    pub cell: (usize, usize),
    pub under: Arc<Mutex<Under>>,
}

/// The screen while frames are written to it.
struct Writing {
    screen: Screen,
    /// The size and the rows of the first frame: one of another size is
    /// left out.
    placed: Option<((usize, usize), usize)>,
    shrunk: Vec<u8>,
    out: Vec<u8>,
    /// The version of what stands under the picture that is written.
    under: u64,
    /// The files of pixels that may still be there, the oldest first.
    files: VecDeque<PathBuf>,
    written: u64,
    /// What the names of the files cannot be guessed by.
    secret: u64,
    bytes: u64,
    by: usize,
}

impl Writing {
    /// Writes one frame over the one before. Whether it was written.
    fn draw(&mut self, rgb: &[u8], full: (usize, usize)) -> std::io::Result<bool> {
        self.by = self.screen.fit.map_or(1, |most| fit::by(full, most));
        let mut shrunk = std::mem::take(&mut self.shrunk);
        let (rgb, size) = if self.by > 1 {
            let size = fit::shrink(rgb, full, self.by, &mut shrunk);
            (shrunk.as_slice(), size)
        } else {
            (rgb, full)
        };
        let drawn = self.write(rgb, size);
        self.shrunk = shrunk;
        drawn
    }

    fn write(&mut self, rgb: &[u8], size: (usize, usize)) -> std::io::Result<bool> {
        let cells = fit::cells(size, self.screen.scale, self.screen.cell);
        self.out.clear();
        let rows = match self.placed {
            Some((placed, rows)) if placed == size => rows,
            Some(_) => return Ok(false),
            None => {
                kitty::reserve(cells.1, LINES, &mut self.out);
                self.placed = Some((size, cells.1));
                cells.1
            }
        };
        match self.screen.medium {
            Medium::Direct => kitty::frame_direct(rgb, size, cells, &mut self.out),
            Medium::File => {
                // The name has to hold `tty-graphics-protocol` for the
                // terminal to remove the file when it has read it.
                // WHY a name nobody can guess, made anew and for this user
                // alone: the directory is everybody's, and a file that is
                // written by its name there follows a link somebody put in
                // its place.
                let path = std::env::temp_dir().join(format!(
                    "tty-graphics-protocol-xmoq-{}-{:016x}-{}.rgb",
                    std::process::id(),
                    self.secret,
                    self.written
                ));
                self.written += 1;
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)?
                    .write_all(rgb)?;
                self.bytes += u64::try_from(rgb.len()).unwrap_or(u64::MAX);
                kitty::frame_file(&path.to_string_lossy(), size, cells, &mut self.out);
                self.files.push_back(path);
                // Where no terminal removes them (it is slow, or there is
                // none), the oldest goes; one that is gone already is fine.
                while self.files.len() > MOST_FILES {
                    if let Some(oldest) = self.files.pop_front() {
                        let _ = std::fs::remove_file(oldest);
                    }
                }
            }
        }
        // What stands under the picture, when it has changed. WHY with the
        // frame: one writer, and the cursor is put back to the picture
        // before every frame.
        {
            let under = self
                .screen
                .under
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if under.version != self.under {
                self.under = under.version;
                let chat = under.chat.len().saturating_sub(LINES - 1);
                let lines = under
                    .chat
                    .iter()
                    .skip(chat)
                    .map(String::as_str)
                    .chain(std::iter::once(under.status.as_str()));
                kitty::lines_under(rows, lines, &mut self.out);
            }
        }
        self.screen.to.write_all(&self.out)?;
        self.screen.to.flush()?;
        self.bytes += u64::try_from(self.out.len()).unwrap_or(u64::MAX);
        Ok(true)
    }

    fn leave(&mut self) {
        for file in self.files.drain(..) {
            let _ = std::fs::remove_file(file);
        }
        if let Some((_, rows)) = self.placed {
            self.out.clear();
            kitty::leave(rows, LINES, &mut self.out);
            let _ = self.screen.to.write_all(&self.out);
            let _ = self.screen.to.flush();
        }
    }
}

/// What the thread of the picture is started with.
pub struct Picture {
    pub frames: mpsc::Receiver<Frame>,
    pub screen: Screen,
    pub decoder: VideoDecoder,
    /// Times are counted from here.
    pub began: Instant,
    pub measure_marks: bool,
    /// Set when the thread is to end without drawing what waits.
    pub stop: Arc<AtomicBool>,
}

/// Decodes and draws what comes on `frames`, until the sender is gone or
/// the thread is told to stop.
pub fn run(picture: Picture) -> Seen {
    let Picture {
        frames,
        screen,
        mut decoder,
        began,
        measure_marks,
        stop,
    } = picture;
    let mut seen = Seen {
        by: 1,
        ..Seen::default()
    };
    let mut writing = Writing {
        screen,
        placed: None,
        shrunk: Vec::new(),
        out: Vec::new(),
        under: 0,
        files: VecDeque::new(),
        written: 0,
        secret: std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish(),
        bytes: 0,
        by: 1,
    };
    let mut rgb = Vec::new();
    // The Object the decoder can take next.
    let mut next: Option<(u64, u64)> = None;
    let mut was_marked = false;
    let mut shown: Option<(Instant, Instant)> = None;
    let mut after_due = Recent::new(MOST_COUNTED);
    'frames: for frame in frames {
        seen.came += 1;
        let follows = next == Some((frame.group, frame.object));
        let begins = frame.object == 0 && next.is_none_or(|(group, _)| frame.group >= group);
        if !follows && !begins {
            seen.left_out += 1;
            continue;
        }
        next = Some((frame.group, frame.object + 1));
        let before = Instant::now();
        let picture = decoder.decode(&frame.annex_b, &mut rgb);
        seen.decoding += before.elapsed();
        let size = match picture {
            Decoded::Picture(size) => size,
            Decoded::TooLarge(size) => {
                seen.too_large += 1;
                seen.too_large_size = size;
                continue;
            }
            Decoded::Nothing => continue,
        };
        seen.decoded += 1;
        seen.size = size;
        // WHY in naps: the end must not wait for a frame to be due.
        while let Some(wait) = frame.due.checked_duration_since(Instant::now()) {
            if stop.load(Ordering::Relaxed) {
                break 'frames;
            }
            std::thread::sleep(wait.min(A_NAP));
        }
        let before = Instant::now();
        match writing.draw(&rgb, size) {
            Ok(true) => {}
            Ok(false) => continue,
            // The terminal is gone.
            Err(_) => break,
        }
        let now = Instant::now();
        seen.drawing += now - before;
        seen.drawn += 1;
        shown = Some((shown.map_or(now, |(first, _)| first), now));
        after_due.push(
            i64::try_from(micros_since(frame.due, now)).unwrap_or(i64::MAX)
                - i64::try_from(micros_since(now, frame.due)).unwrap_or(i64::MAX),
        );
        if measure_marks {
            let marked = marks::picture_is_marked(&rgb, size);
            if marked && !was_marked && seen.marks_us.len() < MOST_COUNTED {
                seen.marks_us.push(micros_since(began, now));
            }
            was_marked = marked;
        }
    }
    writing.leave();
    seen.after_due_us = after_due.to_vec();
    seen.bytes = writing.bytes;
    seen.by = writing.by;
    seen.shown_for = shown.map_or(Duration::ZERO, |(first, last)| last - first);
    seen
}
