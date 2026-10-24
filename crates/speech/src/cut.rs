//! Where the sound is cut into pieces that are read one at a time: a piece
//! begins where the sound is loud enough to be a voice, and ends after a
//! pause, or where it has grown too long.
//!
//! WHY by loudness: it needs nothing but the samples. WHY NOT the VAD of
//! whisper.cpp (Silero): a second model to fetch and load, for sound that
//! a broadcast's microphone has made clean already; it is the one to try
//! where a room's noise is taken for a voice. Every value is provisional
//! (`Cut::default`), chosen on voices of `say` and not yet on a meeting.

use crate::decode::RATE;

/// A frame of 20 ms, the unit loudness is judged in.
const FRAME: usize = 320;
/// The microseconds of one frame.
const FRAME_US: u64 = 20_000;

/// How the sound is cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// A frame whose root mean square is at least this, in dB below a
    /// full-scale sample, is a voice. -40: speech near a microphone is
    /// -30 to -15, a quiet room below -55.
    pub voice_dbfs: f32,
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
            voice_dbfs: -40.0,
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
    /// Samples that do not fill a frame yet, and the Timestamp of the first.
    partial: Vec<f32>,
    partial_us: u64,
    /// The Timestamp the next sample has where the sound goes on.
    next_us: Option<u64>,
    /// The frames before a voice, at most [`Cut::before_ms`] of them.
    before: std::collections::VecDeque<(u64, Vec<f32>)>,
    voice: Option<Voice>,
    left: Left,
}

/// A packet whose Timestamp is this far from where the sound before it
/// ended begins anew: packets were lost, or the publisher began again.
const GAP_US: u64 = 100_000;

impl Cutter {
    /// A cutter that cuts by `cut`.
    #[must_use]
    pub fn new(cut: Cut) -> Self {
        Self {
            cut,
            partial: Vec::new(),
            partial_us: 0,
            next_us: None,
            before: std::collections::VecDeque::new(),
            voice: None,
            left: Left::default(),
        }
    }

    /// The samples of one packet, whose first sample has the Timestamp
    /// `start_us`. Gives the pieces that end with them.
    pub fn push(&mut self, start_us: u64, samples: &[f32]) -> Vec<Piece> {
        let mut pieces = Vec::new();
        if self
            .next_us
            .is_some_and(|next| start_us.abs_diff(next) > GAP_US)
        {
            pieces.extend(self.end());
        }
        if self.partial.is_empty() {
            self.partial_us = start_us;
        }
        self.next_us = Some(start_us + micros(samples.len()));
        self.partial.extend_from_slice(samples);
        while self.partial.len() >= FRAME {
            let frame: Vec<f32> = self.partial.drain(..FRAME).collect();
            let at = self.partial_us;
            self.partial_us += FRAME_US;
            pieces.extend(self.frame(at, frame));
        }
        pieces
    }

    /// No more sound comes, or what comes does not go on from it: the piece
    /// under way, if it has voice enough.
    pub fn end(&mut self) -> Option<Piece> {
        self.partial.clear();
        self.next_us = None;
        self.before.clear();
        let voice = self.voice.take()?;
        self.close(voice)
    }

    /// What was left out.
    #[must_use]
    pub fn left(&self) -> &Left {
        &self.left
    }

