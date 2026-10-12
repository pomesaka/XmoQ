//! A picture in the terminal with the Kitty graphics protocol, as in
//! spikes/kitty-graphics: one image that every frame replaces.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

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

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8], out: &mut Vec<u8>) {
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(BASE64[(n >> 18) as usize & 63]);
        out.push(BASE64[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 { BASE64[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if chunk.len() > 2 { BASE64[n as usize & 63] } else { b'=' });
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Medium {
    /// The pixels in the escape codes.
    Direct,
    /// A temporary file the terminal reads and removes: the same machine.
    File,
}

pub struct Screen {
    to: Box<dyn Write + Send>,
    medium: Medium,
    scale: usize,
    size: Option<(usize, usize)>,
    rows: usize,
    encoded: Vec<u8>,
    out: Vec<u8>,
    pub drawn: u64,
    under: Arc<Mutex<Under>>,
    /// The version of `under` that is written.
    written: u64,
    /// The most pixels that are written, and the pixels of a cell.
    fit: Option<(usize, usize)>,
    cell: (usize, usize),
    shrunk: Vec<u8>,
    /// Bytes written to the terminal, and to the files it reads.
    pub bytes: u64,
    /// By how much the frames are made smaller, each way.
    pub by: usize,
}

/// What the terminal says of itself: the pixels of its window and of a cell.
/// None where stdout is no terminal, or it does not say its pixels.
pub fn terminal() -> Option<((usize, usize), (usize, usize))> {
    // SAFETY: an all-zero winsize is a valid value for the ioctl to fill.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: file descriptor 1 and a winsize of this frame.
    if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut size) } != 0 || size.ws_col == 0 || size.ws_row == 0 || size.ws_xpixel == 0 {
        return None;
    }
    let (columns, rows) = (usize::from(size.ws_col), usize::from(size.ws_row));
    let (width, height) = (usize::from(size.ws_xpixel), usize::from(size.ws_ypixel));
    let cell = ((width / columns).max(1), (height / rows).max(1));
    // The lines under the picture, and one for the prompt the viewer was started from.
    Some(((width, height.saturating_sub((LINES + 1) * cell.1).max(cell.1)), cell))
}

/// Makes a picture smaller by a whole number each way: every pixel of the
/// result is the mean of a square of the original.
fn shrink(rgb: &[u8], (width, height): (usize, usize), by: usize, out: &mut Vec<u8>) -> (usize, usize) {
    let (w, h) = (width / by, height / by);
    out.clear();
    out.reserve(w * h * 3);
    let area = (by * by) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 3];
            for row in 0..by {
                let at = ((y * by + row) * width + x * by) * 3;
                for pixel in rgb[at..at + by * 3].chunks_exact(3) {
                    sum[0] += u32::from(pixel[0]);
                    sum[1] += u32::from(pixel[1]);
                    sum[2] += u32::from(pixel[2]);
                }
            }
            out.extend_from_slice(&[(sum[0] / area) as u8, (sum[1] / area) as u8, (sum[2] / area) as u8]);
        }
    }
    (w, h)
}

impl Screen {
    pub fn new(to: Box<dyn Write + Send>, medium: Medium, scale: usize, under: Arc<Mutex<Under>>) -> Self {
        Self {
            to,
            medium,
            scale,
            size: None,
            rows: 0,
            encoded: Vec::new(),
            out: Vec::new(),
            drawn: 0,
            under,
            written: 0,
            fit: None,
            cell: (8, 16),
            shrunk: Vec::new(),
            bytes: 0,
            by: 1,
        }
    }

    /// Frames of more pixels than `most` are made smaller before they are
    /// written; `cell` is what a cell of the terminal has.
    pub fn fit(mut self, most: (usize, usize), cell: (usize, usize)) -> Self {
        self.fit = Some(most);
        self.cell = cell;
        self
    }

