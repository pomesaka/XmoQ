//! When an Object is due, from its Timestamp of LOC.
//!
//! The Timestamps are of the publisher's clock, which the viewer does not
//! have. The smallest (arrival - Timestamp) seen on any track is taken for
//! the least an Object needs on the way, and every Object is due a buffer
//! after its Timestamp plus that. Both tracks have to carry Timestamps of
//! one clock for their Objects to be due at matching moments.

/// The clock of one session. Times are microseconds: arrivals and what is
/// due count from a moment of the viewer's choosing, Timestamps from the
/// publisher's.
#[derive(Debug, Clone)]
pub struct Clock {
    /// The smallest (arrival - Timestamp) so far.
    least: Option<i128>,
    buffer_us: u64,
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
        }
    }

    /// Takes the arrival of an Object, and says when it is due.
    ///
    /// WHY the least is taken before the answer: the first Object is then
    /// due a buffer after it came, and not at some moment of a clock that
    /// is not known yet.
    pub fn due(&mut self, arrived_us: u64, timestamp_us: u64) -> Due {
        let took = i128::from(arrived_us) - i128::from(timestamp_us);
        let least = self.least.map_or(took, |least| least.min(took));
        self.least = Some(least);
        let at = i128::from(timestamp_us) + least + i128::from(self.buffer_us);
        Due {
            at_us: u64::try_from(at.max(0)).unwrap_or(u64::MAX),
            over_least_us: u64::try_from(took - least).unwrap_or(u64::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                Due {
                    at_us,
                    over_least_us
                },
                "{arrived}"
            );
        }
    }

    #[test]
    fn a_timestamp_far_in_the_future_is_due_at_once_and_not_before_the_beginning() {
        let mut clock = Clock::new(0);
        assert_eq!(
            clock.due(10, 5_000),
            Due {
                at_us: 10,
                over_least_us: 0
            }
        );
        // Earlier on the publisher's clock than the viewer's has room for.
        assert_eq!(
            clock.due(20, 0),
            Due {
                at_us: 0,
                over_least_us: 5_010
            }
        );
    }
}
