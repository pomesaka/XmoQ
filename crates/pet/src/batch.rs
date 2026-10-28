//! What the pet perceives of a room's garden, gathered into one batch, which
//! is decided at once: from the first happening, for [`BATCH_MS`], and
//! while a decision of the room is being asked for, until it is answered.
//!
//! WHY a batch and not a decision for each happening: a call of the API
//! takes a while, and people in a room do several things in a second; the
//! pet reacts once, to what they did together, as a dog would.

use crate::perceive::Happening;

/// How long the happenings of a room are gathered after the first one is
/// perceived. **Provisional.**
pub const BATCH_MS: u64 = 1_000;

/// The most happenings a batch holds; one more is dropped. WHY a limit:
/// what happens in a garden is whatever its actors do, and it is held until
/// an answer comes. **Provisional.**
pub const BATCH_MOST: usize = 32;

/// The batch under way of a room.
#[derive(Debug, Clone, Default)]
pub struct Batch {
    happenings: Vec<Happening>,
    due_at: Option<u64>,
    asking: bool,
}

/// The batch holds [`BATCH_MOST`] happenings: the one perceived is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

impl Batch {
    /// A happening perceived at `now`.
    ///
    /// # Errors
    ///
    /// [`Full`].
    pub fn add(&mut self, happening: Happening, now: u64) -> Result<(), Full> {
        if self.happenings.len() >= BATCH_MOST {
            return Err(Full);
        }
        self.happenings.push(happening);
        self.due_at.get_or_insert(now.saturating_add(BATCH_MS));
        Ok(())
    }

    /// When the batch is to be decided; `None` while there is none, and
    /// while a decision is being asked for.
    #[must_use]
    pub fn due_at(&self) -> Option<u64> {
        self.due_at.filter(|_| !self.asking)
    }

    /// The batch, where it is due at `now`; a new one begins.
    pub fn take(&mut self, now: u64) -> Option<Vec<Happening>> {
        if self.due_at()? > now {
            return None;
        }
        self.due_at = None;
        Some(std::mem::take(&mut self.happenings))
    }

    /// A decision is being asked for: no batch is due until it is
    /// [`Self::answered`].
    pub fn asking(&mut self) {
        self.asking = true;
    }

    /// The decision that was asked for came, or failed.
    pub fn answered(&mut self) {
        self.asking = false;
    }
}

#[cfg(test)]
mod tests {
    use xmoq_pet_tracks::payloads::{Event, EventKind};

    use super::*;

    fn acted(name: &str) -> Happening {
        Happening::Done(Event {
            id: 1,
            kind: EventKind::Pet,
            by: name.to_string(),
            text: None,
            at: 0,
        })
    }

    enum Step {
        Add(&'static str, u64, bool),
        DueAt(Option<u64>),
        Take(u64, Option<&'static [&'static str]>),
        Asking,
        Answered,
    }
    use Step::{Add, Answered, Asking, DueAt, Take};

    #[test]
    fn happenings_are_gathered_for_a_while_and_wait_for_the_decision_under_way() {
        let cases: [(&str, Vec<Step>); 3] = [
            (
                "a second from the first",
                vec![
                    DueAt(None),
                    Add("a", 100, true),
                    Add("b", 900, true),
                    DueAt(Some(1_100)),
                    Take(1_099, None),
                    Take(1_100, Some(&["a", "b"])),
                    DueAt(None),
                    Take(5_000, None),
                ],
            ),
            (
                "while a decision is asked for",
                vec![
                    Add("a", 0, true),
                    Take(1_000, Some(&["a"])),
                    Asking,
                    Add("b", 1_200, true),
                    DueAt(None),
                    Take(9_000, None),
                    Answered,
                    DueAt(Some(2_200)),
                    Take(9_000, Some(&["b"])),
                ],
            ),
            ("nothing", vec![Take(0, None)]),
        ];
        for (case, steps) in cases {
            let mut batch = Batch::default();
            for (index, step) in steps.into_iter().enumerate() {
                match step {
                    Add(name, now, added) => {
                        assert_eq!(
                            batch.add(acted(name), now).is_ok(),
                            added,
                            "{case}, {index}"
                        );
                    }
                    DueAt(at) => assert_eq!(batch.due_at(), at, "{case}, {index}"),
                    Take(now, names) => {
                        let got = batch.take(now).map(|taken| {
                            taken
                                .into_iter()
                                .map(|happening| happening.by().to_string())
                                .collect::<Vec<_>>()
                        });
                        let expected =
                            names.map(|names| names.iter().map(ToString::to_string).collect());
                        assert_eq!(got, expected, "{case}, {index}");
                    }
                    Asking => batch.asking(),
                    Answered => batch.answered(),
                }
            }
        }
    }

    #[test]
    fn a_batch_holds_no_more_than_the_most() {
        let mut batch = Batch::default();
        for index in 0..BATCH_MOST {
            assert_eq!(batch.add(acted(&index.to_string()), 0), Ok(()));
        }
        assert_eq!(batch.add(acted("one more"), 0), Err(Full));
        assert_eq!(
            batch.take(BATCH_MS).map(|taken| taken.len()),
            Some(BATCH_MOST)
        );
        assert_eq!(batch.add(acted("after"), BATCH_MS), Ok(()));
    }
}
