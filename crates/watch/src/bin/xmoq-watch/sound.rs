//! The sound: packets of Opus decoded with the crate `opus-rs`, and played
//! by [`crate::audio_output`] at the moments they are due.
//!
//! WHY `opus-rs` 0.1.39 (decided on 2026-10-24): set against libopus on
//! 9,000 packets of a browser and 9,000 of moqlivemock, it decoded within
//! -130 dB of libopus, in 1.2 to 1.5 times libopus's processor time, and did
//! not overflow with the checks of a debug build on
//! (`docs/experiments/2026-10-21-opus-decoders-in-rust-set-against-libopus.md`).
//! WHY NOT `opus-decoder` 0.1.1, the decoder before: it overflowed on 169 of
//! the browser's 9,000 packets (a debug build panics there) and took 92
//! times libopus's time. WHY NOT libopus (the crate `opus`): a C compiler
//! and cmake in every build. WHY NOT `opus-pure`, a fork of `opus-rs` as
//! close and as fast: two contributors, where `opus-rs` has six.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use anyhow::{Result, anyhow};
use xmoq_watch::marks::Marks;
use xmoq_watch::playout::{Counts, Packet, Playout, RATE};

use crate::audio_output::AudioOutput;

/// Samples of the longest packet of Opus, 120 ms of one channel at 48 kHz:
/// what the decoder is given room for.
const MOST_SAMPLES: usize = 5760;

/// How loud the sound is played, changed while it plays.
pub struct Volume {
    /// The bits of an f32, 0 to 1.
    level: AtomicU32,
    muted: AtomicBool,
}

impl Volume {
    pub fn new(level: f32) -> Self {
        Self {
            level: AtomicU32::new(level.to_bits()),
            muted: AtomicBool::new(false),
        }
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn change(&self, by: f32) {
        let level = (self.level() + by).clamp(0.0, 1.0);
        self.level.store(level.to_bits(), Ordering::Relaxed);
    }

    pub fn mute(&self) {
        self.muted.fetch_xor(true, Ordering::Relaxed);
    }

    fn now(&self) -> f32 {
        if self.muted() { 0.0 } else { self.level() }
    }
}

/// What the output was handed, before the volume.
#[derive(Default)]
struct Level {
    peak: f32,
    squares: f64,
    samples: f64,
}

#[derive(Default)]
struct Shared {
    playout: Playout,
    level: Level,
}

/// What the sound did.
pub struct Heard {
    pub counts: Counts,
    pub refused: u64,
    /// The greatest sample and the root of the mean square of what the
    /// output was handed, before the volume: not silence, if they are not 0.
    pub peak: f32,
    pub rms: f64,
}

/// The decoder and the output of one session.
pub struct Sound {
    shared: Arc<Mutex<Shared>>,
    decoder: opus_rs::OpusDecoder,
    pcm: Vec<f32>,
    refused: u64,
    began: Instant,
    was_marked: bool,
    output: AudioOutput,
}

fn locked(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    // A lock that was held by a thread that panicked: its state is still
    // the best there is, and the sound goes on.
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `--clock-skew-ppm`: by how many parts in a million [`micros_since`]
/// counts faster than the machine's clock. For measuring; 0 unless the
/// flag says otherwise, and set once before anything is counted.
pub static CLOCK_SKEW_PPM: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// The microseconds from `began` to `at`, on the viewer's clock.
pub fn micros_since(began: Instant, at: Instant) -> u64 {
    let real = i128::try_from(at.saturating_duration_since(began).as_micros()).unwrap_or(i128::MAX);
    // Relaxed: one number, written once at the start.
    let ppm = i128::from(CLOCK_SKEW_PPM.load(std::sync::atomic::Ordering::Relaxed));
    u64::try_from(real + real * ppm / 1_000_000).unwrap_or(u64::MAX)
}

impl Sound {
    /// Opens the output. Times are counted from `began`.
    /// With `with_output` false no device is opened ([`AudioOutput::silent`]).
    pub fn open(volume: Arc<Volume>, began: Instant, with_output: bool) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let for_output = Arc::clone(&shared);
        let fill = move |first: Instant, out: &mut [f32]| {
            let mut shared = locked(&for_output);
            shared.playout.fill(micros_since(began, first), out);
            let volume = volume.now();
            for sample in out.iter_mut() {
                shared.level.peak = shared.level.peak.max(sample.abs());
                shared.level.squares += f64::from(*sample) * f64::from(*sample);
                shared.level.samples += 1.0;
                *sample *= volume;
            }
        };
        let output = if with_output {
            AudioOutput::open(fill)?
        } else {
            AudioOutput::silent(fill)
        };
        Ok(Self {
            shared,
            decoder: opus_rs::OpusDecoder::new(i32::try_from(RATE)?, 1)
                .map_err(|error| anyhow!("the decoder of Opus: {error}"))?,
            pcm: vec![0.0; MOST_SAMPLES],
            refused: 0,
            began,
            was_marked: false,
            output,
        })
    }

    /// Decodes one packet and queues it: `due_us` is its own time, and it
    /// is played there or after the packet before it
    /// ([`xmoq_watch::playout::place`]). Says
    /// whether the beep of `marks` begins with it.
    ///
    /// WHY decoded by the caller, on the task that reads the session, and
    /// outside the lock the output takes: 2.5 ms for a packet of 20 ms, and
    /// the output must not wait for it.
    pub fn take(&mut self, packet: &[u8], due_us: u64, marks: Option<&Marks>) -> bool {
        let Ok(samples) = self.decoder.decode(packet, MOST_SAMPLES, &mut self.pcm) else {
            self.refused += 1;
            return false;
        };
        let samples = self.pcm.get(..samples).unwrap_or_default().to_vec();
        let marked = marks.is_some_and(|marks| marks.sound_is_marked(&samples));
        let begins = marked && !self.was_marked;
        self.was_marked = marked;
        let now_us = micros_since(self.began, Instant::now());
        locked(&self.shared)
            .playout
            .push(now_us, Packet { due_us, samples });
        begins
    }

    /// No more packets come.
    pub fn end(&self) {
        locked(&self.shared).playout.end();
    }

    pub fn heard(&self) -> Heard {
        let shared = locked(&self.shared);
        let level = &shared.level;
        Heard {
            counts: shared.playout.counts().clone(),
            refused: self.refused,
            peak: level.peak,
            rms: if level.samples > 0.0 {
                (level.squares / level.samples).sqrt()
            } else {
                0.0
            },
        }
    }

    /// How long before a sample leaves the machine the output asks for it
    /// ([`AudioOutput::lead_us`]).
    pub async fn lead_us(&self) -> u64 {
        self.output.lead_us().await
    }

    pub fn began(&self) -> Instant {
        self.began
    }
}
