//! Decodes an H.264 byte stream (Annex B) with the `openh264` crate, which
//! builds Cisco's OpenH264 from source, and says how many frames came out.
//!
//!   h264-decoders <file.h264> [--rgb]     # --rgb: raw rgb24 frames to stdout

use std::io::Write;
use std::time::Instant;

use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a file");
    let rgb_out = args.next().as_deref() == Some("--rgb");
    let bytes = std::fs::read(&path).expect("read");
    let mut decoder = Decoder::new().expect("decoder");
    let (mut frames, mut errors, mut size) = (0u64, 0u64, (0, 0));
    let mut rgb = Vec::new();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let started = Instant::now();
    for nal in openh264::nal_units(&bytes) {
        match decoder.decode(nal) {
            Ok(Some(yuv)) => {
                frames += 1;
                size = yuv.dimensions();
                if rgb_out {
                    rgb.resize(size.0 * size.1 * 3, 0);
                    yuv.write_rgb8(&mut rgb);
                    out.write_all(&rgb).expect("write");
                }
            }
            Ok(None) => {}
            Err(error) => {
                errors += 1;
                if errors <= 3 {
                    eprintln!("error: {error}");
                }
            }
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    eprintln!(
        "OPENH264 file={path} bytes={} frames={frames} errors={errors} size={}x{} ms_a_frame={:.2}",
        bytes.len(),
        size.0,
        size.1,
        seconds * 1000.0 / frames.max(1) as f64
    );
}
