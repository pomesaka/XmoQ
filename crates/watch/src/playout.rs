//! Which sample the sound output gets next: packets of decoded sound, each
//! played at the moment it is due.
//!
//! Times are microseconds of the viewer's clock ([`crate::clock`]); the
//! sound is one channel at 48 kHz, which is what Opus is decoded to.

use std::collections::VecDeque;

/// Samples a second.
pub const RATE: u32 = 48_000;

/// A packet that follows the one before it is played at once when it is
/// due within this, sooner or later: a gap or an overlap of a few samples
/// would be a click.
const JOINS_US: u64 = 10_000;

/// The most packets that wait. Two seconds of packets of 20 ms.
const MOST_WAITING: usize = 100;

/// Decoded sound that is due at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    /// When its first sample is due.
    pub due_us: u64,
    /// Its samples, one channel.
    pub samples: Vec<f32>,
}

/// What happened to the packets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    /// Packets taken.
    pub packets: u64,
    /// Samples that were played.
    pub played: u64,
    /// Calls of the output that found nothing to play though a packet had
    /// been played before and more were to come.
    pub underruns: u64,
    /// Samples of silence while the next packet was not due yet.
    pub waited: u64,
    /// Packets that were over when their turn came.
    pub left_out_late: u64,
    /// Packets whose beginning was left out because it was late.
    pub cut_late: u64,
    /// Packets left out because too many waited.
    pub dropped_full: u64,
}

/// The packets that wait, and where the output is in them.
#[derive(Debug, Default)]
pub struct Playout {
    waiting: VecDeque<Packet>,
    playing: Option<(Packet, usize)>,
    /// A packet was played.
    began: bool,
    /// No more packets come.
    ended: bool,
    /// The sample before this one was of a packet, not silence.
    flowing: bool,
    counts: Counts,
}

/// The microseconds `samples` samples last.
fn duration_us(samples: usize) -> u64 {
    u64::try_from(samples)
        .unwrap_or(u64::MAX)
        .saturating_mul(1_000_000)
        / u64::from(RATE)
}

impl Playout {
    /// Takes a packet. Where [`MOST_WAITING`] wait already, the oldest is
    /// left out for it.
    pub fn push(&mut self, packet: Packet) {
        self.counts.packets += 1;
        self.waiting.push_back(packet);
        while self.waiting.len() > MOST_WAITING {
            self.waiting.pop_front();
            self.counts.dropped_full += 1;
        }
    }

    /// No more packets come: what the output finds empty from here on is
    /// the end, and no underrun.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// What happened so far.
    #[must_use]
    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// Fills one call of the output: `out` gets the samples that leave the
    /// machine from `first_us` on, one after the other, and silence where
    /// nothing is due.
    pub fn fill(&mut self, first_us: u64, out: &mut [f32]) {
        let mut silent = false;
        for (index, sample) in out.iter_mut().enumerate() {
            let value = self.next(first_us.saturating_add(duration_us(index)));
            silent |= value.is_none();
            *sample = value.unwrap_or(0.0);
        }
        // WHY not once no more is to come: the output goes on asking until
        // it is closed.
        if silent && self.began && !self.ended && self.playing.is_none() && self.waiting.is_empty()
        {
            self.counts.underruns += 1;
        }
    }