    /// Draws one frame of RGB over the one before. A frame of another size
    /// than the first is left out.
    pub fn draw(&mut self, rgb: &[u8], full: (usize, usize)) -> io::Result<()> {
        // WHY a whole number: every pixel written is then the mean of as many of the frame,
        // with no weights to work out for each frame.
        self.by = self.fit.map_or(1, |most| full.0.div_ceil(most.0).max(full.1.div_ceil(most.1)).max(1));
        let mut shrunk = std::mem::take(&mut self.shrunk);
        let (rgb, (width, height)) = if self.by > 1 {
            let size = shrink(rgb, full, self.by, &mut shrunk);
            (shrunk.as_slice(), size)
        } else {
            (rgb, full)
        };
        let drawn = self.write(rgb, (width, height));
        self.shrunk = shrunk;
        drawn
    }

    fn write(&mut self, rgb: &[u8], (width, height): (usize, usize)) -> io::Result<()> {
        let cells = ((width * self.scale).div_ceil(self.cell.0), (height * self.scale).div_ceil(self.cell.1));
        if self.size.is_none() {
            self.size = Some((width, height));
            self.rows = cells.1;
            self.to.write_all("\n".repeat(self.rows + LINES).as_bytes())?;
            self.to.write_all(format!("\x1b[{}A\x1b7", self.rows + LINES).as_bytes())?;
        }
        if self.size != Some((width, height)) {
            return Ok(());
        }
        self.out.clear();
        self.out.extend_from_slice(b"\x1b8");
        let keys = format!("a=T,f=24,s={width},v={height},i=1,p=1,q=2,C=1,c={},r={}", cells.0, cells.1);
        self.encoded.clear();
        match self.medium {
            Medium::Direct => {
                base64(rgb, &mut self.encoded);
                let mut chunks = self.encoded.chunks(4096).peekable();
                let mut first = true;
                while let Some(chunk) = chunks.next() {
                    let more = u8::from(chunks.peek().is_some());
                    if first {
                        self.out.extend_from_slice(format!("\x1b_G{keys},m={more};").as_bytes());
                        first = false;
                    } else {
                        self.out.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
                    }
                    self.out.extend_from_slice(chunk);
                    self.out.extend_from_slice(b"\x1b\\");
                }
            }
            Medium::File => {
                // The name has to hold `tty-graphics-protocol` for the terminal to remove it.
                let path = std::env::temp_dir()
                    .join(format!("tty-graphics-protocol-xmoq-{}-{}.rgb", std::process::id(), self.drawn));
                std::fs::write(&path, rgb)?;
                self.bytes += rgb.len() as u64;
                // For a run nobody watches: the file before this one is removed, as a
                // terminal would have removed it when it had read it.
                if self.drawn > 0 && std::env::var_os("XMOQ_WATCH_REMOVES_FILES").is_some() {
                    let before = std::env::temp_dir()
                        .join(format!("tty-graphics-protocol-xmoq-{}-{}.rgb", std::process::id(), self.drawn - 1));
                    let _ = std::fs::remove_file(before);
                }
                base64(path.to_string_lossy().as_bytes(), &mut self.encoded);
                self.out.extend_from_slice(format!("\x1b_G{keys},t=t;").as_bytes());
                self.out.extend_from_slice(&self.encoded);
                self.out.extend_from_slice(b"\x1b\\");
            }
        }
        // What stands under the picture, when it has changed. WHY with the frame: one
        // writer, and the cursor is put back to the picture before every frame.
        let under = self.under.lock().expect("the lock");
        if under.version != self.written {
            self.written = under.version;
            self.out.extend_from_slice(format!("\x1b8\x1b[{}B", self.rows).as_bytes());
            let chat = under.chat.len().saturating_sub(LINES - 1);
            for line in under.chat[chat..].iter().chain(std::iter::once(&under.status)) {
                self.out.extend_from_slice(format!("\r\x1b[K{line}\x1b[B").as_bytes());
            }
        }
        drop(under);
        self.to.write_all(&self.out)?;
        self.to.flush()?;
        self.bytes += self.out.len() as u64;
        self.drawn += 1;
        Ok(())
    }

    /// Leaves the cursor under the picture.
    pub fn leave(&mut self) {
        if self.size.is_some() {
            let _ = self.to.write_all(format!("\x1b8\x1b[{}B\n", self.rows + LINES).as_bytes());
            let _ = self.to.flush();
        }
    }
}
