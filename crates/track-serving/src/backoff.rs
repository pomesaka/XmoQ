//! How long a client that serves tracks waits before it asks again: for a
//! session after one ended, and for its `SUBSCRIBE_NAMESPACE` after the
//! request ended while the session goes on. The gatherer of the chat waits
//! by it.
//!
//! WHY a wait that grows: a relay that ends either at once every time (one
//! that tells more namespaces than the client keeps, one that refuses it)
//! would otherwise have it ask again every second for good. WHY it comes
//! back to the first wait: after a session or a request that went well for
//! a while, the next end is one of its own and not the same one again.

use std::time::Duration;

/// The first wait. **Provisional.**
pub const FIRST: Duration = Duration::from_secs(1);

/// The longest wait. **Provisional**: long against a relay that is being
/// started again, short against a person waiting for the chat.
pub const MOST: Duration = Duration::from_secs(30);

/// How long a session or a request has to have lasted for the wait to
/// begin again from [`FIRST`]. **Provisional.**
pub const WENT_WELL: Duration = Duration::from_secs(60);

/// The wait before the next try.
#[derive(Debug, Clone)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self { next: FIRST }
    }
}

impl Backoff {
    /// The wait after something that lasted `lasted` ended: [`FIRST`] after
    /// one that lasted [`WENT_WELL`] or more, else twice the wait before, up
    /// to [`MOST`].
    pub fn after(&mut self, lasted: Duration) -> Duration {
        if lasted >= WENT_WELL {
            self.next = FIRST;
        }
        let wait = self.next;
        self.next = (wait * 2).min(MOST);
        wait
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_up_to_its_most_and_begins_again_after_a_long_run() {
        let seconds = Duration::from_secs;
        // (case, how long each lasted, the waits after each, in seconds)
        let cases: [(&str, &[u64], &[u64]); 4] = [
            ("one at once", &[0], &[1]),
            (
                "again and again at once",
                &[0, 0, 0, 0, 0, 0, 0, 0],
                &[1, 2, 4, 8, 16, 30, 30, 30],
            ),
            (
                "one that went well between",
                &[0, 0, 0, 60, 0],
                &[1, 2, 4, 1, 2],
            ),
            ("one short of going well", &[0, 0, 59], &[1, 2, 4]),
        ];
        for (case, lasted, expected) in cases {
            let mut backoff = Backoff::default();
            let waits: Vec<u64> = lasted
                .iter()
                .map(|lasted| backoff.after(seconds(*lasted)).as_secs())
                .collect();
            assert_eq!(waits, expected, "{case}");
        }
    }
}
