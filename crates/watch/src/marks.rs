//! For measuring with the demo's publisher, and nothing a viewer needs to
//! play: its picture has a white square in the upper right corner for the
//! first 200 ms of every 2 seconds, and its sound is a loud beep for the
//! same 200 ms (`marked` and `MARK` in `demo/shared.ts`). How far apart the
//! two leave the viewer says how well picture and sound are held together.

/// Whether a picture of 3 bytes a pixel has the white square: its middle
/// is at (580, 60) of a picture of 640x360, and at the same share of the
/// way in any other size.
#[must_use]
pub fn picture_is_marked(rgb: &[u8], size: (usize, usize)) -> bool {
    let (x, y) = (580 * size.0 / 640, 60 * size.1 / 360);
    if x >= size.0 || y >= size.1 {
        return false;
    }
    let at = (y * size.0 + x) * 3;
    rgb.get(at..at + 3)
        .is_some_and(|pixel| pixel.iter().all(|value| *value > 200))
}

/// Whether decoded sound has the beep: the beep is 0.6 of full scale, and
/// the tone it stands out from 0.25.
#[must_use]
pub fn sound_is_marked(samples: &[f32]) -> bool {
    samples.iter().any(|sample| sample.abs() > 0.45)
}

/// For every moment a square began to be shown, how long after the nearest
/// beginning of a beep that was, in microseconds: less than 0 where the
/// picture was the earlier one. A square with no beep within a second is
/// left out: one of the two was missed.
#[must_use]
pub fn picture_after_sound_us(picture_us: &[u64], sound_us: &[u64]) -> Vec<i64> {
    picture_us
        .iter()
        .filter_map(|picture| {
            sound_us
                .iter()
                .map(|sound| i128::from(*picture) - i128::from(*sound))
                .min_by_key(|apart| apart.abs())
        })
        .filter(|apart| apart.abs() < 1_000_000)
        .filter_map(|apart| i64::try_from(apart).ok())
        .collect()
}

/// The last values of a series: what is counted over a broadcast of any
/// length without growing with it.
#[derive(Debug, Clone)]
pub struct Recent {
    values: std::collections::VecDeque<i64>,
    most: usize,
}

impl Recent {
    /// A series of which the last `most` values are kept.
    #[must_use]
    pub fn new(most: usize) -> Self {
        Self {
            values: std::collections::VecDeque::new(),
            most,
        }
    }

    /// Takes a value; the oldest goes where `most` are there already.
    pub fn push(&mut self, value: i64) {
        if self.values.len() >= self.most {
            self.values.pop_front();
        }
        if self.most > 0 {
            self.values.push_back(value);
        }
    }

    /// The values that are kept, the oldest first.
    #[must_use]
    pub fn to_vec(&self) -> Vec<i64> {
        self.values.iter().copied().collect()
    }
}

/// The least, the median, the 95th of a hundred and the greatest of some
/// values; all 0 for none.
#[must_use]
pub fn spread(values: &[i64]) -> [i64; 4] {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let at = |hundredths: usize| {
        let index = (sorted.len() * hundredths / 100).min(sorted.len().saturating_sub(1));
        sorted.get(index).copied().unwrap_or(0)
    };
    [at(0), at(50), at(95), at(100)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A black picture with one white pixel.
    fn picture(size: (usize, usize), white: (usize, usize)) -> Vec<u8> {
        let mut rgb = vec![0; size.0 * size.1 * 3];
        let at = (white.1 * size.0 + white.0) * 3;
        rgb[at..at + 3].fill(255);
        rgb
    }

    #[test]
    fn the_square_is_looked_for_at_the_same_share_of_every_size() {
        // (the size; the white pixel; whether it is the mark)
        let cases = [
            ((640, 360), (580, 60), true),
            ((640, 360), (579, 60), false),
            ((1280, 720), (1160, 120), true),
            ((1920, 1080), (1740, 180), true),
            ((1920, 1080), (580, 60), false),
            ((320, 180), (290, 30), true),
        ];
        for (size, white, marked) in cases {
            assert_eq!(
                picture_is_marked(&picture(size, white), size),
                marked,
                "{size:?} {white:?}"
            );
        }
        // A picture of nothing, and one with fewer bytes than its size asks for.
        assert!(!picture_is_marked(&[], (0, 0)));
        assert!(!picture_is_marked(&[255; 30], (640, 360)));
    }

    #[test]
    fn the_beep_is_louder_than_the_tone() {
        assert!(!sound_is_marked(&[0.0, 0.25, -0.25, 0.45]));
        assert!(sound_is_marked(&[0.0, 0.25, -0.6]));
        assert!(!sound_is_marked(&[]));
    }

    #[test]
    fn a_square_is_set_against_the_nearest_beep() {
        // (when squares began; when beeps began; how far apart)
        let cases: [(&[u64], &[u64], &[i64]); 5] = [
            (
                &[2_010_000, 4_000_000, 5_990_000],
                &[2_000_000, 4_000_000, 6_000_000],
                &[10_000, 0, -10_000],
            ),
            // A beep that was missed: the square after it has none within a second.
            (&[2_000_000, 4_000_000], &[2_020_000], &[-20_000]),
            (&[2_000_000], &[], &[]),
            (&[], &[2_000_000], &[]),
            // Just within a second, and just not.
            (&[999_999, 3_000_000], &[0, 2_000_000], &[999_999]),
        ];
        for (picture, sound, apart) in cases {
            assert_eq!(picture_after_sound_us(picture, sound), apart, "{picture:?}");
        }
    }

    #[test]
    fn of_a_series_the_last_values_are_kept() {
        let mut recent = Recent::new(3);
        assert_eq!(recent.to_vec(), [0i64; 0]);
        for value in [1, 2, 3] {
            recent.push(value);
        }
        assert_eq!(recent.to_vec(), [1, 2, 3]);
        recent.push(4);
        recent.push(5);
        assert_eq!(recent.to_vec(), [3, 4, 5]);
        let mut none = Recent::new(0);
        none.push(1);
        assert_eq!(none.to_vec(), [0i64; 0]);
    }

    #[test]
    fn the_spread_of_some_values() {
        // (the values; the least, the median, the 95th of a hundred, the greatest)
        let cases: [(&[i64], [i64; 4]); 4] = [
            (&[], [0, 0, 0, 0]),
            (&[7], [7, 7, 7, 7]),
            (&[3, -1, 2], [-1, 2, 3, 3]),
            (
                &[
                    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
                ],
                [1, 11, 20, 20],
            ),
        ];
        for (values, expected) in cases {
            assert_eq!(spread(values), expected, "{values:?}");
        }
    }
}
