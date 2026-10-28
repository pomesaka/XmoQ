//! The pet's whims: a small generator of numbers that a seed fixes.
//!
//! WHY xorshift64* written here and not the `rand` crate: the pet needs no
//! secret and no quality of randomness beyond not looking regular, and its
//! tests need a seed that fixes what it does; `std` has no generator to give
//! such a seed to. The program seeds it from `RandomState`, whose keys `std`
//! makes anew for each process.

/// A generator of numbers.
#[derive(Debug, Clone)]
pub struct Chance(u64);

impl Chance {
    /// The generator of `seed`. WHY 0 is replaced: xorshift gives nothing
    /// but 0 from it.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }

    /// The next number.
    pub fn next_number(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number from `least` to `most`, both of them included; `least` where
    /// `most` is below it. The bias of the remainder is let be: a span of a
    /// few thousand out of 2^64 is not one a pet shows.
    pub fn within(&mut self, least: u64, most: u64) -> u64 {
        let span = most.saturating_sub(least).saturating_add(1);
        least.saturating_add(self.next_number() % span)
    }

    /// True once in `n` times, on the whole; never for 0.
    pub fn one_in(&mut self, n: u64) -> bool {
        n > 0 && self.next_number().is_multiple_of(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seed_fixes_the_numbers_and_they_stay_within_their_bounds() {
        let cases: [(&str, u64, u64, u64); 4] = [
            ("a span", 7, 3_000, 8_000),
            ("one number", 7, 5, 5),
            ("the seed 0", 0, 0, 1_000),
            ("most below least", 9, 10, 2),
        ];
        for (case, seed, least, most) in cases {
            let (mut one, mut other) = (Chance::new(seed), Chance::new(seed));
            for _ in 0..1_000 {
                let number = one.within(least, most);
                assert_eq!(number, other.within(least, most), "{case}");
                assert!(number >= least && number <= most.max(least), "{case}");
            }
        }
    }

    #[test]
    fn once_in_n_times() {
        let mut chance = Chance::new(1);
        let cases: [(u64, usize, usize); 3] = [(0, 0, 0), (1, 1_000, 1_000), (4, 150, 350)];
        for (n, least, most) in cases {
            let count = (0..1_000).filter(|_| chance.one_in(n)).count();
            assert!(count >= least && count <= most, "one in {n}: {count}");
        }
    }
}
