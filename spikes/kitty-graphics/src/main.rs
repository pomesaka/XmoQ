//! Draws RGB frames in a terminal with the Kitty graphics protocol.
//!
//!   kitty-frames --pattern 320x180 --frames 250 --fps 25
//!   ffmpeg ... -f rawvideo -pix_fmt rgb24 pipe:1 | kitty-frames --raw 320x180
//!
//! `--medium direct` (the default) sends the pixels in the escape codes,
//! base64 in chunks of 4096; `--medium file` writes them to a temporary file
//! the terminal reads and deletes (t=t), which only a terminal on the same
//! machine can do. `--scale N` has the terminal draw each pixel N times as
//! wide and high (the image is placed over that many cells by its size in
//! pixels; the terminal scales). Numbers of the run go to stderr at the end.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

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
enum Medium {
    Direct,
    File,
}

struct Screen {
    width: usize,
    height: usize,
    medium: Medium,
    /// Columns and rows the image is drawn over; 0 leaves it to its pixels.
    cells: (usize, usize),
    encoded: Vec<u8>,
    out: Vec<u8>,
    drawn: u64,
    bytes: u64,
}

impl Screen {
    /// One frame: the image with the id 1 is sent anew, which replaces the
    /// one before it, at the place the cursor was saved at.
    fn draw(&mut self, rgb: &[u8], to: &mut impl Write) -> io::Result<()> {
        self.out.clear();
        // Back to where the first frame began.
        self.out.extend_from_slice(b"\x1b8");
        let mut keys = format!("a=T,f=24,s={},v={},i=1,p=1,q=2,C=1", self.width, self.height);
        if self.cells != (0, 0) {
            keys.push_str(&format!(",c={},r={}", self.cells.0, self.cells.1));
        }
        match self.medium {
            Medium::Direct => {
                self.encoded.clear();
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
                // The protocol has the terminal delete a file of t=t only
                // where its path has this string in it.
                let path = std::env::temp_dir()
                    .join(format!("tty-graphics-protocol-xmoq-{}-{}.rgb", std::process::id(), self.drawn));
                std::fs::write(&path, rgb)?;
                self.encoded.clear();
                base64(path.to_string_lossy().as_bytes(), &mut self.encoded);
                self.out.extend_from_slice(format!("\x1b_G{keys},t=t;").as_bytes());
                self.out.extend_from_slice(&self.encoded);
                self.out.extend_from_slice(b"\x1b\\");
            }
        }
        to.write_all(&self.out)?;
        to.flush()?;
        self.drawn += 1;
        self.bytes += self.out.len() as u64;
        Ok(())
    }
}

/// Frame `n` of the pattern: a colour ramp, and a white bar that moves 4
/// pixels a frame, so that a frame that is missing or late is seen.
fn pattern(n: usize, width: usize, height: usize, rgb: &mut [u8]) {
    let bar = (n * 4) % width;
    for y in 0..height {
        for x in 0..width {
            let at = (y * width + x) * 3;
            let on_bar = x >= bar && x < bar + 8;
            rgb[at] = if on_bar { 255 } else { (x * 255 / width) as u8 };
            rgb[at + 1] = if on_bar { 255 } else { (y * 255 / height) as u8 };
            rgb[at + 2] = if on_bar { 255 } else { (n * 5 % 256) as u8 };
        }
    }
}

fn size(text: &str) -> (usize, usize) {
    let (w, h) = text.split_once('x').expect("a size is <width>x<height>");
    (w.parse().expect("width"), h.parse().expect("height"))
}

fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let (mut raw, mut frames, mut fps, mut medium, mut scale) = (false, 250usize, 25u64, Medium::Direct, 0usize);
    let (mut width, mut height) = (320, 180);
    // Width and height of a cell in pixels, for --scale: what Ghostty has
    // with its default font here is not known to this program.
    let mut cell = (8usize, 16usize);
    while let Some(flag) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--pattern" => (width, height) = size(&value()),
            "--raw" => {
                raw = true;
                (width, height) = size(&value());
            }
            "--frames" => frames = value().parse().expect("--frames"),
            "--fps" => fps = value().parse().expect("--fps"),
            "--scale" => scale = value().parse().expect("--scale"),
            "--cell" => cell = size(&value()),
            "--medium" => {
                medium = match value().as_str() {
                    "direct" => Medium::Direct,
                    "file" => Medium::File,
                    other => panic!("--medium {other}: direct or file"),
                }
            }
            other => panic!("{other}: not a flag of kitty-frames"),
        }
    }
    let cells = if scale == 0 {
        (0, 0)
    } else {
        ((width * scale).div_ceil(cell.0), (height * scale).div_ceil(cell.1))
    };
    let mut screen =
        Screen { width, height, medium, cells, encoded: Vec::new(), out: Vec::new(), drawn: 0, bytes: 0 };
    let mut rgb = vec![0u8; width * height * 3];
    let stdout = io::stdout();
    let mut to = stdout.lock();
    // Room for the picture below the cursor, then the cursor is saved at
    // its top left corner.
    let rows = if cells.1 == 0 { height.div_ceil(cell.1) } else { cells.1 };
    to.write_all("\n".repeat(rows).as_bytes())?;
    to.write_all(format!("\x1b[{rows}A\x1b7").as_bytes())?;
    let started = Instant::now();
    let mut spent = Duration::ZERO;
    if raw {
        let stdin = io::stdin();
        let mut from = stdin.lock();
        // Until the input ends: a frame that is cut is not drawn.
        while from.read_exact(&mut rgb).is_ok() {
            let before = Instant::now();
            screen.draw(&rgb, &mut to)?;
            spent += before.elapsed();
        }
    } else {
        for n in 0..frames {
            pattern(n, width, height, &mut rgb);
            let before = Instant::now();
            screen.draw(&rgb, &mut to)?;
            spent += before.elapsed();
            // The time frame n + 1 is due at, from the start.
            let due = Duration::from_micros((n as u64 + 1) * 1_000_000 / fps.max(1));
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
        }
    }
    let elapsed = started.elapsed();
    // Below the picture.
    to.write_all(format!("\x1b8\x1b[{rows}B\n").as_bytes())?;
    to.flush()?;
    eprintln!(
        "KITTY_FRAMES size={width}x{height} medium={} frames={} seconds={:.2} fps={:.1} bytes_a_frame={} ms_a_frame_to_encode_and_write={:.2}",
        if medium == Medium::Direct { "direct" } else { "file" },
        screen.drawn,
        elapsed.as_secs_f64(),
        screen.drawn as f64 / elapsed.as_secs_f64(),
        screen.bytes / screen.drawn.max(1),
        spent.as_secs_f64() * 1000.0 / screen.drawn.max(1) as f64,
    );
    Ok(())
}
