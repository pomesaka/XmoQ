//! The picture: frames of H.264 decoded with the crate `openh264` as they
//! come, each drawn at the moment it is due.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

use crate::screen::Screen;

pub struct Frame {
    pub group: u64,
    pub object: u64,
    pub annex_b: Vec<u8>,
    pub due: Instant,
}

#[derive(Default)]
pub struct Counts {
    pub came: u64,
    /// Frames before the first key frame, and the rest of a Group after a
    /// frame that did not come.
    pub left_out: u64,
    pub decoded: u64,
    pub drawn: u64,
    pub decoding: Duration,
    pub drawing: Duration,
    /// For every frame drawn: the moment it was written to the terminal,
    /// less the moment it was due, in milliseconds.
    pub off_ms: Vec<f32>,
    /// The moments a frame with the white square of the demo was drawn
    /// after one without.
    pub marks: Vec<Instant>,
    /// What was written for the terminal, and by how much the frames were made smaller.
    pub bytes: u64,
    pub by: usize,
    pub size: (usize, usize),
    pub first: Option<Instant>,
    pub last: Option<Instant>,
}

fn start_code(bytes: &[u8], from: usize) -> Option<usize> {
    bytes.get(from..)?.windows(3).position(|w| w == [0, 0, 1]).map(|at| at + from)
}

/// The white square of the demo (`MARK` in demo/shared.ts), by its middle.
fn marked(rgb: &[u8], (width, height): (usize, usize)) -> bool {
    // The page paints in the units of 640x360 and sends at any size.
    let (x, y) = (580 * width / 640, 60 * height / 360);
    if x >= width || y >= height {
        return false;
    }
    let at = (y * width + x) * 3;
    rgb[at..at + 3].iter().all(|value| *value > 200)
}

/// Decodes and draws what comes on `frames`, until the sender is gone.
pub fn run(frames: mpsc::Receiver<Frame>, mut screen: Screen) -> Counts {
    let mut counts = Counts::default();
    let mut decoder = Decoder::new().expect("the decoder of openh264");
    let mut rgb = Vec::new();
    // The Object the decoder can take next.
    let mut next: Option<(u64, u64)> = None;
    let mut was_marked = false;
    for frame in frames {
        counts.came += 1;
        let follows = next == Some((frame.group, frame.object));
        let begins = frame.object == 0 && next.is_none_or(|(group, _)| frame.group >= group);
        if !follows && !begins {
            counts.left_out += 1;
            continue;
        }
        next = Some((frame.group, frame.object + 1));
        // One NAL unit at a time, as spikes/kitty-graphics feeds the decoder.
        let before = Instant::now();
        let mut picture = None;
        let mut at = start_code(&frame.annex_b, 0);
        while let Some(first) = at {
            let following = start_code(&frame.annex_b, first + 3);
            let end = following.map_or(frame.annex_b.len(), |next| if frame.annex_b[next - 1] == 0 { next - 1 } else { next });
            match decoder.decode(&frame.annex_b[first..end]) {
                Ok(Some(yuv)) => {
                    let size = yuv.dimensions();
                    rgb.resize(size.0 * size.1 * 3, 0);
                    yuv.write_rgb8(&mut rgb);
                    picture = Some(size);
                }
                Ok(None) => {}
                Err(error) => eprintln!("openh264: {error}"),
            }
            at = following;
        }
        counts.decoding += before.elapsed();
        let Some(size) = picture else { continue };
        counts.decoded += 1;
        counts.size = size;
        if let Some(wait) = frame.due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let before = Instant::now();
        if screen.draw(&rgb, size).is_err() {
            break;
        }
        let now = Instant::now();
        counts.drawing += now - before;
        counts.drawn += 1;
        counts.first.get_or_insert(now);
        counts.last = Some(now);
        counts.off_ms.push(if now >= frame.due {
            (now - frame.due).as_secs_f32() * 1000.0
        } else {
            -((frame.due - now).as_secs_f32() * 1000.0)
        });
        let mark = marked(&rgb, size);
        if mark && !was_marked {
            counts.marks.push(now);
        }
        was_marked = mark;
    }
    screen.leave();
    counts.bytes = screen.bytes;
    counts.by = screen.by;
    counts
}
