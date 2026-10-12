//! Decodes the Objects of an Opus track of LOC and plays them on the
//! machine's output, with Rust crates alone.
//!
//!   xmoq-moqt-client --mode subscribe ... --track audio \
//!       | opus-play [--channels 1] [--volume 0] [--buffer-ms 100]
//!
//! It reads the `OBJECT ... payload_hex=...` lines of the client: the payload
//! of an Object of a LOC audio track is one Opus packet. `--volume` is 0
//! unless given, so that a run makes no sound: what is counted (the peak and
//! the RMS of what the output asked for) is taken before the volume.
//!
//! Playing begins when `--buffer-ms` of sound wait, and a call of the output
//! that finds less than it asks for after that is an underrun: it gets
//! silence for the rest. More than a second waiting is dropped from the
//! front. An Object that did not come is not noticed: the packets are played
//! one after the other as they come.

use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: u32 = 48_000;

#[derive(Default)]
struct Shared {
    /// Interleaved samples for the device, not yet asked for.
    waiting: VecDeque<f32>,
    playing: bool,
    /// No more comes: an output that finds nothing is not an underrun.
    ended: bool,
    calls: u64,
    underruns: u64,
    given: u64,
    peak: f32,
    squares: f64,
}

#[cfg(not(feature = "libopus"))]
mod decoder {
    pub const NAME: &str = "opus-decoder 0.1.1";
    pub struct Decoder(opus_decoder::OpusDecoder);
    impl Decoder {
        pub fn new(channels: usize) -> Self {
            Self(opus_decoder::OpusDecoder::new(super::RATE, channels).expect("a decoder"))
        }
        /// Samples for each channel, or None for a packet it refuses.
        pub fn decode(&mut self, packet: &[u8], pcm: &mut [f32]) -> Option<usize> {
            self.0.decode_float(packet, pcm, false).ok()
        }
    }
}

#[cfg(feature = "libopus")]
mod decoder {
    pub const NAME: &str = "opus 0.4.0 (libopus)";
    pub struct Decoder(opus::Decoder);
    impl Decoder {
        pub fn new(channels: usize) -> Self {
            let channels = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };
            Self(opus::Decoder::new(super::RATE, channels).expect("a decoder"))
        }
        pub fn decode(&mut self, packet: &[u8], pcm: &mut [f32]) -> Option<usize> {
            self.0.decode_float(packet, pcm, false).ok()
        }
    }
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).filter_map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).ok()).collect()
}

fn main() {
    let mut channels = 1usize;
    let mut volume = 0f32;
    let mut buffer_ms = 100usize;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().expect("a value for the flag");
        match flag.as_str() {
            "--channels" => channels = value.parse().expect("--channels"),
            "--volume" => volume = value.parse().expect("--volume"),
            "--buffer-ms" => buffer_ms = value.parse().expect("--buffer-ms"),
            other => panic!("unknown flag {other}"),
        }
    }

    let device = cpal::default_host().default_output_device().expect("an output device");
    // The decoder gives 48 kHz and nothing here resamples: the device has to take it.
    let config = device
        .supported_output_configs()
        .expect("the configurations of the device")
        .filter(|range| range.sample_format() == cpal::SampleFormat::F32)
        .find_map(|range| range.try_with_sample_rate(RATE))
        .expect("an output of 48 kHz float samples");
    let out_channels = usize::from(config.channels());
    let shared = Arc::new(Mutex::new(Shared::default()));
    let for_output = Arc::clone(&shared);
    let begin_at = RATE as usize * out_channels * buffer_ms / 1000;
    let stream = device
        .build_output_stream(
            config.config(),
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut shared = for_output.lock().expect("the lock");
                shared.calls += 1;
                if !shared.playing && shared.waiting.len() >= begin_at {
                    shared.playing = true;
                }
                let mut short = false;
                for sample in out.iter_mut() {
                    let next = if shared.playing { shared.waiting.pop_front() } else { None };
                    match next {
                        Some(value) => {
                            shared.given += 1;
                            shared.peak = shared.peak.max(value.abs());
                            shared.squares += f64::from(value) * f64::from(value);
                            *sample = value * volume;
                        }
                        None => {
                            short = shared.playing;
                            *sample = 0.0;
                        }
                    }
                }
                if short && !shared.ended {
                    shared.underruns += 1;
                    // It waits for the buffer to fill again, not for one packet.
                    shared.playing = false;
                }
            },
            |error| eprintln!("AUDIO_OUTPUT_ERROR {error}"),
            None,
        )
        .expect("an output stream");
    stream.play().expect("the stream to play");

    let mut decoder = decoder::Decoder::new(channels);
    // 120 ms, the longest packet of Opus, for each channel.
    let mut pcm = vec![0f32; 5760 * channels];
    let (mut packets, mut refused, mut dropped) = (0u64, 0u64, 0u64);
    let mut decoding = Duration::ZERO;
    let started = Instant::now();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !line.starts_with("OBJECT ") {
            continue;
        }
        let Some(hex) = line.rsplit("payload_hex=").next() else { continue };
        let packet = unhex(hex.trim());
        if packet.is_empty() {
            continue;
        }
        let before = Instant::now();
        let decoded = decoder.decode(&packet, &mut pcm);
        decoding += before.elapsed();
        let Some(samples) = decoded else {
            refused += 1;
            continue;
        };
        packets += 1;
        let mut shared = shared.lock().expect("the lock");
        for frame in pcm[..samples * channels].chunks(channels) {
            // The device's channels are filled from the track's, the last one again where
            // the device has more.
            for channel in 0..out_channels {
                shared.waiting.push_back(frame[channel.min(channels - 1)]);
            }
        }
        let most = RATE as usize * out_channels;
        while shared.waiting.len() > most {
            shared.waiting.pop_front();
            dropped += 1;
        }
    }
    // What waits is played out, for no longer than it lasts and a little.
    shared.lock().expect("the lock").ended = true;
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until && !shared.lock().expect("the lock").waiting.is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    let shared = shared.lock().expect("the lock");
    let rms = if shared.given == 0 { 0.0 } else { (shared.squares / shared.given as f64).sqrt() };
    println!(
        "AUDIO_PLAYED decoder=\"{}\" output=\"cpal 0.18.2\" device_channels={out_channels} volume={volume} packets={packets} refused={refused} \
         decode_ms_total={:.1} seconds_given={:.2} wall_seconds={:.2} calls={} underruns={} dropped_samples={dropped} peak={:.4} rms={:.4}",
        decoder::NAME,
        decoding.as_secs_f64() * 1000.0,
        shared.given as f64 / (RATE as usize * out_channels) as f64,
        started.elapsed().as_secs_f64(),
        shared.calls,
        shared.underruns,
        shared.peak,
        rms,
    );
}
