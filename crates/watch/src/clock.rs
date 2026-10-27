//! When an Object is due, from its Timestamp of LOC.
//!
//! The Timestamps are of the publisher's clock, which the viewer does not
//! have. The smallest (arrival - Timestamp) seen on any track in the last
//! minute is taken for the least an Object needs on the way, and every
//! Object is due a buffer after its Timestamp plus that. Both tracks have to carry Timestamps of
//! one clock for their Objects to be due at matching moments.

/// How far from the fastest way an Object may be, sooner or later, and
/// still be of this clock: 10 seconds. **Provisional.**
///
/// WHY a limit at all: the Timestamp is the publisher's to say. One that
/// is years ahead would be the fastest way ever seen, and every Object
/// after it would be years late; one that is years behind would be held
/// for years. Neither says anything about the path. 10 seconds is far
/// beyond what a buffer holds and what a viewer waits for a live picture.
pub const MOST_FROM_THE_LEAST_US: u64 = 10_000_000;

/// After so many Objects in a row that are not of this clock, the clock
/// begins anew with the next: the publisher's clock has moved (it began
/// again), or the first Object was the one that was wrong. **Provisional**:
/// a second of video, more or less.
pub const ANEW_AFTER: u32 = 30;

/// How far back the least is looked for: it is the least of the Objects
/// that came in this second of the viewer's clock and the 59 before it.
/// **Provisional**, and the value of `@xmoq/media` (`FASTEST_WINDOW_MS` of
/// `packages/media/src/core/fastest-way.ts`, which has why a least that was
/// kept for good lost the sound of a viewer whose clock runs faster than
/// its publisher's, and why a minute). `fixtures/playout/least.tsv` has
/// what both go by.
pub const LEAST_WINDOW_SECONDS: u64 = 60;

/// The sound waits at least this long, also where the buffer is less
/// (`DEFAULT_AUDIO_MIN_MS` of `@xmoq/media`). **Provisional.**
pub const AUDIO_MIN_US: u64 = 30_000;

/// The automatic buffer (**a trial, not the default**; its values are
/// **provisional**), as that of
/// `@xmoq/media` (`packages/media/src/core/playout-clock.ts`), from where
/// every value here is: it is what 99 of 100 Objects of the last
/// [`AUTO_WINDOW_US`] came within, after the fastest, and
/// [`AUTO_MARGIN_US`] more, in steps of [`AUTO_STEP_US`] and no more than
/// [`AUTO_MOST_US`]. It grows at once and shrinks a step at a
/// [`Clock::tick`], which the caller gives twice a second. Fewer than
/// [`AUTO_LEAST_OBJECTS`] Objects move nothing, and no more than
/// [`AUTO_MOST_OBJECTS`], the newest, are kept.
/// `fixtures/playout/auto-buffer.tsv` has what both go by.
pub const AUTO_WINDOW_US: u64 = 10_000_000;
/// See [`AUTO_WINDOW_US`].
pub const AUTO_MARGIN_US: u64 = 10_000;
/// See [`AUTO_WINDOW_US`].
pub const AUTO_STEP_US: u64 = 10_000;
/// See [`AUTO_WINDOW_US`].
pub const AUTO_MOST_US: u64 = 400_000;
/// See [`AUTO_WINDOW_US`].
pub const AUTO_LEAST_OBJECTS: usize = 50;
/// See [`AUTO_WINDOW_US`].
pub const AUTO_MOST_OBJECTS: usize = 1600;

/// The clock of one session. Times are microseconds: arrivals and what is
/// due count from a moment of the viewer's choosing, Timestamps from the
/// publisher's.
#[derive(Debug, Clone)]
pub struct Clock {
    /// The smallest (arrival - Timestamp) of each second in which an Object
    /// of the clock came, the oldest first; at most
    /// [`LEAST_WINDOW_SECONDS`] of them.
    least: std::collections::VecDeque<(u64, i128)>,
    buffer_us: u64,
    /// The most the automatic buffer may become; `None`: it is not
    /// automatic.
    auto_most_us: Option<u64>,
    /// When each Object of the window came, and how long after the fastest.
    late: std::collections::VecDeque<(u64, u64)>,
    /// Objects in a row that were not of this clock.
    strays: u32,
}

/// When an Object would be shown or played with no buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base {
    /// Its Timestamp, by the fastest way: the moment on the viewer's clock
    /// an Object with this Timestamp comes at the soonest.
    pub at_us: u64,
    /// How much longer than the fastest Object so far it took to come.
    pub over_least_us: u64,
}

