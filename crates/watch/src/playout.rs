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

/// How far from its own time a packet may begin so as to follow the one
/// before it without a seam: less than half a packet of 20 ms.
/// **Provisional.** As
/// `SEAM_SECONDS` of `@xmoq/media` (`packages/media/src/core/audio-seam.ts`),
/// from where the three values are; `fixtures/playout/seam.tsv` has what
/// both go by.
pub const SEAM_US: u64 = 8_000;
/// How much slower or faster a packet is played to get back to its own
/// time: a twentieth, so that 20 ms of sound make up 1 ms. **Provisional.**
pub const STRETCH_ONE_IN: u64 = 20;
/// How far from its own time the sound is brought back by speed alone.
/// **Provisional.**
pub const ABSORB_US: u64 = 300_000;

/// Where a packet begins ([`place`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Its time is over: it is not played.
    TooLate,
    /// It is played.
    Sound {
        /// When its first sample is played.
        begin_us: u64,
        /// Where it ends: longer or shorter than it is, where it is
        /// played slower or faster.
        until_us: u64,
        /// It follows the packet before it to the sample.
        follows: bool,
        /// It is played slower or faster.
        stretched: bool,
    },
}

/// Says where a packet begins: `wanted_us` is its own time, with the
/// buffer; `sound_until_us` where the packet before it ends.
///
/// **A packet follows the one before it to the sample**, as long as that
/// one still sounds and ends within [`ABSORB_US`] of the packet's own
/// time. **Where the two are more than [`SEAM_US`] apart, the packet is
/// played a little slower or faster**, packet after packet, until the
/// sound is at its own time again. WHY: a buffer that moves moves the time
/// of every packet, and a packet that jumps there leaves a gap or is cut.
/// A changed speed is a changed pitch for a moment, and no click. Beyond
/// [`ABSORB_US`], or where nothing sounds any more, the packet begins at
/// its own time: that is a pause, not a drift.
#[must_use]
pub fn place(
    wanted_us: u64,
    sound_until_us: Option<u64>,
    now_us: u64,
    duration_us: u64,
) -> Placement {
    let sounding = sound_until_us.filter(|until| *until >= now_us);
    let ahead = sounding.map(|until| i128::from(wanted_us) - i128::from(until));
    let follows = ahead.filter(|ahead| ahead.unsigned_abs() <= u128::from(ABSORB_US));
    let begin_us = match (follows, sounding) {
        (Some(_), Some(until)) => until,
        _ => wanted_us,
    };
    if begin_us < now_us {
        return Placement::TooLate;
    }
    // How much longer (or shorter) the packet sounds than it is.
    let most = i128::from(duration_us / STRETCH_ONE_IN);
    let longer = follows
        .filter(|ahead| ahead.unsigned_abs() > u128::from(SEAM_US))
        .map_or(0, |ahead| ahead.clamp(-most, most));
    let sounds_us = u64::try_from(i128::from(duration_us) + longer).unwrap_or(0);
    Placement::Sound {
        begin_us,
        until_us: begin_us.saturating_add(sounds_us),
        follows: follows.is_some(),
        stretched: longer != 0,
    }
}

