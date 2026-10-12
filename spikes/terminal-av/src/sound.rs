//! The sound: packets of Opus decoded with the crate `opus-decoder`, played
//! with `cpal`, each at the moment it is due.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: u32 = 48_000;
/// A packet that is due within this of where the output is, is played at
/// once: a gap or an overlap of a few samples would be a click.
const JOINS: Duration = Duration::from_millis(10);
/// The beep of the demo is 0.6 of full scale and its tone 0.25.
const LOUD: f32 = 0.45;

struct Packet {
    due: Instant,
    samples: Vec<f32>,
    loud: bool,
}

#[derive(Default)]
pub struct Counts {
    pub packets: u64,
    pub refused: u64,
    pub underruns: u64,
    pub waits: u64,
    pub left_out_late: u64,
    /// Packets whose beginning was left out because it was late.
    pub cut_late: u64,
    pub dropped_full: u64,
    pub given: u64,
    pub peak: f32,
    pub squares: f64,
    /// For every packet: the moment the first sample played of it is
    /// played, less the moment that sample was due, in milliseconds.
    pub off_ms: Vec<f32>,
    /// The moments a beep began to be played.
    pub marks: Vec<Instant>,
}

#[derive(Default)]
struct Shared {
    waiting: VecDeque<Packet>,
    playing: Option<(Packet, usize)>,
    began: bool,
    was_loud: bool,
    counts: Counts,
}

/// How loud the sound is played, changed while it plays.
pub struct Volume {
    /// The bits of an f32, 0 to 1.
    level: AtomicU32,
    muted: AtomicBool,
}

impl Volume {
    pub fn new(level: f32) -> Self {
        Self { level: AtomicU32::new(level.to_bits()), muted: AtomicBool::new(false) }
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn change(&self, by: f32) {
        self.level.store((self.level() + by).clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn mute(&self) {
        self.muted.fetch_xor(true, Ordering::Relaxed);
    }

    fn now(&self) -> f32 {
        if self.muted() { 0.0 } else { self.level() }
    }
}

pub struct Sound {
    shared: Arc<Mutex<Shared>>,
    decoder: opus_decoder::OpusDecoder,
    pcm: Vec<f32>,
    _stream: cpal::Stream,
}

impl Sound {
    pub fn open(volume: Arc<Volume>) -> anyhow::Result<Self> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no output device"))?;
        // The decoder gives 48 kHz and nothing here resamples.
        let config = device
            .supported_output_configs()?
            .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
            .find_map(|range| range.try_with_sample_rate(RATE))
            .ok_or_else(|| anyhow::anyhow!("the output device takes no 48 kHz float samples"))?;
        let channels = usize::from(config.channels());
        let shared = Arc::new(Mutex::new(Shared::default()));
        let for_output = Arc::clone(&shared);
        let stream = device.build_output_stream(
            config.config(),
            move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                let times = info.timestamp();
                // When the first sample of this call leaves the machine.
                let ahead = times.playback.duration_since(times.callback);
                let first = Instant::now() + ahead;
                let mut shared = for_output.lock().expect("the lock");
                let volume = volume.now();
                let mut silent = false;
                for (index, frame) in out.chunks_mut(channels).enumerate() {
                    let at = first + Duration::from_secs_f64(index as f64 / f64::from(RATE));
                    let value = shared.next_sample(at);
                    silent |= value.is_none();
                    let value = value.unwrap_or(0.0);
                    for sample in frame {
                        *sample = value * volume;
                    }
                }
                if silent && shared.began && shared.playing.is_none() && shared.waiting.is_empty() {
                    shared.counts.underruns += 1;
                }
            },
            |error| eprintln!("AUDIO_OUTPUT_ERROR {error}"),
            None,
        )?;
        stream.play()?;
        Ok(Self {
            shared,
            decoder: opus_decoder::OpusDecoder::new(RATE, 1).map_err(|error| anyhow::anyhow!("{error}"))?,
            // 120 ms, the longest packet of Opus.
            pcm: vec![0.0; 5760],
            _stream: stream,
        })
    }

    /// Decodes one packet and queues it to be played at `due`.
    pub fn take(&mut self, packet: &[u8], due: Instant) {
        let decoded = self.decoder.decode_float(packet, &mut self.pcm, false);
        let mut shared = self.shared.lock().expect("the lock");
        let Ok(samples) = decoded else {
            shared.counts.refused += 1;
            return;
        };
        shared.counts.packets += 1;
        let samples = self.pcm[..samples].to_vec();
        let loud = samples.iter().any(|sample| sample.abs() > LOUD);
        shared.waiting.push_back(Packet { due, samples, loud });
        // Two seconds of packets of 20 ms.
        while shared.waiting.len() > 100 {
            shared.waiting.pop_front();
            shared.counts.dropped_full += 1;
        }
    }

    pub fn counts(&self) -> Counts {
        std::mem::take(&mut self.shared.lock().expect("the lock").counts)
    }
}

impl Shared {
    /// The sample to play at `at`, or None for silence.
    fn next_sample(&mut self, at: Instant) -> Option<f32> {
        loop {
            if let Some((packet, position)) = &mut self.playing {
                if let Some(value) = packet.samples.get(*position) {
                    *position += 1;
                    self.counts.given += 1;
                    self.counts.peak = self.counts.peak.max(value.abs());
                    self.counts.squares += f64::from(*value) * f64::from(*value);
                    return Some(*value);
                }
                self.playing = None;
            }
            let next = self.waiting.front()?;
            if next.due > at + JOINS {
                // Not yet: silence until it is due.
                if self.began {
                    self.counts.waits += 1;
                }
                return None;
            }
            let packet = self.waiting.pop_front()?;
            // A packet that is late begins at the sample that is due now, so that the sound
            // does not stay behind for good; one that is over already is left out. WHY not
            // played whole: what comes first after SUBSCRIBE is the Group that is under way,
            // up to 200 ms old.
            let late = at.saturating_duration_since(packet.due);
            let skipped = if late > JOINS { (late.as_secs_f64() * f64::from(RATE)) as usize } else { 0 };
            if skipped >= packet.samples.len() {
                self.counts.left_out_late += 1;
                continue;
            }
            if skipped > 0 {
                self.counts.cut_late += 1;
            }
            let begins = packet.due + Duration::from_secs_f64(skipped as f64 / f64::from(RATE));
            let off = if at >= begins {
                at.duration_since(begins).as_secs_f32() * 1000.0
            } else {
                -(begins.duration_since(at).as_secs_f32() * 1000.0)
            };
            self.counts.off_ms.push(off);
            if packet.loud && !self.was_loud {
                self.counts.marks.push(at);
            }
            self.was_loud = packet.loud;
            self.began = true;
            self.playing = Some((packet, skipped));
        }
    }
}
