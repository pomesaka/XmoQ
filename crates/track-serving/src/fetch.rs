//! What a FETCH gets of a track whose every Group has one Object, Object 0,
//! and whose Groups are numbered from where the publisher still has them
//! to the largest without a gap (draft 18 §10.12.3, §10.13, §11.4.4.2): the
//! `lines` of a room of the chat. Every Group
//! of the range up to a limit of the publisher's.
//!
//! **A range of more Groups than the limit** gets the Groups at its end, the
//! latest, and an End of Unknown Range indicator (0x10C, §11.4.4.2) for the
//! Groups before them that are left out. What the draft has a publisher do
//! with Objects it will not send:
//!
//! - §10.12.3: "The publisher responding to a FETCH is responsible for
//!   delivering all available Objects in the requested range". It names no
//!   way to send fewer and say so but the indicators.
//! - §10.13: the End Location of `FETCH_OK` "is the End Location from the
//!   FETCH request unless the requested range extends beyond published
//!   data". WHY NOT a shorter End Location for the limit: that is not one
//!   of the cases, and a subscriber would read the range as having ended
//!   there.
//! - §10.12.3: "Any gaps in the Group and Object IDs in the response stream
//!   indicate objects that do not exist", for a stream that ends with a
//!   FIN. WHY NOT leave the older Groups out without a word: that says they
//!   were never published. WHY NOT End of Non-Existent Range (0x8C): the
//!   same. What is left is "unknown", which §10.12.3 also has a publisher
//!   use for Objects it cannot give ("indicate the range of unknown Objects
//!   and continue serving other known Objects"); a subscriber that wants
//!   them asks for the earlier range with a FETCH of its own.
//! - WHY the latest and not the first Groups of the range: a viewer who
//!   comes late reads the end of what was said first; the Rust client
//!   answers what it let go of the same way (`crates/moqt-client/src/published.rs`).
//!   The indicator stands before the Groups in Ascending order and after
//!   them in Descending order, where it names Object 0 of the first Group
//!   left out, as there.
//! - WHY NOT refuse such a FETCH: a viewer who asked for the whole history
//!   would get none of it.

use xmoq_moqt::fetch::{FetchObject, RangeEnd};
use xmoq_moqt::message::request_error_code;
use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;

/// What the answer to a FETCH carries after its header, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// An Object.
    Object(FetchObject),
    /// An End of Range indicator (§11.4.4.2).
    EndOfRange(RangeEnd, Location),
}

/// What the fetch stream of a FETCH carries: the Groups, each `(Group ID,
/// payload of its Object 0)` in the order they are sent, and the End of
/// Unknown Range indicator of [`Plan::left_out`], before or after them.
pub fn pieces_of(
    groups: impl IntoIterator<Item = (u64, Vec<u8>)>,
    left_out: Option<(Location, bool)>,
) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = groups
        .into_iter()
        .map(|(group_id, payload)| {
            Piece::Object(FetchObject {
                group_id,
                subgroup_id: Some(0),
                object_id: 0,
                // §12.4: the default, which the subgroup streams of
                // `send_group` leave to the track.
                publisher_priority: 128,
                properties: vec![],
                payload,
            })
        })
        .collect();
    match left_out {
        Some((location, true)) => pieces.insert(0, Piece::EndOfRange(RangeEnd::Unknown, location)),
        Some((location, false)) => pieces.push(Piece::EndOfRange(RangeEnd::Unknown, location)),
        None => {}
    }
    pieces
}

/// The bytes of payload the pieces of an answer hold.
#[must_use]
pub fn payload_bytes(pieces: &[Piece]) -> usize {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Object(object) => object.payload.len(),
            Piece::EndOfRange(..) => 0,
        })
        .sum()
}

/// The bytes of payload the answers to FETCH requests that are being
/// written hold together, up to a bound of the publisher's. A FETCH whose
/// answer would take them past it is refused with `EXCESSIVE_LOAD`
/// (§10.6.2: "The responder is overloaded and cannot process the request
/// at this time"), and the bytes come back as each answer is written.
#[derive(Debug)]
pub struct Held {
    most: usize,
    bytes: usize,
}

impl Held {
    /// Nothing held, and no more than `most` bytes at once.
    #[must_use]
    pub fn new(most: usize) -> Self {
        Self { most, bytes: 0 }
    }

