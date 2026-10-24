//! Packets of Opus decoded to what whisper.cpp reads: one channel at
//! 16 kHz.
//!
//! WHY the decoder gives 16 kHz itself, and no resampler: an Opus decoder
//! may give its output at 8, 12, 16, 24 or 48 kHz whatever the encoder's
//! rate was (RFC 6716 §2), and one of one channel mixes a stereo packet
//! down. `opus-rs` 0.1.39 takes 16000 and 1 (`OpusDecoder::init`).

use anyhow::{Result, anyhow};

/// The rate whisper.cpp reads (`WHISPER_SAMPLE_RATE`).
pub const RATE: u32 = 16_000;

/// Samples of the longest packet of Opus, 120 ms at [`RATE`]: what the
/// decoder is given room for.
const MOST_SAMPLES: usize = 1920;

/// The decoder of one track.
pub struct Decoder {
    decoder: opus_rs::OpusDecoder,
    pcm: Vec<f32>,
}

impl Decoder {
    /// A decoder of one channel at [`RATE`].
    ///
    /// # Errors
    /// When `opus-rs` refuses the rate, which it does not at 0.1.39.
    pub fn new() -> Result<Self> {
        Ok(Self {
            decoder: opus_rs::OpusDecoder::new(i32::try_from(RATE)?, 1)
                .map_err(|error| anyhow!("the decoder of Opus: {error}"))?,
            pcm: vec![0.0; MOST_SAMPLES],
        })
    }

    /// The samples of one packet, or `None` where the decoder refused it.
    pub fn decode(&mut self, packet: &[u8]) -> Option<&[f32]> {
        let samples = self
            .decoder
            .decode(packet, MOST_SAMPLES, &mut self.pcm)
            .ok()?;
        self.pcm.get(..samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tone of 440 Hz at 48 kHz, as a browser sends it, in packets of 20 ms.
    fn tone_packets(count: usize) -> Vec<Vec<u8>> {
        let mut encoder =
            opus_rs::OpusEncoder::new(48_000, 1, opus_rs::Application::Audio).unwrap();
        let mut packets = Vec::new();
        for n in 0..count {
            let pcm: Vec<f32> = (0..960)
                .map(|i| {
                    let t = f32::from(u16::try_from(n * 960 + i).unwrap_or(0)) / 48_000.0;
                    0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                })
                .collect();
            let mut out = vec![0; 1500];
            let len = encoder.encode(&pcm, 960, &mut out).unwrap();
            out.truncate(len);
            packets.push(out);
        }
        packets
    }

    #[test]
    fn a_packet_of_20_ms_at_48_khz_gives_320_samples_at_16_khz() {
        let mut decoder = Decoder::new().unwrap();
        for packet in tone_packets(5) {
            assert_eq!(decoder.decode(&packet).map(<[f32]>::len), Some(320));
        }
    }

    #[test]
    fn the_tone_is_still_there_at_16_khz() {
        let mut decoder = Decoder::new().unwrap();
        let mut samples = Vec::new();
        for packet in tone_packets(25) {
            samples.extend_from_slice(decoder.decode(&packet).unwrap());
        }
        // Past the decoder's first packets; a sine of 0.5 has an RMS of 0.35.
        let tail = samples.get(1600..).unwrap();
        let rms = (tail.iter().map(|s| s * s).sum::<f32>()
            / f32::from(u16::try_from(tail.len()).unwrap()))
        .sqrt();
        assert!((0.25..0.45).contains(&rms), "rms {rms}");
        // 440 Hz crosses zero 880 times a second: 352 times in the 0.4 s here.
        let crossings = tail
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        assert!((330..375).contains(&crossings), "crossings {crossings}");
    }
}
