//! The picture: frames decoded as they come, each written for the terminal
//! at the moment it is due, on a thread of its own.

use std::io::Write;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use xmoq_watch::args::Medium;
use xmoq_watch::{fit, kitty, marks};

use crate::sound::micros_since;
use crate::video_decoder::VideoDecoder;

/// Lines under the picture: the last of the chat, and one for the keys.
pub const LINES: usize = 4;

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
    pub decoded: u32,
    pub drawn: u32,
    pub decoding: Duration,
    pub drawing: Duration,
    /// For every frame drawn: how long after it was due it was written, in
    /// microseconds.
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
    files: u64,
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
                let name = |number: u64| {
                    std::env::temp_dir().join(format!(
                        "tty-graphics-protocol-xmoq-{}-{number}.rgb",
                        std::process::id()
                    ))
                };
                let path = name(self.files);
                std::fs::write(&path, rgb)?;
                self.bytes += u64::try_from(rgb.len()).unwrap_or(u64::MAX);
                // For a run nobody watches: the file before this one is
                // removed, as a terminal would have removed it.
                if self.files > 0 && std::env::var_os("XMOQ_WATCH_REMOVES_FILES").is_some() {
                    let _ = std::fs::remove_file(name(self.files - 1));
                }
                self.files += 1;
                kitty::frame_file(&path.to_string_lossy(), size, cells, &mut self.out);
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
        if let Some((_, rows)) = self.placed {
            self.out.clear();
            kitty::leave(rows, LINES, &mut self.out);
            let _ = self.screen.to.write_all(&self.out);
            let _ = self.screen.to.flush();
        }
    }
}

/// Decodes and draws what comes on `frames`, until the sender is gone.
/// Times are counted from `began`.
pub fn run(
    frames: mpsc::Receiver<Frame>,
    screen: Screen,
    mut decoder: VideoDecoder,
    began: Instant,
    measure_marks: bool,
) -> Seen {
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
        files: 0,
        bytes: 0,
        by: 1,
    };
    let mut rgb = Vec::new();
    // The Object the decoder can take next.
    let mut next: Option<(u64, u64)> = None;
    let mut was_marked = false;
    let mut shown: Option<(Instant, Instant)> = None;
    for frame in frames {
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
        let Some(size) = picture else { continue };
        seen.decoded += 1;
        seen.size = size;
        if let Some(wait) = frame.due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
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
        let after_due = i64::try_from(micros_since(frame.due, now)).unwrap_or(i64::MAX)
            - i64::try_from(micros_since(now, frame.due)).unwrap_or(i64::MAX);
        seen.after_due_us.push(after_due);
        if measure_marks {
            let marked = marks::picture_is_marked(&rgb, size);
            if marked && !was_marked {
                seen.marks_us.push(micros_since(began, now));
            }
            was_marked = marked;
        }
    }
    writing.leave();
    seen.bytes = writing.bytes;
    seen.by = writing.by;
    seen.shown_for = shown.map_or(Duration::ZERO, |(first, last)| last - first);
    seen
}
