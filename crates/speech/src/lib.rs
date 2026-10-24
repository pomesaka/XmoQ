//! The words of audio tracks: the packets of Opus of a track are decoded
//! to one channel at 16 kHz ([`decode`]) and cut where there is a voice
//! ([`cut`]) by a [`Listener`] of that track, and each piece is read by
//! whisper.cpp ([`whisper`]). `xmoq-transcriber` has a listener for each
//! track and one reader for all of them.
//!
//! WHY one reader: a model is loaded once (the large one is 550 MiB), and
//! whisper.cpp reads one piece at a time as fast as it reads several
//! (README). [`Words::text`] takes as long as whisper.cpp does: a fifth of
//! the piece's length with the default model on a Mac, more without a
//! graphics processor. The caller keeps it off the task that reads the
//! session.

pub mod cut;
pub mod decode;
pub mod model;
pub mod whisper;

use anyhow::Result;

use crate::cut::{Cut, Cutter, Piece};
use crate::decode::Decoder;
use crate::whisper::Whisper;

/// One packet of an audio track.
#[derive(Clone, Copy)]
pub struct AudioPacket<'a> {
    /// The Timestamp of LOC of the Object, in microseconds.
    pub timestamp_us: u64,
    /// The Object's payload: one packet of Opus.
    pub opus: &'a [u8],
}

/// What a listener left out, counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Left {
    /// Packets the decoder refused.
    pub refused: u64,
    /// Pieces with too little voice to be read ([`Cut::least_voice_ms`]).
    pub too_little_voice: u64,
}

/// What reads the words of a piece: whisper.cpp, or a stand-in in tests.
pub trait Words {
    /// The words of `samples`, one channel at 16 kHz, trimmed; empty where
    /// there are none.
    ///
    /// # Errors
    /// When they cannot be read.
    fn text(&mut self, samples: &[f32]) -> Result<String>;
}

impl Words for Whisper {
    fn text(&mut self, samples: &[f32]) -> Result<String> {
        Whisper::text(self, samples)
    }
}

/// Decodes and cuts one audio track.
pub struct Listener {
    decoder: Decoder,
    cutter: Cutter,
    refused: u64,
}

impl Listener {
    /// A listener that cuts by `cut`.
    ///
    /// # Errors
    /// When the decoder of Opus cannot be made.
    pub fn new(cut: Cut) -> Result<Self> {
        Ok(Self {
            decoder: Decoder::new()?,
            cutter: Cutter::new(cut),
            refused: 0,
        })
    }

    /// One packet. Gives the pieces that end with it.
    pub fn push(&mut self, packet: AudioPacket<'_>) -> Vec<Piece> {
        let Some(samples) = self.decoder.decode(packet.opus) else {
            self.refused += 1;
            return Vec::new();
        };
        self.cutter.push(packet.timestamp_us, samples)
    }

    /// The track ended: the piece under way, if it has voice enough.
    pub fn end(&mut self) -> Option<Piece> {
        self.cutter.end()
    }

    /// What was left out.
    #[must_use]
    pub fn left(&self) -> Left {
        Left {
            refused: self.refused,
            too_little_voice: self.cutter.left().too_little_voice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_packets(count: usize, level: f32) -> Vec<Vec<u8>> {
        let mut encoder =
            opus_rs::OpusEncoder::new(48_000, 1, opus_rs::Application::Audio).unwrap();
        (0..count)
            .map(|n| {
                let pcm: Vec<f32> = (0..960)
                    .map(|i| {
                        let t =
                            f32::from(u16::try_from((n * 960 + i) % 48_000).unwrap()) / 48_000.0;
                        level * (2.0 * std::f32::consts::PI * 300.0 * t).sin()
                    })
                    .collect();
                let mut out = vec![0; 1500];
                let len = encoder.encode(&pcm, 960, &mut out).unwrap();
                out.truncate(len);
                out
            })
            .collect()
    }

    #[test]
    fn a_tone_between_silences_is_one_piece_on_the_clock_of_its_timestamps() {
        let mut listener = Listener::new(Cut::default()).unwrap();
        let mut packets = tone_packets(25, 0.0);
        packets.extend(tone_packets(50, 0.3));
        packets.extend(tone_packets(50, 0.0));
        let mut pieces = Vec::new();
        for (n, opus) in packets.iter().enumerate() {
            let timestamp_us = 5_000_000 + u64::try_from(n).unwrap() * 20_000;
            pieces.extend(listener.push(AudioPacket { timestamp_us, opus }));
        }
        pieces.extend(listener.end());
        // The tone is 0.5 s to 1.5 s from the first packet; the decoder's
        // delay and the 200 ms kept before and after move its edges.
        let [piece] = pieces.as_slice() else {
            panic!("{} pieces", pieces.len());
        };
        let span = (piece.start_us, piece.end_us);
        assert!(
            (5_280_000..=5_340_000).contains(&piece.start_us),
            "{span:?}"
        );
        assert!((6_680_000..=6_760_000).contains(&piece.end_us), "{span:?}");
        let samples = u64::try_from(piece.samples.len()).unwrap();
        assert_eq!(samples * 1_000_000 / 16_000, piece.end_us - piece.start_us);
        assert_eq!(listener.left(), Left::default());
    }

    #[test]
    fn a_packet_the_decoder_refuses_is_counted() {
        let mut listener = Listener::new(Cut::default()).unwrap();
        // A code of 3 with a frame count of 0 (RFC 6716 §3.2.5): not a packet.
        let pieces = listener.push(AudioPacket {
            timestamp_us: 0,
            opus: &[0x03, 0x00, 0x00],
        });
        assert_eq!(pieces, []);
        assert_eq!(listener.left().refused, 1);
    }
}