impl Clock {
    /// A clock whose buffer is `buffer_us` and stays so.
    #[must_use]
    pub fn new(buffer_us: u64) -> Self {
        Self {
            least: std::collections::VecDeque::new(),
            buffer_us,
            auto_most_us: None,
            late: std::collections::VecDeque::new(),
            strays: 0,
        }
    }

    /// A clock whose buffer begins at `buffer_us` and follows how late
    /// Objects come, up to `most_us` ([`AUTO_MOST_US`] where nothing else
    /// is asked).
    #[must_use]
    pub fn automatic(buffer_us: u64, most_us: u64) -> Self {
        Self {
            auto_most_us: Some(most_us),
            ..Self::new(buffer_us)
        }
    }

    /// Takes the arrival of an Object, and says when it would be shown with
    /// no buffer. `None` for an Object that is not of this clock
    /// ([`MOST_FROM_THE_LEAST_US`]): it is to be left out.
    ///
    /// WHY the least is taken before the answer: the first Object is then
    /// due a buffer after it came, and not at some moment of a clock that
    /// is not known yet.
    pub fn base(&mut self, arrived_us: u64, timestamp_us: u64) -> Option<Base> {
        let took = i128::from(arrived_us) - i128::from(timestamp_us);
        // A clock that was set back does not open a second before the last
        // one: the Object counts for the newest.
        let second = (arrived_us / 1_000_000).max(self.least.back().map_or(0, |(at, _)| *at));
        while self
            .least
            .front()
            .is_some_and(|(at, _)| at.saturating_add(LEAST_WINDOW_SECONDS) <= second)
        {
            self.least.pop_front();
        }
        let of_the_window = self.least.iter().map(|(_, least)| *least).min();
        if let Some(least) = of_the_window
            && (took - least).unsigned_abs() > u128::from(MOST_FROM_THE_LEAST_US)
        {
            self.strays += 1;
            if self.strays < ANEW_AFTER {
                return None;
            }
            self.least.clear();
            // What came by the clock before says nothing of this one.
            self.late.clear();
        }
        self.strays = 0;
        match self.least.back_mut() {
            Some((at, least)) if *at == second => *least = (*least).min(took),
            _ => self.least.push_back((second, took)),
        }
        let least = self
            .least
            .iter()
            .map(|(_, least)| *least)
            .min()
            .unwrap_or(took);
        let over_least_us = u64::try_from(took - least).unwrap_or(u64::MAX);
        self.forget(arrived_us);
        self.late.push_back((arrived_us, over_least_us));
        // WHY a limit beside the window: how many come in the window is
        // the publisher's to decide.
        if self.late.len() > AUTO_MOST_OBJECTS {
            self.late.pop_front();
        }
        Some(Base {
            at_us: u64::try_from((i128::from(timestamp_us) + least).max(0)).unwrap_or(u64::MAX),
            over_least_us,
        })
    }

    /// Lets go of what came longer ago than the window.
    fn forget(&mut self, now_us: u64) {
        let since = now_us.saturating_sub(AUTO_WINDOW_US);
        // `<=`, and only for a clock that is older than the window: as the
        // page's, whose window begins at its first Object.
        while now_us >= AUTO_WINDOW_US && self.late.front().is_some_and(|(at, _)| *at <= since) {
            self.late.pop_front();
        }
    }

    /// What 99 of 100 of the Objects of the window came within, after the
    /// fastest; 0 without any.
    #[must_use]
    pub fn lateness_us(&self) -> u64 {
        let mut sorted: Vec<u64> = self.late.iter().map(|(_, late)| *late).collect();
        sorted.sort_unstable();
        let index = (sorted.len() * 99 / 100).min(sorted.len().saturating_sub(1));
        sorted.get(index).copied().unwrap_or(0)
    }

    /// Moves the automatic buffer; does nothing without it.
    pub fn tick(&mut self, now_us: u64) {
        self.forget(now_us);
        let Some(most_us) = self.auto_most_us else {
            return;
        };
        if self.late.len() < AUTO_LEAST_OBJECTS {
            return;
        }
        let wanted = (self.lateness_us() + AUTO_MARGIN_US)
            .div_ceil(AUTO_STEP_US)
            .saturating_mul(AUTO_STEP_US)
            .min(most_us);
        if wanted > self.buffer_us {
            self.buffer_us = wanted;
        } else if wanted < self.buffer_us {
            self.buffer_us = self.buffer_us.saturating_sub(AUTO_STEP_US);
        }
    }

    /// How long after its base a frame is shown.
    #[must_use]
    pub fn buffer_us(&self) -> u64 {
        self.buffer_us
    }