/// `samples` as `length` samples that sound the same, slower or faster:
/// each is on the line between the two it lies between.
fn stretched(samples: &[f32], length: usize) -> Vec<f32> {
    let Some(last) = samples.len().checked_sub(1) else {
        return Vec::new();
    };
    (0..length)
        .map(|index| {
            // Counted in whole samples and a rest, so that nothing is
            // rounded twice.
            let at = index * samples.len();
            let (whole, rest) = (at / length, at % length);
            let here = samples.get(whole).copied().unwrap_or(0.0);
            let next = samples.get((whole + 1).min(last)).copied().unwrap_or(here);
            // WHY by way of a u16: an f32 has every one of them exactly. A
            // packet of Opus is 5,760 samples at most, and 6,048 stretched.
            let small = |count: usize| f32::from(u16::try_from(count).unwrap_or(u16::MAX));
            let share = small(rest) / small(length).max(1.0);
            here + (next - here) * share
        })
        .collect()
}

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
    /// Packets played slower or faster, to bring the sound back to its
    /// own time.
    pub stretched: u64,
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
    /// Where the packet taken last ends.
    sound_until_us: Option<u64>,
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
    /// Takes a packet whose own time is `packet.due_us`, at `now_us`, and
    /// has it begin where [`place`] says. Where [`MOST_WAITING`] wait
    /// already, the oldest is left out for it.
    pub fn push(&mut self, now_us: u64, packet: Packet) {
        self.counts.packets += 1;
        let lasts_us = duration_us(packet.samples.len());
        let Placement::Sound {
            begin_us,
            until_us,
            stretched: is_stretched,
            ..
        } = place(packet.due_us, self.sound_until_us, now_us, lasts_us)
        else {
            self.counts.left_out_late += 1;
            return;
        };
        self.sound_until_us = Some(until_us);
        let samples = if is_stretched {
            self.counts.stretched += 1;
            let length =
                usize::try_from((until_us - begin_us).saturating_mul(u64::from(RATE)) / 1_000_000)
                    .unwrap_or(packet.samples.len());
            stretched(&packet.samples, length)
        } else {
            packet.samples
        };
        self.waiting.push_back(Packet {
            due_us: begin_us,
            samples,
        });
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

    /// `fixtures/playout/seam.tsv`, which `@xmoq/media` reads too.
    #[test]
    fn a_packet_is_placed_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/playout/seam.tsv");
        let number = |field: &str| field.parse::<u64>().expect(field);
        let yes = |field: &str| match field {
            "yes" => true,
            "no" => false,
            other => panic!("yes or no: {other}"),
        };
        let mut rows = 0;
        for row in table.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [
                name,
                wanted,
                until,
                now,
                what,
                begin,
                ends,
                follows,
                stretched,
            ] = fields[..]
            else {
                panic!("a row of 9 fields: {row}");
            };
            let sound_until = (until != "-").then(|| number(until));
            let expected = match what {
                "too-late" => Placement::TooLate,
                "sound" => Placement::Sound {
                    begin_us: number(begin),
                    until_us: number(ends),
                    follows: yes(follows),
                    stretched: yes(stretched),
                },
                other => panic!("sound or too-late: {other}"),
            };
            assert_eq!(
                place(number(wanted), sound_until, number(now), 20_000),
                expected,
                "{name}"
            );
            rows += 1;
        }
        assert_eq!(rows, 9);
    }

    #[test]
    fn a_packet_is_made_longer_or_shorter_on_the_lines_between_its_samples() {
        // (the samples; as how many; what they become)
        let cases: [(&[f32], usize, &[f32]); 4] = [
            (&[0.0, 2.0], 4, &[0.0, 1.0, 2.0, 2.0]),
            (&[0.0, 1.0, 2.0, 3.0], 2, &[0.0, 2.0]),
            (&[1.0, 2.0], 2, &[1.0, 2.0]),
            (&[], 3, &[]),
        ];
        for (samples, length, expected) in cases {
            assert_eq!(stretched(samples, length), expected, "{samples:?}");
        }
    }

    #[test]
    fn packets_follow_each_other_and_begin_at_their_own_time_after_a_pause() {
        let mut playout = Playout::default();
        playout.push(0, packet(100_000, 1.0));
        // Its own time is 5 ms before the end of the one before: it follows.
        playout.push(0, packet(115_000, 2.0));
        // 30 ms after the end of the one before: it follows as well, 1 ms
        // longer than it is, from 140 to 161 ms.
        playout.push(0, packet(170_000, 3.0));
        // More than 0.3 s after: silence, and its own time.
        playout.push(0, packet(500_000, 4.0));
        // (the call begins at; its first and its last sample)
        let cases = [
            (80_000, (0.0, 0.0)),
            // Not begun early after silence: the packet is due in the middle.
            (95_000, (0.0, 1.0)),
            (105_000, (1.0, 1.0)),
            (115_000, (1.0, 2.0)),
            (125_000, (2.0, 2.0)),
            (135_000, (2.0, 3.0)),
            (145_000, (3.0, 3.0)),
            (155_000, (3.0, 0.0)),
            (495_000, (0.0, 4.0)),
        ];
        for (first_us, samples) in cases {
            assert_eq!(ten_ms(&mut playout, first_us), samples, "{first_us}");
        }
        assert_eq!(
            *playout.counts(),
            Counts {
                packets: 4,
                // 960, 960, the 1,008 of the third and 5 ms of the fourth.
                played: 3_168,
                underruns: 0,
                // Of the calls above: from 161 to 165 ms, and from 495 to 500 ms.
                waited: 432,
                stretched: 1,
                ..Counts::default()
            }
        );
    }

    #[test]
    fn a_late_packet_begins_at_the_sample_that_is_due_and_one_that_is_over_is_left_out() {
        let mut playout = Playout::default();
        // Over before the output comes to it.
        playout.push(0, packet(0, 1.0));
        // Its own time, after a pause; 15 ms of it are over at the output.
        playout.push(0, packet(400_000, 2.0));
        // Over when it comes: not taken at all.
        playout.push(450_000, packet(100_000, 3.0));
        let mut out = [9.0; 480];
        playout.fill(415_000, &mut out);
        // Its last 5 ms, and silence.
        assert_eq!((out[0], out[239], out[240]), (2.0, 2.0, 0.0));
        assert_eq!(
            *playout.counts(),
            Counts {
                packets: 3,
                played: 240,
                underruns: 1,
                left_out_late: 2,
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
        playout.push(0, packet(10_000, 1.0));
        assert_eq!(ten_ms(&mut playout, 10_000), (1.0, 1.0));
        assert_eq!(ten_ms(&mut playout, 20_000), (1.0, 1.0));
        assert_eq!(playout.counts().underruns, 0);
        // Nothing waits.
        assert_eq!(ten_ms(&mut playout, 30_000), (0.0, 0.0));
        assert_eq!(playout.counts().underruns, 1);
        // A packet that waits and is not due yet is no underrun.
        playout.push(40_000, packet(100_000, 2.0));
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
            playout.push(0, packet(u64::from(number) * 20_000, f32::from(number)));
        }
        assert_eq!(playout.counts().dropped_full, 3);
        // The first that is left: number 3, due at 60 ms.
        assert_eq!(ten_ms(&mut playout, 60_000), (3.0, 3.0));
    }
}
