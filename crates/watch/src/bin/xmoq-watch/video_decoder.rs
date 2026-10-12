//! The decoder of the picture: H.264 with the crate `openh264`. The one
//! file that names that crate; which decoder the viewer uses is not decided
//! (`docs/open-questions.md`).

use anyhow::{Result, anyhow};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use xmoq_watch::fit::too_large;

/// A decoder of H.264 that gives pictures of 3 bytes a pixel.
pub struct VideoDecoder(Decoder);

/// What a frame gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded {
    /// A picture of this size, in the buffer that was given.
    Picture((usize, usize)),
    /// A picture of more pixels than are handled
    /// (`xmoq_watch::fit::MOST_PIXELS`); the buffer is as it was.
    TooLarge((usize, usize)),
    /// No picture.
    Nothing,
}

/// Where the next start code of three bytes begins, from `from` on.
fn start_code(bytes: &[u8], from: usize) -> Option<usize> {
    bytes
        .get(from..)?
        .windows(3)
        .position(|window| window == [0, 0, 1])
        .map(|at| at + from)
}

impl VideoDecoder {
    /// A decoder with nothing decoded yet.
    pub fn new() -> Result<Self> {
        Ok(Self(Decoder::new().map_err(|error| {
            anyhow!("the decoder of openh264: {error}")
        })?))
    }

    /// Decodes one frame, a byte stream with start codes, into `rgb`.
    ///
    /// WHY one NAL unit at a time: that is what the decoder was seen to
    /// take (`spikes/kitty-graphics`); a frame with its parameter sets
    /// before it is several.
    pub fn decode(&mut self, annex_b: &[u8], rgb: &mut Vec<u8>) -> Decoded {
        let mut picture = Decoded::Nothing;
        let mut at = start_code(annex_b, 0);
        while let Some(first) = at {
            let following = start_code(annex_b, first + 3);
            // A start code of four bytes: the zero before the three belongs to it.
            let end = following.map_or(annex_b.len(), |next| {
                if next > 0 && annex_b.get(next - 1) == Some(&0) {
                    next - 1
                } else {
                    next
                }
            });
            // A unit that gives no picture, or one the decoder turns down,
            // is passed over: the frames after it may still decode.
            if let Some(Ok(Some(yuv))) = annex_b.get(first..end).map(|nal| self.0.decode(nal)) {
                let size = yuv.dimensions();
                // WHY looked at before the pixels are asked for: 3 bytes
                // are kept for each, and the size is the stream's to say.
                // What the decoder holds itself for such a picture is not
                // bounded here.
                if too_large(size) {
                    picture = Decoded::TooLarge(size);
                } else {
                    rgb.resize(size.0 * size.1 * 3, 0);
                    yuv.write_rgb8(rgb);
                    picture = Decoded::Picture(size);
                }
            }
            at = following;
        }
        picture
    }
}