    /// How long after its base a packet of sound is played: the buffer,
    /// and [`AUDIO_MIN_US`] at least.
    #[must_use]
    pub fn audio_buffer_us(&self) -> u64 {
        self.buffer_us.max(AUDIO_MIN_US)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(at_us: u64, over_least_us: u64) -> Base {
        Base {
            at_us,
            over_least_us,
        }
    }

    #[test]
    fn the_base_of_an_object_is_its_timestamp_by_the_fastest_way() {
        let mut clock = Clock::new(50_000);
        // (arrived, Timestamp; its base, over the least)
        let cases = [
            // The first: its own way is the least.
            (1_000_000, 900_000_000, 1_000_000, 0),
            // 20 ms later on the publisher's clock, 30 ms later here: 10 ms slow.
            (1_030_000, 900_020_000, 1_020_000, 10_000),
            // A faster one: the least becomes 5 ms less, and its base is by that.
            (1_035_000, 900_040_000, 1_035_000, 0),
            // After it, the same pace is 5 ms over the new least.
            (1_060_000, 900_060_000, 1_055_000, 5_000),
        ];
        for (arrived, timestamp, at_us, over_least_us) in cases {
            assert_eq!(
                clock.base(arrived, timestamp),
                Some(at(at_us, over_least_us)),
                "{arrived}"
            );
        }
    }

    #[test]
    fn a_base_is_not_before_the_beginning_of_the_viewers_clock() {
        let mut clock = Clock::new(0);
        assert_eq!(clock.base(10, 5_000), Some(at(10, 0)));
        // Earlier on the publisher's clock than the viewer's has room for.
        assert_eq!(clock.base(20, 0), Some(at(0, 5_010)));
    }

    #[test]
    fn an_object_far_from_the_clock_is_left_out_and_does_not_move_it() {
        let mut clock = Clock::new(50_000);
        // (arrived, Timestamp; its base)
        let cases = [
            (1_000_000, 900_000_000, Some(at(1_000_000, 0))),
            // A Timestamp a year ahead: it would be the fastest way by far.
            (1_020_000, 31_536_900_020_000_000, None),
            // One 20 seconds behind: it would be held for 20 seconds.
            (1_040_000, 880_040_000, None),
            // The largest there is.
            (1_040_000, u64::MAX, None),
            // The clock is where it was.
            (1_060_000, 900_060_000, Some(at(1_060_000, 0))),
            // Just within 10 seconds, and just beyond.
            (11_060_000, 900_060_000, Some(at(1_060_000, 10_000_000))),
            (11_060_001, 900_060_000, None),
        ];
        for (arrived, timestamp, expected) in cases {
            assert_eq!(clock.base(arrived, timestamp), expected, "{arrived}");
        }
    }

    #[test]
    fn the_clock_begins_anew_after_thirty_objects_in_a_row_that_are_not_of_it() {
        // The first Object has a Timestamp a year ahead: the clock is wrong from it.
        let mut clock = Clock::new(50_000);
        assert_eq!(
            clock.base(1_000_000, 31_536_900_000_000_000),
            Some(at(1_000_000, 0))
        );
        // 29 that are right, by a clock that is not this one, are left out.
        for number in 1..30u64 {
            let ahead = number * 40_000;
            assert_eq!(
                clock.base(1_000_000 + ahead, 900_000_000 + ahead),
                None,
                "{number}"
            );
        }
        // The thirtieth begins the clock anew, and its base is when it came.
        assert_eq!(clock.base(2_200_000, 901_200_000), Some(at(2_200_000, 0)));
        assert_eq!(
            clock.base(2_250_000, 901_240_000),
            Some(at(2_240_000, 10_000))
        );
        // Nothing of the clock before is kept: an Object by it is a stray now.
        assert_eq!(
            clock.base(2_300_000, 31_536_900_000_000_000 + 1_300_000),
            None
        );
        // One that is right in between counts from nothing again.
        let mut clock = Clock::new(0);
        assert_eq!(clock.base(1_000, 0), Some(at(1_000, 0)));
        for _ in 0..29 {
            assert_eq!(clock.base(1_000, u64::MAX), None);
        }
        assert_eq!(clock.base(2_000, 1_000), Some(at(2_000, 0)));
        for _ in 0..29 {
            assert_eq!(clock.base(1_000, u64::MAX), None);
        }
        assert_eq!(clock.base(3_000, 2_000), Some(at(3_000, 0)));
    }

    #[test]
    fn the_buffers_of_a_clock() {
        // (the buffer; for the picture, for the sound)
        let cases = [
            (50_000, 50_000, 50_000),
            (0, 0, 30_000),
            (150_000, 150_000, 150_000),
        ];
        for (buffer, picture, sound) in cases {
            let clock = Clock::new(buffer);
            assert_eq!(
                (clock.buffer_us(), clock.audio_buffer_us()),
                (picture, sound)
            );
        }
    }

    /// `fixtures/playout/least.tsv`, which `@xmoq/media` reads too: when
    /// each Object came and how long it took, in milliseconds, and the
    /// least after it (`-` where it is left out).
    #[test]
    fn the_least_is_of_the_last_minute_as_the_shared_table_says() {
        // The arrivals count from 200 s, a whole number of seconds, so that
        // an Object that took less than no time still has a Timestamp.
        const BEGAN: i64 = 200_000;
        let micros = |millis: i64| u64::try_from(millis * 1000).expect("a time from 0");
        let table = include_str!("../../../fixtures/playout/least.tsv");
        let mut rows = 0;
        for row in table.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, came, after] = fields[..] else {
                panic!("a row of 3 fields: {row}");
            };
            let mut clock = Clock::new(0);
            let got: Vec<String> = came
                .split(' ')
                .map(|each| {
                    let (at, took) = each.split_once(':').expect(each);
                    let arrived = BEGAN + at.parse::<i64>().expect(at);
                    let timestamp = micros(arrived - took.parse::<i64>().expect(took));
                    match clock.base(micros(arrived), timestamp) {
                        // The least: the base less the Timestamp.
                        Some(base) => {
                            let least = i128::from(base.at_us) - i128::from(timestamp);
                            (least / 1000).to_string()
                        }
                        None => "-".to_string(),
                    }
                })
                .collect();
            assert_eq!(got.join(" "), after, "{name}");
            rows += 1;
        }
        assert_eq!(rows, 9);
    }

    /// `fixtures/playout/auto-buffer.tsv`, which `@xmoq/media` reads too.
    #[test]
    fn the_automatic_buffer_moves_as_the_shared_table_says() {
        let table = include_str!("../../../fixtures/playout/auto-buffer.tsv");
        let number = |field: &str| field.parse::<u64>().expect(field);
        let mut rows = 0;
        for row in table.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            let [name, begins, auto, most, objects, late, ticks, after] = fields[..] else {
                panic!("a row of 8 fields: {row}");
            };
            let most_us = if most == "-" {
                AUTO_MOST_US
            } else {
                number(most) * 1000
            };
            let mut clock = match auto {
                "yes" => Clock::automatic(number(begins) * 1000, most_us),
                "no" => Clock::new(number(begins) * 1000),
                other => panic!("yes or no: {other}"),
            };
            let mut now = 1_000_000;
            for object in 0..number(objects) {
                let late_us = if object == 0 { 0 } else { number(late) * 1000 };
                now = 1_000_000 + object * 10_000 + late_us;
                clock.base(now, object * 10_000);
            }
            for _ in 0..number(ticks) {
                clock.tick(now + 1000);
            }
            assert_eq!(clock.buffer_us(), number(after) * 1000, "{name}");
            rows += 1;
        }
        assert_eq!(rows, 6);
    }

    #[test]
    fn what_came_more_than_ten_seconds_ago_is_forgotten() {
        let mut clock = Clock::automatic(50_000, AUTO_MOST_US);
        for object in 0..100u64 {
            let late = if object == 0 { 0 } else { 200_000 };
            clock.base(1_000_000 + object * 10_000 + late, object * 10_000);
        }
        clock.tick(2_200_000);
        assert_eq!((clock.lateness_us(), clock.buffer_us()), (200_000, 210_000));
        // 11 seconds later, 100 that come as fast as the first.
        for object in 0..100u64 {
            clock.base(14_000_000 + object * 10_000, 13_000_000 + object * 10_000);
        }
        clock.tick(15_000_000);
        assert_eq!((clock.lateness_us(), clock.buffer_us()), (0, 200_000));
    }

    #[test]
    fn a_clock_that_begins_anew_forgets_how_late_objects_came() {
        let mut clock = Clock::new(50_000);
        clock.base(1_000_000, 0);
        for object in 1..60u64 {
            clock.base(1_200_000 + object * 10_000, object * 10_000);
        }
        assert_eq!(clock.lateness_us(), 200_000);
        // The publisher's clock is an hour further on.
        for object in 0..29u64 {
            assert_eq!(
                clock.base(2_000_000 + object * 10_000, 3_600_000_000 + object * 10_000),
                None
            );
        }
        assert_eq!(clock.base(2_290_000, 3_600_290_000), Some(at(2_290_000, 0)));
        assert_eq!(clock.lateness_us(), 0);
    }
}
