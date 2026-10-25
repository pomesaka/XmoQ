//! Where the sound is cut into pieces that are read one at a time: a piece
//! begins with a frame that has a voice ([`crate::judge`]), and ends after a
//! pause, or where it has grown too long. Every value is provisional
//! (`Cut::default`), chosen on voices of `say` and not yet on a meeting.

use crate::frames::{FRAME, FRAME_US, micros};
use crate::judge::Judged;

/// How the sound is cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// A piece ends after this much sound that is not a voice. 600 ms is
    /// longer than the pause between words, and is the least a piece is
    /// read after its last word.
    pub pause_ms: u64,
    /// A piece that grows to this is read at once. 15 s: whisper reads 30 s
    /// at a time, and a long piece holds its first word back as long.
    pub longest_ms: u64,
    /// A piece with less voice than this is not read: a click or a cough,
    /// of which whisper makes up words.
    pub least_voice_ms: u64,
    /// The sound before the first frame of a voice that the piece keeps,
    /// for the beginning of a word that is quieter than the rest.
    pub before_ms: u64,
}

impl Default for Cut {
    fn default() -> Self {
        Self {
            pause_ms: 600,
            longest_ms: 15_000,
            least_voice_ms: 250,
            before_ms: 200,
        }
    }
}

/// A piece of sound to be read.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The Timestamp of LOC of its first sample, in microseconds.
    pub start_us: u64,
    /// That of the sample after its last.
    pub end_us: u64,
    /// One channel at 16 kHz.
    pub samples: Vec<f32>,
}

/// What the cutter left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Left {
    /// Pieces with less voice than [`Cut::least_voice_ms`].
    pub too_little_voice: u64,
}

struct Voice {
    start_us: u64,
    samples: Vec<f32>,
    loud_frames: u64,
    quiet_frames: u64,
}

/// Cuts the sound of one track.
pub struct Cutter {
    cut: Cut,
    /// The frames before a voice, at most [`Cut::before_ms`] of them.
    before: std::collections::VecDeque<(u64, Vec<f32>)>,
    voice: Option<Voice>,
    left: Left,
}

impl Cutter {
    /// A cutter that cuts by `cut`.
    #[must_use]
    pub fn new(cut: Cut) -> Self {
        Self {
            cut,
            before: std::collections::VecDeque::new(),
            voice: None,
            left: Left::default(),
        }
    }

    /// No more sound comes, or what comes does not go on from it: the piece
    /// under way, if it has voice enough.
    pub fn end(&mut self) -> Option<Piece> {
        self.before.clear();
        let voice = self.voice.take()?;
        self.close(voice)
    }

    /// What was left out.
    #[must_use]
    pub fn left(&self) -> &Left {
        &self.left
    }

    /// A frame that was judged. Gives the piece that ends with it.
    pub fn push(&mut self, judged: Judged) -> Option<Piece> {
        let Judged {
            at,
            samples: frame,
            voice: loud,
        } = judged;
        let Some(voice) = self.voice.as_mut() else {
            if loud {
                let mut samples = Vec::new();
                let start_us = self.before.front().map_or(at, |(us, _)| *us);
                for (_, before) in self.before.drain(..) {
                    samples.extend(before);
                }
                samples.extend(frame);
                self.voice = Some(Voice {
                    start_us,
                    samples,
                    loud_frames: 1,
                    quiet_frames: 0,
                });
            } else {
                self.before.push_back((at, frame));
                while micros(self.before.len() * FRAME) > self.cut.before_ms * 1000 {
                    self.before.pop_front();
                }
            }
            return None;
        };
        voice.samples.extend(frame);
        if loud {
            voice.loud_frames += 1;
            voice.quiet_frames = 0;
        } else {
            voice.quiet_frames += 1;
        }
        let paused = voice.quiet_frames * FRAME_US >= self.cut.pause_ms * 1000;
        let long = micros(voice.samples.len()) >= self.cut.longest_ms * 1000;
        if !paused && !long {
            return None;
        }
        let voice = self.voice.take()?;
        self.close(voice)
    }

    fn close(&mut self, mut voice: Voice) -> Option<Piece> {
        if voice.loud_frames * FRAME_US < self.cut.least_voice_ms * 1000 {
            self.left.too_little_voice += 1;
            return None;
        }
        // The pause that ended it is not read, but for as much as is kept
        // before a voice.
        let keep_quiet = usize::try_from(self.cut.before_ms * 1000 / FRAME_US).unwrap_or(0);
        let quiet = usize::try_from(voice.quiet_frames).unwrap_or(0);
        let drop_frames = quiet.saturating_sub(keep_quiet);
        let len = voice.samples.len().saturating_sub(drop_frames * FRAME);
        voice.samples.truncate(len);
        Some(Piece {
            start_us: voice.start_us,
            end_us: voice.start_us + micros(voice.samples.len()),
            samples: voice.samples,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One of the test's frames: a voice or none.
    #[derive(Clone, Copy)]
    enum F {
        Voice,
        Quiet,
    }

    /// Runs frames of 20 ms from Timestamp 1 s, and gives the pieces as
    /// (start in ms, end in ms) from it.
    fn run(cut: Cut, frames: &[(usize, F)]) -> (Vec<(u64, u64)>, Left) {
        let mut cutter = Cutter::new(cut);
        let mut at = 1_000_000;
        let mut pieces = Vec::new();
        for (count, f) in frames {
            for _ in 0..*count {
                pieces.extend(cutter.push(Judged {
                    at,
                    samples: vec![0.0; FRAME],
                    voice: matches!(f, F::Voice),
                }));
                at += FRAME_US;
            }
        }
        pieces.extend(cutter.end());
        let ms = pieces
            .iter()
            .map(|piece| {
                (
                    (piece.start_us - 1_000_000) / 1000,
                    (piece.end_us - 1_000_000) / 1000,
                )
            })
            .collect();
        (ms, cutter.left().clone())
    }

    #[test]
    fn pieces() {
        use F::{Quiet, Voice};
        // The name, the frames, the pieces, and how many were left out.
        type Case = (&'static str, Vec<(usize, F)>, Vec<(u64, u64)>, u64);
        let cases: [Case; 6] = [
            ("silence is no piece", vec![(100, Quiet)], vec![], 0),
            (
                "a voice with 200 ms before it and 200 ms of its pause",
                vec![(50, Quiet), (50, Voice), (50, Quiet)],
                vec![(800, 2200)],
                0,
            ),
            (
                "a pause shorter than 600 ms is within the piece",
                vec![
                    (50, Quiet),
                    (25, Voice),
                    (25, Quiet),
                    (25, Voice),
                    (50, Quiet),
                ],
                vec![(800, 2700)],
                0,
            ),
            (
                "a pause of 600 ms ends it",
                vec![
                    (50, Quiet),
                    (25, Voice),
                    (30, Quiet),
                    (25, Voice),
                    (50, Quiet),
                ],
                vec![(800, 1700), (2100, 2800)],
                0,
            ),
            (
                "200 ms of voice is a click, left out",
                vec![(50, Quiet), (10, Voice), (50, Quiet)],
                vec![],
                1,
            ),
            (
                "a voice of 20 s is read every 15 s, and at its end",
                vec![(1000, Voice)],
                vec![(0, 15000), (15000, 20000)],
                0,
            ),
        ];
        for (name, frames, expected, left) in cases {
            let (pieces, counted) = run(Cut::default(), &frames);
            assert_eq!(pieces, expected, "{name}");
            assert_eq!(counted.too_little_voice, left, "{name}");
        }
    }
}
