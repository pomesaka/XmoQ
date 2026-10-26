//! The decoded sound of a track in frames of 20 ms, each with the Timestamp
//! of its first sample: the unit whether there is a voice is judged in
//! ([`crate::judge`]) and the sound is cut in ([`crate::cut`]).

use crate::decode::RATE;

/// Samples of a frame: 20 ms at [`RATE`].
pub const FRAME: usize = 320;
/// The microseconds of one frame.
pub const FRAME_US: u64 = 20_000;

/// The latest a packet's sound may end at, in microseconds: 2^53 - 1. A
/// packet that ends later is not taken ([`Framer::push`]).
///
/// WHY: the times of a piece go into the payloads of a transcript, whose
/// numbers stop there (`MAX_NUMBER` of `xmoq-transcript-tracks`, so that a
/// JavaScript number holds them exactly), and below it nothing that is
/// added to a Timestamp here overflows; the publisher chose the Timestamp,
/// which may be as large as a vi64 of §1.4.1 of the transport, 2^64 - 1.
pub const LATEST_US: u64 = (1 << 53) - 1;

/// A packet whose Timestamp is this far from where the sound before it
/// ended begins anew: packets were lost, or the publisher began again.
const GAP_US: u64 = 100_000;

/// What the framer gives.
#[derive(Clone, Debug, PartialEq)]
pub enum Framed {
    /// A frame of [`FRAME`] samples.
    Frame {
        /// The Timestamp of LOC of its first sample, in microseconds.
        at: u64,
        /// The samples.
        samples: Vec<f32>,
    },
    /// The sound does not go on from the frame before: what was under way
    /// is over.
    Gap,
}

/// Puts the packets of one track into frames.
#[derive(Debug, Default)]
pub struct Framer {
    /// Samples that do not fill a frame yet, and the Timestamp of the first.
    partial: Vec<f32>,
    partial_us: u64,
    /// The Timestamp the next sample has where the sound goes on.
    next_us: Option<u64>,
}

impl Framer {
    /// The samples of one packet, whose first sample has the Timestamp
    /// `start_us`. The samples of a frame that is not full wait for the
    /// next packet. `None`, and nothing is taken, for a packet that ends
    /// after [`LATEST_US`].
    pub fn push(&mut self, start_us: u64, samples: &[f32]) -> Option<Vec<Framed>> {
        let end_us = start_us
            .checked_add(micros(samples.len()))
            .filter(|end_us| *end_us <= LATEST_US)?;
        let mut framed = Vec::new();
        if self
            .next_us
            .is_some_and(|next| start_us.abs_diff(next) > GAP_US)
        {
            self.partial.clear();
            framed.push(Framed::Gap);
        }
        if self.partial.is_empty() {
            self.partial_us = start_us;
        }
        self.next_us = Some(end_us);
        self.partial.extend_from_slice(samples);
        while self.partial.len() >= FRAME {
            let samples: Vec<f32> = self.partial.drain(..FRAME).collect();
            framed.push(Framed::Frame {
                at: self.partial_us,
                samples,
            });
            self.partial_us += FRAME_US;
        }
        Some(framed)
    }

    /// The track ended: what does not fill a frame is let go of.
    pub fn end(&mut self) {
        self.partial.clear();
        self.next_us = None;
    }
}

/// The microseconds of `samples` at [`RATE`].
#[must_use]
pub fn micros(samples: usize) -> u64 {
    u64::try_from(samples).unwrap_or(u64::MAX) * 1_000_000 / u64::from(RATE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a framer made of packets: the Timestamps of the frames, and
    /// `None` for a gap.
    fn framed(packets: &[(u64, usize)]) -> Vec<Option<u64>> {
        let mut framer = Framer::default();
        packets
            .iter()
            .flat_map(|(start_us, samples)| {
                framer
                    .push(*start_us, &vec![0.0; *samples])
                    .expect("a Timestamp in range")
            })
            .map(|framed| match framed {
                Framed::Frame { at, .. } => Some(at),
                Framed::Gap => None,
            })
            .collect()
    }

    #[test]
    fn packets_are_put_into_frames_of_20_ms() {
        // The name, the packets (Timestamp, samples), and the frames.
        type Case = (
            &'static str,
            &'static [(u64, usize)],
            &'static [Option<u64>],
        );
        let cases: [Case; 4] = [
            (
                "packets of 20 ms",
                &[(0, 320), (20_000, 320)],
                &[Some(0), Some(20_000)],
            ),
            (
                "a packet of 60 ms is three frames",
                &[(0, 960)],
                &[Some(0), Some(20_000), Some(40_000)],
            ),
            (
                "packets of 10 ms make a frame of two",
                &[(0, 160), (10_000, 160), (20_000, 160)],
                &[Some(0)],
            ),
            (
                "a Timestamp 10 s on is a gap, and the half frame before it goes",
                &[(0, 480), (10_000_000, 320)],
                &[Some(0), None, Some(10_000_000)],
            ),
        ];
        for (name, packets, expected) in cases {
            assert_eq!(framed(packets), expected, "{name}");
        }
    }

    #[test]
    fn a_packet_that_ends_after_the_latest_timestamp_is_not_taken() {
        // The name, the Timestamp of a packet of 20 ms, and whether it is
        // taken.
        let cases: [(&str, u64, bool); 4] = [
            ("one that ends at the latest", LATEST_US - FRAME_US, true),
            (
                "one that ends a microsecond later",
                LATEST_US - FRAME_US + 1,
                false,
            ),
            ("one that begins after it", LATEST_US + 1, false),
            // Added to, it would overflow (a panic in a debug build).
            ("the largest a Timestamp can be", u64::MAX, false),
        ];
        for (name, start_us, taken) in cases {
            let taken_by = Framer::default().push(start_us, &[0.0; FRAME]);
            assert_eq!(taken_by.is_some(), taken, "{name}");
        }
    }

    /// The latest Timestamp is the largest number of a transcript.
    #[test]
    fn the_latest_timestamp_is_the_largest_number_of_a_transcript() {
        assert_eq!(LATEST_US, xmoq_transcript_tracks::payloads::MAX_NUMBER);
    }
}