    /// How many bytes an answer may take now.
    #[must_use]
    pub fn room(&self) -> usize {
        self.most.saturating_sub(self.bytes)
    }

    /// An answer of `bytes` is held, where it fits ([`Self::room`]).
    pub fn take(&mut self, bytes: usize) -> bool {
        if bytes > self.room() {
            return false;
        }
        self.bytes += bytes;
        true
    }

    /// An answer of `bytes` was written, or given up.
    pub fn give_back(&mut self, bytes: usize) {
        self.bytes = self.bytes.saturating_sub(bytes);
    }
}

/// What a FETCH gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The End Location of `FETCH_OK`, in the encoding of a FETCH.
    pub end_location: Location,
    /// The Groups sent, the smallest and the largest; `None` where no Group
    /// lies in the range.
    pub groups: Option<(u64, u64)>,
    /// The End of Unknown Range indicator for the Groups left out for the
    /// limit, and whether it stands before the Groups.
    pub left_out: Option<(Location, bool)>,
}

/// The last Location an End Location covers: an End Location names the last
/// Object plus 1, and with Object 0 the whole Group (§10.12.1).
fn last_of(end: Location) -> Location {
    Location {
        group: end.group,
        object: end.object.checked_sub(1).unwrap_or(u64::MAX),
    }
}

