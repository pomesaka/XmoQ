//! When an Object is due, from its Timestamp of LOC.
//!
//! The Timestamps are of the publisher's clock, which the viewer does not
//! have. The smallest (arrival - Timestamp) seen on any track is taken for
//! the least an Object needs on the way, and every Object is due a buffer
//! after its Timestamp plus that. Both tracks have to carry Timestamps of
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

/// The clock of one session. Times are microseconds: arrivals and what is
/// due count from a moment of the viewer's choosing, Timestamps from the
/// publisher's.
#[derive(Debug, Clone)]
pub struct Clock {
    /// The smallest (arrival - Timestamp) so far.
    least: Option<i128>,
    buffer_us: u64,
    /// Objects in a row that were not of this clock.
    strays: u32,
}

/// When an Object is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    /// The moment it is to be shown or played, on the viewer's clock.
    pub at_us: u64,
    /// How much longer than the fastest Object so far it took to come.
    pub over_least_us: u64,
}

impl Clock {
    /// A clock that holds every Object `buffer_us` beyond the fastest way.
    #[must_use]
    pub fn new(buffer_us: u64) -> Self {
        Self {
            least: None,
            buffer_us,
            strays: 0,
        }
    }

    /// Takes the arrival of an Object, and says when it is due. `None` for
    /// an Object that is not of this clock
    /// ([`MOST_FROM_THE_LEAST_US`]): it is to be left out.
    ///
    /// An Object is never due more than the buffer after it came.
    ///
    /// WHY the least is taken before the answer: the first Object is then
    /// due a buffer after it came, and not at some moment of a clock that
    /// is not known yet.
    pub fn due(&mut self, arrived_us: u64, timestamp_us: u64) -> Option<Due> {
        let took = i128::from(arrived_us) - i128::from(timestamp_us);
        if let Some(least) = self.least
            && (took - least).unsigned_abs() > u128::from(MOST_FROM_THE_LEAST_US)
        {
            self.strays += 1;
            if self.strays < ANEW_AFTER {
                return None;
            }
            self.least = None;
        }
        self.strays = 0;
        let least = self.least.map_or(took, |least| least.min(took));
        self.least = Some(least);
        let at = i128::from(timestamp_us) + least + i128::from(self.buffer_us);
        Some(Due {
            at_us: u64::try_from(at.max(0)).unwrap_or(u64::MAX),
            over_least_us: u64::try_from(took - least).unwrap_or(u64::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(at_us: u64, over_least_us: u64) -> Due {
        Due {
            at_us,
            over_least_us,
        }
    }

    #[test]
    fn objects_are_due_a_buffer_after_the_fastest_way() {
        let mut clock = Clock::new(50_000);
        // (arrived, Timestamp; due at, over the least)
        let cases = [
            // The first: its own way is the least.
            (1_000_000, 900_000_000, 1_050_000, 0),
            // 20 ms later on the publisher's clock, 30 ms later here: 10 ms slow.
            (1_030_000, 900_020_000, 1_070_000, 10_000),
            // A faster one: the least becomes 5 ms less, and it is due by that.
            (1_035_000, 900_040_000, 1_085_000, 0),
            // After it, the same pace is 5 ms over the new least.
            (1_060_000, 900_060_000, 1_105_000, 5_000),
        ];
        for (arrived, timestamp, at_us, over_least_us) in cases {
            assert_eq!(
                clock.due(arrived, timestamp),
                Some(at(at_us, over_least_us)),
                "{arrived}"
            );
        }
    }

    #[test]
    fn an_object_is_not_due_before_the_beginning_of_the_viewers_clock() {
        let mut clock = Clock::new(0);
        assert_eq!(clock.due(10, 5_000), Some(at(10, 0)));
        // Earlier on the publisher's clock than the viewer's has room for.
        assert_eq!(clock.due(20, 0), Some(at(0, 5_010)));
    }

    #[test]
    fn an_object_far_from_the_clock_is_left_out_and_does_not_move_it() {
        let mut clock = Clock::new(50_000);
        // (arrived, Timestamp; when it is due)
        let cases = [
            (1_000_000, 900_000_000, Some(at(1_050_000, 0))),
            // A Timestamp a year ahead: it would be the fastest way by far.
            (1_020_000, 31_536_900_020_000_000, None),
            // One 20 seconds behind: it would be held for 20 seconds.
            (1_040_000, 880_040_000, None),
            // The largest there is.
            (1_040_000, u64::MAX, None),
            // The clock is where it was.
            (1_060_000, 900_060_000, Some(at(1_110_000, 0))),
            // Just within 10 seconds, and just beyond.
            (11_060_000, 900_060_000, Some(at(1_110_000, 10_000_000))),
            (11_060_001, 900_060_000, None),
        ];
        for (arrived, timestamp, expected) in cases {
            assert_eq!(clock.due(arrived, timestamp), expected, "{arrived}");
        }
    }

    #[test]
    fn the_clock_begins_anew_after_thirty_objects_in_a_row_that_are_not_of_it() {
        // The first Object has a Timestamp a year ahead: the clock is wrong from it.
        let mut clock = Clock::new(50_000);
        assert_eq!(
            clock.due(1_000_000, 31_536_900_000_000_000),
            Some(at(1_050_000, 0))
        );
        // 29 that are right, by a clock that is not this one, are left out.
        for number in 1..30u64 {
            let ahead = number * 40_000;
            assert_eq!(
                clock.due(1_000_000 + ahead, 900_000_000 + ahead),
                None,
                "{number}"
            );
        }
        // The thirtieth begins the clock anew, and is due a buffer after it came.
        assert_eq!(clock.due(2_200_000, 901_200_000), Some(at(2_250_000, 0)));
        assert_eq!(
            clock.due(2_250_000, 901_240_000),
            Some(at(2_290_000, 10_000))
        );
        // One that is right in between counts from nothing again.
        let mut clock = Clock::new(0);
        assert_eq!(clock.due(1_000, 0), Some(at(1_000, 0)));
        for _ in 0..29 {
            assert_eq!(clock.due(1_000, u64::MAX), None);
        }
        assert_eq!(clock.due(2_000, 1_000), Some(at(2_000, 0)));
        for _ in 0..29 {
            assert_eq!(clock.due(1_000, u64::MAX), None);
        }
        assert_eq!(clock.due(3_000, 2_000), Some(at(3_000, 0)));
    }
}
