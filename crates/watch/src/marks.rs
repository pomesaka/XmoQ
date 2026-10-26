//! For measuring, and nothing a viewer needs to play: a publisher that
//! marks its picture with a white square for a moment at a steady pace,
//! and its sound with a loud beep for the same moment, has how far apart
//! the two leave the viewer say how well picture and sound are held
//! together. Where the square is and how loud the beep is are the
//! publisher's, given with `--measure-marks` ([`Marks`]).

/// The marks of a publisher.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marks {
    /// The middle of the square, in a picture of `w`x`h`; at the same share
    /// of the way in any other size.
    pub x: usize,
    /// See [`Self::x`].
    pub y: usize,
    /// The size the square's place is given in.
    pub w: usize,
    /// See [`Self::w`].
    pub h: usize,
    /// How often a mark comes, in milliseconds.
    pub every_ms: u64,
    /// How loud the beep is, as a share of full scale.
    pub beep: f32,
    /// How loud the tone the beep stands out from is, the same way.
    pub ground: f32,
}

impl Marks {
    /// Reads `x=580,y=60,w=640,h=360,every-ms=2000,beep=0.6,ground=0.25`:
    /// every name once, in any order.
    ///
    /// # Errors
    ///
    /// A name missing, unknown or given twice; a value that is no number of
    /// its kind; a square outside its picture; a beep no louder than its
    /// ground, or louder than full scale.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (mut x, mut y, mut w, mut h) = (None, None, None, None);
        let (mut every_ms, mut beep, mut ground) = (None, None, None);
        for pair in text.split(',') {
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("{pair} is no name=value"))?;
            let place = match name {
                "x" => &mut x,
                "y" => &mut y,
                "w" => &mut w,
                "h" => &mut h,
                "every-ms" => &mut every_ms,
                "beep" => &mut beep,
                "ground" => &mut ground,
                _ => {
                    return Err(format!(
                        "{name} is none of x, y, w, h, every-ms, beep, ground"
                    ));
                }
            };
            if place.replace(value).is_some() {
                return Err(format!("{name} is given twice"));
            }
        }
        let whole = |value: Option<&str>, name: &str| -> Result<usize, String> {
            value
                .ok_or_else(|| format!("{name} is missing"))?
                .parse()
                .map_err(|_| format!("{name} is no whole number"))
        };
        let share = |value: Option<&str>, name: &str| -> Result<f32, String> {
            value
                .ok_or_else(|| format!("{name} is missing"))?
                .parse()
                .map_err(|_| format!("{name} is no number"))
        };
        let marks = Self {
            x: whole(x, "x")?,
            y: whole(y, "y")?,
            w: whole(w, "w")?,
            h: whole(h, "h")?,
            every_ms: u64::try_from(whole(every_ms, "every-ms")?)
                .map_err(|_| "every-ms is too large".to_string())?,
            beep: share(beep, "beep")?,
            ground: share(ground, "ground")?,
        };
        if marks.x >= marks.w || marks.y >= marks.h {
            return Err("the square is outside its picture".to_string());
        }
        if marks.every_ms == 0 {
            return Err("every-ms is 0".to_string());
        }
        if !(0.0 <= marks.ground && marks.ground < marks.beep && marks.beep <= 1.0) {
            return Err("ground is from 0, and beep above it to 1 at most".to_string());
        }
        Ok(marks)
    }

    /// Whether a picture of 3 bytes a pixel has the white square.
    #[must_use]
    pub fn picture_is_marked(&self, rgb: &[u8], size: (usize, usize)) -> bool {
        let (x, y) = (self.x * size.0 / self.w, self.y * size.1 / self.h);
        if x >= size.0 || y >= size.1 {
            return false;
        }
        let at = (y * size.0 + x) * 3;
        rgb.get(at..at + 3)
            .is_some_and(|pixel| pixel.iter().all(|value| *value > 200))
    }

    /// Whether decoded sound has the beep: a sample louder than halfway
    /// between the ground and the beep.
    #[must_use]
    pub fn sound_is_marked(&self, samples: &[f32]) -> bool {
        let between = f32::midpoint(self.ground, self.beep);
        samples.iter().any(|sample| sample.abs() > between)
    }

    /// For every moment a square began to be shown, how long after the
    /// nearest beginning of a beep that was, in microseconds: less than 0
    /// where the picture was the earlier one. A square with no beep within
    /// half the time between marks is left out: one of the two was missed,
    /// and the nearest beep is another mark's.
    #[must_use]
    pub fn picture_after_sound_us(&self, picture_us: &[u64], sound_us: &[u64]) -> Vec<i64> {
        let within = i128::from(self.every_ms) * 1000 / 2;
        picture_us
            .iter()
            .filter_map(|picture| {
                sound_us
                    .iter()
                    .map(|sound| i128::from(*picture) - i128::from(*sound))
                    .min_by_key(|apart| apart.abs())
            })
            .filter(|apart| apart.abs() < within)
            .filter_map(|apart| i64::try_from(apart).ok())
            .collect()
    }
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

    /// A square at (580, 60) of 640x360 every 2 s, a beep of 0.6 over a
    /// tone of 0.25.
    const MARKS: Marks = Marks {
        x: 580,
        y: 60,
        w: 640,
        h: 360,
        every_ms: 2000,
        beep: 0.6,
        ground: 0.25,
    };

    #[test]
    fn the_marks_are_read_by_name() {
        let all = "x=580,y=60,w=640,h=360,every-ms=2000,beep=0.6,ground=0.25";
        let cases = [
            (all, Ok(MARKS)),
            (
                "ground=0.25,beep=0.6,every-ms=2000,h=360,w=640,y=60,x=580",
                Ok(MARKS),
            ),
            (
                "x=580,y=60,w=640,h=360,every-ms=2000,beep=0.6",
                Err("ground is missing"),
            ),
            (
                "x=580,x=1,y=60,w=640,h=360,every-ms=2000,beep=0.6,ground=0.25",
                Err("x is given twice"),
            ),
            (
                "z=1",
                Err("z is none of x, y, w, h, every-ms, beep, ground"),
            ),
            ("x", Err("x is no name=value")),
            (
                "x=a,y=60,w=640,h=360,every-ms=2000,beep=0.6,ground=0.25",
                Err("x is no whole number"),
            ),
            (
                "x=640,y=60,w=640,h=360,every-ms=2000,beep=0.6,ground=0.25",
                Err("the square is outside its picture"),
            ),
            (
                "x=580,y=60,w=640,h=360,every-ms=0,beep=0.6,ground=0.25",
                Err("every-ms is 0"),
            ),
            (
                "x=580,y=60,w=640,h=360,every-ms=2000,beep=0.2,ground=0.25",
                Err("ground is from 0, and beep above it to 1 at most"),
            ),
        ];
        for (text, read) in cases {
            assert_eq!(Marks::parse(text), read.map_err(str::to_string), "{text}");
        }
    }

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
                MARKS.picture_is_marked(&picture(size, white), size),
                marked,
                "{size:?} {white:?}"
            );
        }
        // A picture of nothing, and one with fewer bytes than its size asks for.
        assert!(!MARKS.picture_is_marked(&[], (0, 0)));
        assert!(!MARKS.picture_is_marked(&[255; 30], (640, 360)));
    }

    #[test]
    fn the_beep_is_louder_than_the_tone() {
        // Halfway between 0.25 and 0.6 is 0.425.
        assert!(!MARKS.sound_is_marked(&[0.0, 0.25, -0.25, 0.42]));
        assert!(MARKS.sound_is_marked(&[0.0, 0.25, -0.43]));
        assert!(MARKS.sound_is_marked(&[0.0, 0.25, -0.6]));
        assert!(!MARKS.sound_is_marked(&[]));
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
            // A beep that was missed: the square after it has none within
            // half the time between marks, a second.
            (&[2_000_000, 4_000_000], &[2_020_000], &[-20_000]),
            (&[2_000_000], &[], &[]),
            (&[], &[2_000_000], &[]),
            // Just within a second, and just not.
            (&[999_999, 3_000_000], &[0, 2_000_000], &[999_999]),
        ];
        for (picture, sound, apart) in cases {
            assert_eq!(
                MARKS.picture_after_sound_us(picture, sound),
                apart,
                "{picture:?}"
            );
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
