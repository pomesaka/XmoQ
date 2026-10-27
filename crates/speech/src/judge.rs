//! Whether a frame of 20 ms has a voice: by a model of voice activity
//! (Silero, through whisper.cpp; the default), or by how loud it is.
//!
//! WHY a model by default, decided on 2026-10-25: by loudness alone the
//! demo's tone (440 Hz, and a beep of 1 kHz every 2 s) is a voice that does
//! not end, and whisper.cpp read 15 s of it as 「ご視聴ありがとうございました」
//! (`large-v3-turbo-q5_0`, `ja`), `*music*` (`auto`) and 「(エンディング)」
//! (`small-q5_1`). The `no_speech_probability` whisper.cpp gives each segment
//! was 0.000 for the tone as for speech, so the words cannot be told apart
//! after they are read. Silero gave the tone 0.00 to 0.06 and the voices of
//! `say` 0.66 to 1.00, in 0.12 ms for 32 ms of sound
//! (`docs/experiments/2026-10-25-the-transcriber-reads-a-voice.md`).
//! WHY loudness is kept: it needs no model, and the tests of the cutting
//! use it; the transcriber takes it with `--voice loudness`.
//!
//! What the model costs: a piece cannot end before its frames are judged,
//! so its words come up to half a second later. WHY NOT the VAD that
//! whisper.cpp runs inside `whisper_full` (`vad` of its parameters): it
//! runs on what it is given to read, and the cutting into pieces, which
//! decides when words come, is before that.

use anyhow::Result;

use crate::frames::FRAME;

/// What gives the probability of a voice for each window of
/// [`WINDOW`] samples: Silero (`crate::vad::Silero`), or a stand-in in tests.
pub trait Probabilities {
    /// The probability of a voice in each window of [`WINDOW`] samples of
    /// `samples`, from the first; the last window is filled with silence.
    /// What the model holds between windows begins anew at each call.
    ///
    /// # Errors
    /// When the model fails.
    fn probabilities(&mut self, samples: &[f32]) -> Result<Vec<f32>>;
}

/// Samples of the window Silero judges: 32 ms at 16 kHz
/// (`n_window` of whisper.cpp 1.8.3).
pub const WINDOW: usize = 512;

/// Frames judged at once: 0.5 s. WHY not a frame at a time: the model
/// begins anew at each call (above), and a window of its own would be
/// judged with nothing before it. WHY not longer: a piece cannot end before
/// its frames are judged, so this is added to the time its words take.
const CHUNK_FRAMES: usize = 25;

/// Samples of the frames before a chunk that are judged with it, and whose
/// probabilities are not taken: what the model hears before the chunk.
const CONTEXT: usize = CHUNK_FRAMES * FRAME;

/// A frame, and whether it has a voice.
#[derive(Clone, Debug, PartialEq)]
pub struct Judged {
    /// The Timestamp of LOC of its first sample, in microseconds.
    pub at: u64,
    /// Its samples.
    pub samples: Vec<f32>,
    /// Whether it has a voice.
    pub voice: bool,
}

/// The loudness of a voice for [`Judge::Loudness`]: -40 dBFS. Speech near a
/// microphone is -30 to -15, a quiet room below -55. Provisional.
pub const VOICE_DBFS: f32 = -40.0;

/// How a voice is told.
pub enum Judge {
    /// A frame whose root mean square is at least this many dB below a
    /// full-scale sample ([`VOICE_DBFS`]) has a voice.
    Loudness(f32),
    /// By the model.
    Model(ModelJudge),
}

impl Judge {
    /// A frame. Gives the frames that are judged now, in order: with
    /// [`Judge::Model`] those of a chunk once it is full.
    pub fn push(&mut self, at: u64, samples: Vec<f32>) -> Vec<Judged> {
        match self {
            Self::Loudness(dbfs) => {
                let voice = loudness(&samples) >= *dbfs;
                vec![Judged { at, samples, voice }]
            }
            Self::Model(model) => model.push(at, samples),
        }
    }

    /// The frames not judged yet are judged, and what came before is
    /// forgotten: the sound does not go on from them.
    pub fn flush(&mut self) -> Vec<Judged> {
        match self {
            Self::Loudness(_) => Vec::new(),
            Self::Model(model) => model.flush(),
        }
    }

    /// Chunks the model failed on; their frames were taken for silence.
    #[must_use]
    pub fn failed(&self) -> u64 {
        match self {
            Self::Loudness(_) => 0,
            Self::Model(model) => model.failed,
        }
    }
}

/// Judges by a model of voice activity, a chunk of frames at a time.
pub struct ModelJudge {
    model: Box<dyn Probabilities + Send>,
    threshold: f32,
    /// The samples before the frames that wait, up to [`CONTEXT`].
    context: Vec<f32>,
    waiting: Vec<(u64, Vec<f32>)>,
    failed: u64,
}

impl ModelJudge {
    /// A frame has a voice where the window its middle sample is in has a
    /// probability of `threshold` or more. 0.5 is whisper.cpp's default
    /// (`whisper_vad_default_params`).
    #[must_use]
    pub fn new(model: Box<dyn Probabilities + Send>, threshold: f32) -> Self {
        Self {
            model,
            threshold,
            context: Vec::new(),
            waiting: Vec::new(),
            failed: 0,
        }
    }

