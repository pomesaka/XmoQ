//! How many calls of the decisions API the pet makes in a minute
//! (`--decisions-per-minute`); a batch over it is decided by the table.

use std::collections::VecDeque;

/// The span the limit is counted over.
const SPAN_MS: u64 = 60_000;

/// The calls made in the last minute.
///
/// WHY a sliding minute: "no more than n in any minute" is what costs the
/// API's owner no more than n, also at the turn of a minute of the clock.
#[derive(Debug, Clone)]
pub struct PerMinute {
    most: usize,
    /// When each call of the last minute was made, the oldest first: no more
    /// than `most` of them.
    made: VecDeque<u64>,
}

impl PerMinute {
    /// No more than `most` calls in any minute; none for 0.
    #[must_use]
    pub fn new(most: usize) -> Self {
        Self {
            most,
            made: VecDeque::with_capacity(most),
        }
    }

    /// Whether a call may be made at `now_ms` (a clock that does not go
    /// back); one that may is counted.
    pub fn take(&mut self, now_ms: u64) -> bool {
        while self
            .made
            .front()
            .is_some_and(|made| now_ms.saturating_sub(*made) >= SPAN_MS)
        {
            self.made.pop_front();
        }
        if self.made.len() >= self.most {
            return false;
        }
        self.made.push_back(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_more_than_the_limit_in_any_minute() {
        let cases: [(&str, usize, &[u64], &[bool]); 4] = [
            ("two of two", 2, &[0, 1], &[true, true]),
            (
                "a third, and a minute after the first",
                2,
                &[0, 30_000, 59_999, 60_000, 89_999, 90_000],
                &[true, true, false, true, false, true],
            ),
            ("never", 0, &[0, 120_000], &[false, false]),
            ("slow enough", 1, &[0, 60_000, 120_000], &[true, true, true]),
        ];
        for (case, most, times, expected) in cases {
            let mut rate = PerMinute::new(most);
            let got: Vec<bool> = times.iter().map(|now| rate.take(*now)).collect();
            assert_eq!(got, expected, "{case}");
        }
    }
}