    /// The sample to play at `at_us`, or `None` for silence.
    fn next(&mut self, at_us: u64) -> Option<f32> {
        loop {
            if let Some((packet, position)) = &mut self.playing {
                if let Some(value) = packet.samples.get(*position) {
                    *position += 1;
                    self.counts.played += 1;
                    self.flowing = true;
                    return Some(*value);
                }
                self.playing = None;
            }
            let Some(next) = self.waiting.front() else {
                self.flowing = false;
                return None;
            };
            // WHY a packet joins the one before it up to 10 ms early, and
            // one that follows silence does not: joined, the sound has no
            // gap of a few samples; begun early, every packet after it is
            // played that much early as well, and the buffer is that much
            // smaller for good.
            let early = if self.flowing { JOINS_US } else { 0 };
            if next.due_us > at_us.saturating_add(early) {
                if self.began {
                    self.counts.waited += 1;
                }
                self.flowing = false;
                return None;
            }
            let packet = self.waiting.pop_front()?;
            // A packet that is late begins at the sample that is due now,
            // so that the sound does not stay behind for good; one that is
            // over already is left out. WHY not played whole: what comes
            // first after SUBSCRIBE is the Group that is under way, up to
            // 200 ms old.
            let late_us = at_us.saturating_sub(packet.due_us);
            let skipped = if late_us > JOINS_US {
                usize::try_from(late_us.saturating_mul(u64::from(RATE)) / 1_000_000)
                    .unwrap_or(usize::MAX)
            } else {
                0
            };
            if skipped >= packet.samples.len() {
                self.counts.left_out_late += 1;
                continue;
            }
            if skipped > 0 {
                self.counts.cut_late += 1;
            }
            self.began = true;
            self.playing = Some((packet, skipped));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packet of 20 ms whose samples all have `value`.
    fn packet(due_us: u64, value: f32) -> Packet {
        Packet {
            due_us,
            samples: vec![value; 960],
        }
    }

    /// 10 ms of output from `first_us`: the value of its first and of its
    /// last sample, 0 for silence.
    fn ten_ms(playout: &mut Playout, first_us: u64) -> (f32, f32) {
        let mut out = [9.0; 480];
        playout.fill(first_us, &mut out);
        (out[0], out[479])
    }

    #[test]
    fn packets_are_played_when_they_are_due_and_joined_to_each_other() {
        let mut playout = Playout::default();
        playout.push(packet(100_000, 1.0));
        // Due 5 ms early for where the output will be: it joins.
        playout.push(packet(115_000, 2.0));
        // Due 30 ms after the end of the one before: silence first.
        playout.push(packet(170_000, 3.0));
        // (the call begins at; its first and its last sample)
        let cases = [
            (80_000, (0.0, 0.0)),
            // Not begun early after silence: the packet is due in the middle.
            (95_000, (0.0, 1.0)),
            (105_000, (1.0, 1.0)),
            // 20 ms of it are played at 120 ms, and the next one follows at once.
            (115_000, (1.0, 2.0)),
            (125_000, (2.0, 2.0)),
            (135_000, (2.0, 0.0)),
            (145_000, (0.0, 0.0)),
            (165_000, (0.0, 3.0)),
        ];
        for (first_us, samples) in cases {
            assert_eq!(ten_ms(&mut playout, first_us), samples, "{first_us}");
        }
        assert_eq!(
            *playout.counts(),
            Counts {
                packets: 3,
                // 960, 960, and the 5 ms of the third.
                played: 2_160,
                underruns: 0,
                // Of the calls above: from 140 to 155 ms, and from 165 to 170 ms.
                waited: 960,
                ..Counts::default()
            }
        );
    }

    #[test]
    fn a_late_packet_begins_at_the_sample_that_is_due_and_one_that_is_over_is_left_out() {
        let mut playout = Playout::default();
        // Over 5 ms before the output begins.
        playout.push(packet(0, 1.0));
        // 15 ms of it are over.
        playout.push(packet(10_000, 2.0));
        let mut out = [9.0; 480];
        playout.fill(25_000, &mut out);
        // Its last 5 ms, and silence.
        assert_eq!((out[0], out[239], out[240]), (2.0, 2.0, 0.0));
        assert_eq!(
            *playout.counts(),
            Counts {
                packets: 2,
                played: 240,
                underruns: 1,
                left_out_late: 1,
                cut_late: 1,
                ..Counts::default()
            }
        );
    }

    #[test]
    fn a_call_that_finds_nothing_is_an_underrun_between_the_first_packet_and_the_end() {
        let mut playout = Playout::default();
        // Before anything was played: no underrun.
        assert_eq!(ten_ms(&mut playout, 0), (0.0, 0.0));
        playout.push(packet(10_000, 1.0));
        assert_eq!(ten_ms(&mut playout, 10_000), (1.0, 1.0));
        assert_eq!(ten_ms(&mut playout, 20_000), (1.0, 1.0));
        assert_eq!(playout.counts().underruns, 0);
        // Nothing waits.
        assert_eq!(ten_ms(&mut playout, 30_000), (0.0, 0.0));
        assert_eq!(playout.counts().underruns, 1);
        // A packet that waits and is not due yet is no underrun.
        playout.push(packet(100_000, 2.0));
        assert_eq!(ten_ms(&mut playout, 40_000), (0.0, 0.0));
        assert_eq!(playout.counts().underruns, 1);
        assert_eq!(ten_ms(&mut playout, 100_000), (2.0, 2.0));
        assert_eq!(ten_ms(&mut playout, 110_000), (2.0, 2.0));
        // After the end nothing is one.
        playout.end();
        assert_eq!(ten_ms(&mut playout, 120_000), (0.0, 0.0));
        assert_eq!(playout.counts().underruns, 1);
    }

    #[test]
    fn the_oldest_packets_are_left_out_where_too_many_wait() {
        let mut playout = Playout::default();
        for number in 0..103u16 {
            playout.push(packet(u64::from(number) * 20_000, f32::from(number)));
        }
        assert_eq!(playout.counts().dropped_full, 3);
        // The first that is left: number 3, due at 60 ms.
        assert_eq!(ten_ms(&mut playout, 60_000), (3.0, 3.0));
    }
}