    fn frame(&mut self, at: u64, frame: Vec<f32>) -> Option<Piece> {
        let loud = dbfs(&frame) >= self.cut.voice_dbfs;
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

/// The microseconds of `samples` at [`RATE`].
fn micros(samples: usize) -> u64 {
    u64::try_from(samples).unwrap_or(u64::MAX) * 1_000_000 / u64::from(RATE)
}

/// The root mean square of `frame`, in dB below a full-scale sample.
fn dbfs(frame: &[f32]) -> f32 {
    let squares: f32 = frame.iter().map(|s| s * s).sum();
    let count = f32::from(u16::try_from(frame.len()).unwrap_or(u16::MAX).max(1));
    20.0 * (squares / count).sqrt().max(1e-10).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One of the test's packets: 20 ms that are loud (a voice) or quiet.
    #[derive(Clone, Copy)]
    enum P {
        Loud,
        Quiet,
    }

    fn packet(p: P) -> Vec<f32> {
        let level = match p {
            P::Loud => 0.1,    // -20 dBFS
            P::Quiet => 0.001, // -60 dBFS
        };
        (0..FRAME)
            .map(|i| if i % 2 == 0 { level } else { -level })
            .collect()
    }

    /// Runs packets of 20 ms from Timestamp 1 s, and gives the pieces as
    /// (start in ms, end in ms) from it.
    fn run(cut: Cut, packets: &[(usize, P)]) -> (Vec<(u64, u64)>, Left) {
        let mut cutter = Cutter::new(cut);
        let mut at = 1_000_000;
        let mut pieces = Vec::new();
        for (count, p) in packets {
            for _ in 0..*count {
                pieces.extend(cutter.push(at, &packet(*p)));
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
        use P::{Loud, Quiet};
        // The name, the packets, the pieces, and how many were left out.
        type Case = (&'static str, Vec<(usize, P)>, Vec<(u64, u64)>, u64);
        let cases: [Case; 6] = [
            ("silence is no piece", vec![(100, Quiet)], vec![], 0),
            (
                "a voice with 200 ms before it and 200 ms of its pause",
                vec![(50, Quiet), (50, Loud), (50, Quiet)],
                vec![(800, 2200)],
                0,
            ),
            (
                "a pause shorter than 600 ms is within the piece",
                vec![
                    (50, Quiet),
                    (25, Loud),
                    (25, Quiet),
                    (25, Loud),
                    (50, Quiet),
                ],
                vec![(800, 2700)],
                0,
            ),
            (
                "a pause of 600 ms ends it",
                vec![
                    (50, Quiet),
                    (25, Loud),
                    (30, Quiet),
                    (25, Loud),
                    (50, Quiet),
                ],
                vec![(800, 1700), (2100, 2800)],
                0,
            ),
            (
                "200 ms of voice is a click, left out",
                vec![(50, Quiet), (10, Loud), (50, Quiet)],
                vec![],
                1,
            ),
            (
                "a voice of 20 s is read every 15 s, and at its end",
                vec![(1000, Loud)],
                vec![(0, 15000), (15000, 20000)],
                0,
            ),
        ];
        for (name, packets, expected, left) in cases {
            let (pieces, counted) = run(Cut::default(), &packets);
            assert_eq!(pieces, expected, "{name}");
            assert_eq!(counted.too_little_voice, left, "{name}");
        }
    }

    #[test]
    fn a_gap_in_the_timestamps_ends_the_piece() {
        let mut cutter = Cutter::new(Cut::default());
        let mut pieces = Vec::new();
        for n in 0..25 {
            pieces.extend(cutter.push(n * FRAME_US, &packet(P::Loud)));
        }
        for n in 0..25 {
            pieces.extend(cutter.push(10_000_000 + n * FRAME_US, &packet(P::Loud)));
        }
        pieces.extend(cutter.end());
        let spans: Vec<(u64, u64)> = pieces.iter().map(|p| (p.start_us, p.end_us)).collect();
        assert_eq!(spans, [(0, 500_000), (10_000_000, 10_500_000)]);
    }

    #[test]
    fn packets_of_another_length_are_cut_in_frames_of_20_ms() {
        let mut cutter = Cutter::new(Cut::default());
        let mut pieces = Vec::new();
        // Packets of 60 ms: three frames each.
        for n in 0..10 {
            let samples: Vec<f32> = (0..3).flat_map(|_| packet(P::Loud)).collect();
            pieces.extend(cutter.push(n * 3 * FRAME_US, &samples));
        }
        pieces.extend(cutter.end());
        let spans: Vec<(u64, u64)> = pieces.iter().map(|p| (p.start_us, p.end_us)).collect();
        assert_eq!(spans, [(0, 600_000)]);
    }
}