/// What a Standalone FETCH from `start` to the End Location `end` gets of a
/// track whose largest Group is `largest`, sending at most `most` Groups:
/// those the publisher has, where it keeps no more than `most`.
///
/// # Errors
///
/// `INVALID_RANGE` where the track has no Group, and where the range starts
/// after the Largest Object: §10.12.3, "If no Objects have been published
/// for the track or Start Location is greater than the Largest Object ...
/// the publisher MUST return `REQUEST_ERROR` with error code
/// `INVALID_RANGE`".
pub fn plan(
    start: Location,
    end: Location,
    largest: Option<u64>,
    most: u64,
    order: GroupOrder,
) -> Result<Plan, u64> {
    let largest = Location {
        group: largest.ok_or(request_error_code::INVALID_RANGE)?,
        object: 0,
    };
    if start > largest {
        return Err(request_error_code::INVALID_RANGE);
    }
    // §10.13: "If the requested FETCH End Location was beyond the Largest
    // known (possibly final) Object, End Location is {Largest.Group,
    // Largest.Object + 1}".
    let (last, end_location) = if last_of(end) > largest {
        let after = Location {
            group: largest.group,
            object: 1,
        };
        (largest.group, after)
    } else {
        (end.group, end)
    };
    // The first Group whose Object 0 is not before the start.
    let first = if start.object == 0 {
        Some(start.group)
    } else {
        start.group.checked_add(1)
    };
    let Some(first) = first.filter(|first| *first <= last) else {
        return Ok(Plan {
            end_location,
            groups: None,
            left_out: None,
        });
    };
    let in_range = (last - first).saturating_add(1);
    if in_range <= most {
        return Ok(Plan {
            end_location,
            groups: Some((first, last)),
            left_out: None,
        });
    }
    // At least one Group is left out: `in_range` is more than `most`, so
    // `kept_first` is above `first`.
    let kept_first = (last - most).saturating_add(1);
    let ascending = order == GroupOrder::Ascending;
    let indicator = Location {
        group: if ascending { kept_first - 1 } else { first },
        object: 0,
    };
    Ok(Plan {
        end_location,
        groups: (most > 0).then_some((kept_first, last)),
        left_out: Some((indicator, ascending)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xmoq_moqt::params::GroupOrder::{Ascending, Descending};

    const fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    fn planned(
        end_location: Location,
        groups: Option<(u64, u64)>,
        left_out: Option<(Location, bool)>,
    ) -> Plan {
        Plan {
            end_location,
            groups,
            left_out,
        }
    }

    /// (case, start, end, largest, most, order, expected)
    type Case = (
        &'static str,
        Location,
        Location,
        Option<u64>,
        u64,
        GroupOrder,
        Result<Plan, u64>,
    );

    #[test]
    fn the_answers_hold_no_more_than_the_bound_together() {
        const MOST: usize = 1000;
        // (case, in order: (true, bytes) takes an answer, (false, bytes)
        // gives one back; whether each was taken)
        type Steps = &'static [(bool, usize)];
        let cases: [(&str, Steps, &[bool]); 4] = [
            ("one of the whole", &[(true, MOST)], &[true]),
            ("one byte more", &[(true, MOST + 1)], &[false]),
            (
                "a second that does not fit",
                &[(true, MOST / 2), (true, MOST / 2 + 1)],
                &[true, false],
            ),
            (
                "room again once one is written",
                &[
                    (true, MOST / 2),
                    (true, MOST / 2),
                    (true, 1),
                    (false, MOST / 2),
                    (true, 1),
                ],
                &[true, true, false, true, true],
            ),
        ];
        for (case, steps, expected) in cases {
            let mut held = Held::new(MOST);
            let taken: Vec<bool> = steps
                .iter()
                .map(|(takes, bytes)| {
                    if *takes {
                        held.take(*bytes)
                    } else {
                        held.give_back(*bytes);
                        true
                    }
                })
                .collect();
            assert_eq!(taken, expected, "{case}");
        }
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "a table of 14 rows and one loop")]
    fn a_fetch_gets_the_groups_of_its_range_up_to_the_limit() {
        let cases: [Case; 14] = [
            (
                "a track with no Group",
                at(0, 0),
                at(5, 0),
                None,
                10,
                Ascending,
                Err(request_error_code::INVALID_RANGE),
            ),
            (
                "a start after the largest",
                at(6, 0),
                at(9, 0),
                Some(5),
                10,
                Ascending,
                Err(request_error_code::INVALID_RANGE),
            ),
            (
                "a start after Object 0 of the largest",
                at(5, 1),
                at(9, 0),
                Some(5),
                10,
                Ascending,
                Err(request_error_code::INVALID_RANGE),
            ),
            (
                "all of it",
                at(0, 0),
                at(100, 0),
                Some(5),
                10,
                Ascending,
                Ok(planned(at(5, 1), Some((0, 5)), None)),
            ),
            (
                "within the Groups, whole ones",
                at(2, 0),
                at(4, 0),
                Some(5),
                10,
                Ascending,
                Ok(planned(at(4, 0), Some((2, 4)), None)),
            ),
            (
                "an end inside a Group",
                at(2, 0),
                at(4, 1),
                Some(5),
                10,
                Ascending,
                Ok(planned(at(4, 1), Some((2, 4)), None)),
            ),
            (
                "the end at Object 1 of the largest",
                at(0, 0),
                at(5, 1),
                Some(5),
                10,
                Ascending,
                Ok(planned(at(5, 1), Some((0, 5)), None)),
            ),
            (
                "a start after Object 0 of its Group",
                at(2, 1),
                at(4, 0),
                Some(5),
                10,
                Ascending,
                Ok(planned(at(4, 0), Some((3, 4)), None)),
            ),
            (
                "no Group in the range",
                at(3, 1),
                at(3, 5),
                Some(5),
                10,
                Ascending,
                Ok(planned(at(3, 5), None, None)),
            ),
            (
                "exactly the limit",
                at(0, 0),
                at(100, 0),
                Some(9),
                10,
                Ascending,
                Ok(planned(at(9, 1), Some((0, 9)), None)),
            ),
            (
                "one over the limit",
                at(0, 0),
                at(100, 0),
                Some(10),
                10,
                Ascending,
                Ok(planned(at(10, 1), Some((1, 10)), Some((at(0, 0), true)))),
            ),
            (
                "over the limit, Ascending",
                at(2, 0),
                at(100, 0),
                Some(20),
                3,
                Ascending,
                Ok(planned(at(20, 1), Some((18, 20)), Some((at(17, 0), true)))),
            ),
            (
                "over the limit, Descending",
                at(2, 0),
                at(100, 0),
                Some(20),
                3,
                Descending,
                Ok(planned(at(20, 1), Some((18, 20)), Some((at(2, 0), false)))),
            ),
            (
                "a limit of none",
                at(0, 0),
                at(100, 0),
                Some(2),
                0,
                Ascending,
                Ok(planned(at(2, 1), None, Some((at(2, 0), true)))),
            ),
        ];
        for (case, start, end, largest, most, order, expected) in cases {
            assert_eq!(plan(start, end, largest, most, order), expected, "{case}");
        }
    }
}