    fn push(&mut self, at: u64, samples: Vec<f32>) -> Vec<Judged> {
        self.waiting.push((at, samples));
        if self.waiting.len() < CHUNK_FRAMES {
            return Vec::new();
        }
        self.judge()
    }

    fn flush(&mut self) -> Vec<Judged> {
        let judged = self.judge();
        self.context.clear();
        judged
    }

    fn judge(&mut self) -> Vec<Judged> {
        let waiting = std::mem::take(&mut self.waiting);
        if waiting.is_empty() {
            return Vec::new();
        }
        let mut heard = std::mem::take(&mut self.context);
        let first = heard.len();
        for (_, samples) in &waiting {
            heard.extend_from_slice(samples);
        }
        let probabilities = self.model.probabilities(&heard).unwrap_or_else(|_| {
            self.failed += 1;
            Vec::new()
        });
        let judged = waiting
            .into_iter()
            .enumerate()
            .map(|(index, (at, samples))| {
                let middle = first + index * FRAME + FRAME / 2;
                let voice = probabilities
                    .get(middle / WINDOW)
                    .is_some_and(|probability| *probability >= self.threshold);
                Judged { at, samples, voice }
            })
            .collect();
        self.context = heard.split_off(heard.len().saturating_sub(CONTEXT));
        judged
    }
}

/// The root mean square of `frame`, in dB below a full-scale sample.
#[must_use]
pub fn loudness(frame: &[f32]) -> f32 {
    let squares: f32 = frame.iter().map(|sample| sample * sample).sum();
    let count = f32::from(u16::try_from(frame.len()).unwrap_or(u16::MAX).max(1));
    20.0 * (squares / count).sqrt().max(1e-10).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::FRAME_US;

    /// Gives each window the mean of its samples: a frame of 1.0 is a
    /// voice, of 0.0 none. Says how many samples each call was given.
    struct Mean(std::sync::Arc<std::sync::Mutex<Vec<usize>>>);

    impl Probabilities for Mean {
        fn probabilities(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
            self.0.lock().unwrap().push(samples.len());
            Ok(samples
                .chunks(WINDOW)
                .map(|window| {
                    window.iter().sum::<f32>() / f32::from(u16::try_from(WINDOW).unwrap())
                })
                .collect())
        }
    }

    fn frame(level: f32) -> Vec<f32> {
        vec![level; FRAME]
    }

    #[test]
    fn the_model_judges_a_chunk_of_half_a_second_with_the_half_second_before() {
        let calls = std::sync::Arc::default();
        let mut judge = Judge::Model(ModelJudge::new(
            Box::new(Mean(std::sync::Arc::clone(&calls))),
            0.5,
        ));
        let mut judged = Vec::new();
        // 30 frames of voice, then 30 of silence.
        for n in 0..60 {
            let level = if n < 30 { 1.0 } else { 0.0 };
            judged.extend(judge.push(n * FRAME_US, frame(level)));
        }
        assert_eq!(judged.len(), 50, "two chunks are full");
        judged.extend(judge.flush());
        let voices: Vec<bool> = judged.iter().map(|frame| frame.voice).collect();
        let first_silent = voices.iter().position(|voice| !voice).unwrap();
        // The window of the middle of frame 30 has samples of frame 29 in it
        // at most; frames on the line are judged by their middle.
        assert!((29..=31).contains(&first_silent), "{first_silent}");
        assert!(voices[first_silent..].iter().all(|voice| !voice));
        let ats: Vec<u64> = judged.iter().map(|frame| frame.at).collect();
        assert_eq!(ats, (0..60).map(|n| n * FRAME_US).collect::<Vec<_>>());
        // The first chunk alone; the second with the first before it; the
        // last 10 frames with the 25 before them.
        assert_eq!(*calls.lock().unwrap(), [25 * FRAME, 50 * FRAME, 35 * FRAME]);
    }

    #[test]
    fn loudness_judges_each_frame_at_once() {
        let mut judge = Judge::Loudness(-40.0);
        let cases = [(0.1, true), (0.001, false)];
        for (level, voice) in cases {
            let judged = judge.push(0, frame(level));
            assert_eq!(
                judged.iter().map(|frame| frame.voice).collect::<Vec<_>>(),
                [voice],
                "{level}"
            );
        }
    }

    /// Fails every time.
    struct Broken;

    impl Probabilities for Broken {
        fn probabilities(&mut self, _: &[f32]) -> Result<Vec<f32>> {
            anyhow::bail!("broken")
        }
    }

    #[test]
    fn a_chunk_the_model_fails_on_is_silence_and_counted() {
        let mut judge = Judge::Model(ModelJudge::new(Box::new(Broken), 0.5));
        let mut judged = Vec::new();
        for n in 0..25 {
            judged.extend(judge.push(n * FRAME_US, frame(1.0)));
        }
        assert_eq!(judged.len(), 25);
        assert!(judged.iter().all(|frame| !frame.voice));
        assert_eq!(judge.failed(), 1);
    }
}
