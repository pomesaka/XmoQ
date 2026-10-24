//! How large a picture is let be, and making it smaller by a whole number
//! each way.

/// The most pixels of a decoded picture that are handled: those of
/// 3840x2160. **Provisional.**
///
/// WHY: a picture's size is what its stream says, and 3 bytes are kept for
/// every pixel of it. The largest the demo's publisher sends is 1920x1080;
/// four times that is as far as a terminal's window goes.
pub const MOST_PIXELS: usize = 3840 * 2160;

/// Whether a picture of this size is more than is handled
/// ([`MOST_PIXELS`]).
#[must_use]
pub fn too_large(size: (usize, usize)) -> bool {
    size.0
        .checked_mul(size.1)
        .is_none_or(|pixels| pixels > MOST_PIXELS)
}

/// By how much a picture of `full` pixels is made smaller, each way, for it
/// to have no more than `most`: 1 for a picture that fits.
///
/// WHY a whole number: every pixel that is written is then the mean of as
/// many of the frame, with no weights to work out for each frame. What it
/// costs: a picture can come out well under the limit (1920x1080 within
/// 1280x1280 is 960x540).
#[must_use]
pub fn by(full: (usize, usize), most: (usize, usize)) -> usize {
    let each = |full: usize, most: usize| full.div_ceil(most.max(1));
    each(full.0, most.0).max(each(full.1, most.1)).max(1)
}

/// Makes a picture of 3 bytes a pixel smaller by `by` each way, into `out`:
/// every pixel of the result is the mean of a square of the original. What
/// is left over at the right and at the bottom is left out. Says the size
/// of the result.
///
/// A picture whose bytes are fewer than its size asks for, or a `by` of 0,
/// gives a picture of nothing.
pub fn shrink(rgb: &[u8], full: (usize, usize), by: usize, out: &mut Vec<u8>) -> (usize, usize) {
    out.clear();
    let enough = full
        .0
        .checked_mul(full.1)
        .and_then(|pixels| pixels.checked_mul(3))
        .is_some_and(|bytes| bytes <= rgb.len());
    let Ok(area) = u32::try_from(by.saturating_mul(by)) else {
        return (0, 0);
    };
    if !enough || area == 0 {
        return (0, 0);
    }
    let (width, height) = (full.0 / by, full.1 / by);
    out.reserve(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0u32; 3];
            for row in 0..by {
                let begins = ((y * by + row) * full.0 + x * by) * 3;
                let square = rgb.get(begins..begins + by * 3).unwrap_or_default();
                for pixel in square.as_chunks::<3>().0 {
                    for (sum, value) in sum.iter_mut().zip(pixel) {
                        *sum += u32::from(*value);
                    }
                }
            }
            out.extend(sum.map(|sum| u8::try_from(sum / area).unwrap_or(u8::MAX)));
        }
    }
    (width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_made_smaller_by_the_least_whole_number_that_fits() {
        // (the picture; the most; by how much)
        let cases = [
            ((640, 360), (640, 360), 1),
            ((640, 360), (1920, 1080), 1),
            ((1280, 720), (640, 360), 2),
            ((1920, 1080), (960, 540), 2),
            ((1920, 1080), (640, 360), 3),
            // One pixel too many is the next whole number.
            ((1921, 1080), (960, 540), 3),
            // The narrower of the two limits decides.
            ((1920, 1080), (1920, 100), 11),
            ((1920, 1080), (100, 1080), 20),
            // A longest side of 1280, as the agent's default.
            ((1920, 1080), (1280, 1280), 2),
            ((1280, 720), (1280, 1280), 1),
            ((1080, 1920), (1280, 1280), 2),
            ((2560, 1440), (1280, 1280), 2),
            // A limit of nothing is taken for one pixel.
            ((1920, 1080), (0, 0), 1920),
            ((0, 0), (640, 360), 1),
        ];
        for (full, most, expected) in cases {
            assert_eq!(by(full, most), expected, "{full:?} into {most:?}");
        }
    }

    #[test]
    fn a_picture_of_more_pixels_than_3840x2160_is_too_large() {
        // (the size; whether it is too large)
        let cases = [
            ((0, 0), false),
            ((1920, 1080), false),
            ((3840, 2160), false),
            ((2160, 3840), false),
            ((3841, 2160), true),
            ((3840, 2161), true),
            ((16_384, 16_384), true),
            ((usize::MAX, 2), true),
        ];
        for (size, expected) in cases {
            assert_eq!(too_large(size), expected, "{size:?}");
        }
    }

    #[test]
    fn a_smaller_picture_has_the_mean_of_every_square() {
        // 4x2 pixels, each a shade of grey but the last, which is red.
        let picture = [
            10, 10, 10, 20, 20, 20, 0, 0, 0, 100, 100, 100, //
            30, 30, 30, 40, 40, 40, 0, 0, 0, 255, 0, 0,
        ];
        let mut out = Vec::new();
        // (by how much; the size of the result; its bytes)
        let cases: [(usize, (usize, usize), &[u8]); 4] = [
            (1, (4, 2), &picture),
            (2, (2, 1), &[25, 25, 25, 88, 25, 25]),
            // What does not fill a square is left out: 4x2 by 3 is 1x0.
            (3, (1, 0), &[]),
            (0, (0, 0), &[]),
        ];
        for (by, size, bytes) in cases {
            assert_eq!(shrink(&picture, (4, 2), by, &mut out), size, "{by}");
            assert_eq!(out, bytes, "{by}");
        }
    }

    #[test]
    fn a_picture_with_too_few_bytes_gives_nothing() {
        let mut out = vec![1, 2, 3];
        assert_eq!(shrink(&[0; 11], (2, 2), 1, &mut out), (0, 0));
        assert_eq!(out, [0u8; 0]);
        assert_eq!(shrink(&[0; 12], (usize::MAX, 2), 1, &mut out), (0, 0));
    }
}
