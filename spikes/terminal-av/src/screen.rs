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
}

impl Screen {
    pub fn new(to: Box<dyn Write + Send>, medium: Medium, scale: usize, under: Arc<Mutex<Under>>) -> Self {
        Self { to, medium, scale, size: None, rows: 0, encoded: Vec::new(), out: Vec::new(), drawn: 0, under, written: 0 }
    }

    /// Draws one frame of RGB over the one before. A frame of another size
    /// than the first is left out.
    pub fn draw(&mut self, rgb: &[u8], (width, height): (usize, usize)) -> io::Result<()> {
        // A cell of 8x16 pixels is assumed, as in spikes/kitty-graphics.
        let cells = ((width * self.scale).div_ceil(8), (height * self.scale).div_ceil(16));
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
