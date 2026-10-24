//! How many lines one person may say in a second (Q43, decided on
//! 2026-10-24: 5; a line over it is dropped and counted).

use std::collections::VecDeque;

/// The span the limit is counted over, in milliseconds.
const SPAN_MS: u64 = 1_000;

/// The lines of one person that were taken in the last second.
///
/// WHY a sliding second and not a bucket that fills up again: "5 lines a
/// second" read as "no more than 5 in any second" is what a person can be
/// told, and it lets no burst of 10 through at the turn of a second, as
/// counting whole seconds of the clock would. WHY a line that is dropped
/// is not counted: a person who says too much loses what was over, not
/// what comes after.
#[derive(Debug, Clone)]
pub struct Window {
    most: usize,
    /// When each line of the last second was taken, the oldest first: no
    /// more than `most` of them.
    taken: VecDeque<u64>,
}

impl Window {
    /// A window that takes `most` lines in any second.
    #[must_use]
    pub fn new(most: usize) -> Self {
        Self {
            most,
            taken: VecDeque::with_capacity(most),
        }
    }

    /// Whether a line that comes at `now_ms` (a clock that does not go
    /// back) is taken; one that is, is counted.
    pub fn take(&mut self, now_ms: u64) -> bool {
        while self
            .taken
            .front()
            .is_some_and(|taken| now_ms.saturating_sub(*taken) >= SPAN_MS)
        {
            self.taken.pop_front();
        }
        if self.taken.len() >= self.most {
            return false;
        }
        self.taken.push_back(now_ms);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_more_than_the_limit_is_taken_in_any_second() {
        let cases: [(&str, usize, &[u64], &[bool]); 6] = [
            (
                "five at once",
                5,
                &[0, 0, 0, 0, 0],
                &[true, true, true, true, true],
            ),
            (
                "the sixth at once",
                5,
                &[0, 0, 0, 0, 0, 0],
                &[true, true, true, true, true, false],
            ),
            (
                "a second after the first",
                2,
                &[0, 500, 999, 1_000, 1_499, 1_500],
                &[true, true, false, true, false, true],
            ),
            (
                "a dropped line is not counted",
                1,
                &[0, 10, 990, 1_000],
                &[true, false, false, true],
            ),
            (
                "slow enough",
                1,
                &[0, 1_000, 2_000, 3_000],
                &[true, true, true, true],
            ),
            ("none at all", 0, &[0, 5_000], &[false, false]),
        ];
        for (case, most, times, expected) in cases {
            let mut window = Window::new(most);
            let taken: Vec<bool> = times.iter().map(|now| window.take(*now)).collect();
            assert_eq!(taken, expected, "{case}");
        }
    }
}
