//! What this client has published of its track, kept to answer a FETCH with
//! (§10.12.3), and the range a FETCH gets of it.

use std::collections::VecDeque;

use xmoq_moqt::fetch::{FetchObject, RangeEnd};
use xmoq_moqt::message::request_error_code;
use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;

/// Objects kept at most. Provisional.
pub const MAX_KEPT_OBJECTS: usize = 1024;

/// Bytes of payload kept at most. Provisional.
pub const MAX_KEPT_BYTES: usize = 4 * 1024 * 1024;

/// What a fetch stream carries after its header, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Object(FetchObject),
    /// An End of Range indicator (§11.4.4.2) with this Location.
    EndOfRange(RangeEnd, Location),
}

/// The answer to a FETCH that is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The End Location of `FETCH_OK`.
    pub end_location: Location,
    pub pieces: Vec<Piece>,
}

/// The Objects this client published, oldest first, within
/// [`MAX_KEPT_OBJECTS`] and [`MAX_KEPT_BYTES`].
///
/// The client publishes Object 0 of one Group after the other, without a
/// gap; [`Self::answer`] counts on both.
#[derive(Debug, Default)]
pub struct Published {
    kept: VecDeque<FetchObject>,
    bytes: usize,
    /// The first Group that was published.
    first_group: Option<u64>,
    /// The largest Location of an Object that was let go of for the limits.
    /// The Objects from the first Group to it were published and are not
    /// here any more.
    let_go_through: Option<Location>,
}

fn location(object: &FetchObject) -> Location {
    Location {
        group: object.group_id,
        object: object.object_id,
    }
}

/// The last Location an End Location covers: an End Location names the last
/// Object plus 1, and with Object 0 the whole Group (§10.12.1).
fn last_of(end: Location) -> Location {
    Location {
        group: end.group,
        object: end.object.checked_sub(1).unwrap_or(u64::MAX),
    }
}

impl Published {
    /// Keeps an Object that was sent. The caller publishes in the order of
    /// the Locations, so the newest is the largest.
    pub fn push(&mut self, object: FetchObject) {
        self.bytes = self.bytes.saturating_add(object.payload.len());
        self.first_group.get_or_insert(object.group_id);
        self.kept.push_back(object);
        while self.kept.len() > MAX_KEPT_OBJECTS || self.bytes > MAX_KEPT_BYTES {
            // The newest stays, however large: it is the Largest Object.
            if self.kept.len() == 1 {
                break;
            }
            if let Some(gone) = self.kept.pop_front() {
                self.bytes = self.bytes.saturating_sub(gone.payload.len());
                self.let_go_through = Some(location(&gone));
            }
        }
    }

    /// The Largest Object (§10.7).
    fn largest(&self) -> Option<Location> {
        self.kept.back().map(location)
    }

    /// What a Standalone FETCH for `start` to the End Location `end` gets
    /// (§10.12.3).
    ///
    /// The Objects that exist in the range, in the Group Order that was
    /// asked for; what is missing between them does not exist, which a
    /// fetch stream says by leaving it out ("Any gaps in the Group and
    /// Object IDs in the response stream indicate objects that do not
    /// exist"). WHY an End of Unknown Range indicator for what was let go
    /// of: those Objects were published, so leaving them out would say
    /// something false, and so would End of Non-Existent Range; what this
    /// client can still say of them is that it does not know (§11.4.4.2).
    ///
    /// The indicator names Object 0 of a Group that was let go of: the last
    /// such Group of the range in Ascending order, where it stands before
    /// the Objects, and the first in Descending order, where it follows
    /// them. WHERE it stands in Descending order the draft does not say: it
    /// has an indicator cover what lies "between the last serialized
    /// Object, if any, and this Location". None is written where no Object
    /// that was let go of lies in the range.
    ///
    /// WHAT the Ascending indicator covers that was never published: it has
    /// no near end, so the Groups of the range before the first one this
    /// client published read as unknown too. That is not false, and to say
    /// more would take a second indicator before it, whose far end the
    /// first would cover again.
    ///
    /// # Errors
    ///
    /// `INVALID_RANGE` when nothing was published, and when the range
    /// starts after the Largest Object.
    pub fn answer(
        &self,
        start: Location,
        end: Location,
        group_order: GroupOrder,
    ) -> Result<Answer, u64> {
        let Some(largest) = self.largest() else {
            return Err(request_error_code::INVALID_RANGE);
        };
        if start > largest {
            return Err(request_error_code::INVALID_RANGE);
        }
        // §10.13: the End Location that was asked for, unless it is beyond
        // the Largest Object.
        let (last, end_location) = if last_of(end) > largest {
            let after = Location {
                group: largest.group,
                // This client's Objects are Object 0, which has one after
                // it.
                object: largest.object.saturating_add(1),
            };
            (largest, after)
        } else {
            (last_of(end), end)
        };
        let mut objects: Vec<Piece> = self
            .kept
            .iter()
            .filter(|object| (start..=last).contains(&location(object)))
            .cloned()
            .map(Piece::Object)
            .collect();
        let unknown = |group| Piece::EndOfRange(RangeEnd::Unknown, Location { group, object: 0 });
        let pieces = match (group_order, self.let_go_in(start, last)) {
            (GroupOrder::Ascending, None) => objects,
            (GroupOrder::Ascending, Some((_, to))) => {
                let mut pieces = vec![unknown(to)];
                pieces.append(&mut objects);
                pieces
            }
            // One Object a Group, so the order of the Groups is the order of
            // the Objects.
            (GroupOrder::Descending, let_go) => {
                objects.reverse();
                objects.extend(let_go.map(|(from, _)| unknown(from)));
                objects
            }
        };
        Ok(Answer {
            end_location,
            pieces,
        })
    }

    /// The first and the last Group whose Object was let go of and lies in
    /// the range from `start` to `last`.
    fn let_go_in(&self, start: Location, last: Location) -> Option<(u64, u64)> {
        let gone = self.let_go_through?;
        // The first Group whose Object 0 is not before `start`.
        let from_start = if start.object == 0 {
            start.group
        } else {
            start.group.checked_add(1)?
        };
        let from = from_start.max(self.first_group?);
        let to = gone.group.min(last.group);
        (from <= to).then_some((from, to))
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::fetch::{FetchEvent, FetchHeader, FetchStreamDecoder, FetchStreamEncoder};

    use super::*;

    fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    fn object(group_id: u64, payload_len: usize) -> FetchObject {
        FetchObject {
            group_id,
            subgroup_id: Some(0),
            object_id: 0,
            publisher_priority: 128,
            properties: vec![],
            payload: vec![0x61; payload_len],
        }
    }

    /// The Groups 3 to 7, one Object of 2 bytes each.
    fn groups_3_to_7() -> Published {
        let mut published = Published::default();
        for group in 3..=7 {
            published.push(object(group, 2));
        }
        published
    }

    /// The Groups of the Objects of an answer, and `u` for an End of Unknown
    /// Range indicator with its Location.
    /// What `answer` has, read back from the bytes of the fetch stream that
    /// carries it in `order`: the Group of an Object, and `u` or `n` with
    /// the Location of an indicator.
    ///
    /// WHY through the bytes: the encoder refuses what a fetch stream cannot
    /// say (an Object out of the order of the stream), and an answer it
    /// refuses would only be found by a subscriber.
    fn shown(answer: &Answer, order: GroupOrder) -> Vec<String> {
        let header = FetchHeader { request_id: 0 };
        let mut encoder = FetchStreamEncoder::new(header, order);
        for piece in &answer.pieces {
            match piece {
                Piece::Object(object) => encoder.object(object).unwrap(),
                Piece::EndOfRange(kind, location) => encoder.end_of_range(*kind, *location),
            }
        }
        let mut decoder = FetchStreamDecoder::new(8 * 1024 * 1024, order);
        decoder.push(&encoder.take_bytes()).unwrap();
        let mut shown = Vec::new();
        while let Some(event) = decoder.next_event().unwrap() {
            match event {
                FetchEvent::Header(read) => assert_eq!(read, header),
                FetchEvent::Object(object) => shown.push(object.group_id.to_string()),
                FetchEvent::EndOfRange { kind, location } => {
                    let kind = match kind {
                        RangeEnd::Unknown => "u",
                        RangeEnd::NonExistent => "n",
                    };
                    shown.push(format!("{kind}{},{}", location.group, location.object));
                }
            }
        }
        decoder.finish().unwrap();
        shown
    }

    #[test]
    fn a_fetch_gets_the_objects_of_its_range_and_an_end_location() {
        let ascending = GroupOrder::Ascending;
        let descending = GroupOrder::Descending;
        // (Start Location; End Location; Group Order; the End Location of
        //  FETCH_OK; what the stream carries)
        let cases: [(_, _, _, _, &[&str]); 11] = [
            // Whole Groups: an End Location with Object 0.
            (at(3, 0), at(5, 0), ascending, at(5, 0), &["3", "4", "5"]),
            (at(3, 0), at(5, 0), descending, at(5, 0), &["5", "4", "3"]),
            // To an Object: the last one is the Object before.
            (at(3, 0), at(5, 1), ascending, at(5, 1), &["3", "4", "5"]),
            (at(4, 0), at(4, 1), ascending, at(4, 1), &["4"]),
            // Object 1 of a Group does not exist, and is left out.
            (at(3, 1), at(5, 0), ascending, at(5, 0), &["4", "5"]),
            // The Groups before the first do not exist either: no indicator.
            (at(0, 0), at(4, 0), ascending, at(4, 0), &["3", "4"]),
            // Beyond the Largest Object {7, 0}: the End Location is {7, 1}.
            (at(6, 0), at(9, 0), ascending, at(7, 1), &["6", "7"]),
            (at(6, 0), at(9, 4), descending, at(7, 1), &["7", "6"]),
            // Nothing exists in the range: a stream without Objects.
            (at(3, 1), at(3, 5), ascending, at(3, 5), &[]),
            // The whole of the last Group: beyond the Largest Object too.
            (at(6, 0), at(7, 0), ascending, at(7, 1), &["6", "7"]),
            // A range of one Location that is none, which the session
            // takes: nothing.
            (at(4, 3), at(4, 3), ascending, at(4, 3), &[]),
        ];
        for (start, end, order, end_location, pieces) in cases {
            let answer = groups_3_to_7().answer(start, end, order).unwrap();
            assert_eq!(
                (answer.end_location, shown(&answer, order)),
                (
                    end_location,
                    pieces.iter().map(ToString::to_string).collect()
                ),
                "{start:?} {end:?} {order:?}"
            );
        }
    }

    #[test]
    fn a_range_that_has_nothing_to_start_from_is_invalid() {
        // Nothing was published: INVALID_RANGE (0x11).
        assert_eq!(
            Published::default().answer(at(0, 0), at(1, 0), GroupOrder::Ascending),
            Err(0x11)
        );
        // After the Largest Object {7, 0}.
        for start in [at(7, 1), at(8, 0)] {
            assert_eq!(
                groups_3_to_7().answer(start, at(9, 0), GroupOrder::Ascending),
                Err(0x11),
                "{start:?}"
            );
        }
        // The Largest Object itself is a range.
        let answer = groups_3_to_7()
            .answer(at(7, 0), at(9, 0), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(shown(&answer, GroupOrder::Ascending), ["7"]);
    }

    #[test]
    fn what_was_let_go_of_is_said_to_be_unknown() {
        assert_eq!((MAX_KEPT_OBJECTS, MAX_KEPT_BYTES), (1024, 4 * 1024 * 1024));
        // Groups 0 to 5, 1 MiB each: the two oldest are let go of for the 4
        // MiB, and the Largest Object stays what it is.
        let mut published = Published::default();
        for group in 0..=5 {
            published.push(object(group, 1024 * 1024));
        }
        assert_eq!(published.largest(), Some(at(5, 0)));
        // (Start Location; End Location; Group Order; what the stream
        //  carries)
        let (ascending, descending) = (GroupOrder::Ascending, GroupOrder::Descending);
        let cases: [(_, _, _, &[&str]); 11] = [
            (at(0, 0), at(4, 0), ascending, &["u1,0", "2", "3", "4"]),
            // From the last Group that was let go of.
            (at(1, 0), at(4, 0), ascending, &["u1,0", "2", "3", "4"]),
            // The indicator ends where the range does, at Object 0 of a
            // Group also where the whole Group was asked for.
            (at(0, 0), at(0, 1), ascending, &["u0,0"]),
            (at(0, 0), at(0, 0), ascending, &["u0,0"]),
            (at(0, 0), at(1, 0), ascending, &["u1,0"]),
            // Nothing of the range was let go of.
            (at(2, 0), at(4, 0), ascending, &["2", "3", "4"]),
            (at(1, 1), at(4, 0), ascending, &["2", "3", "4"]),
            // No Object that was let go of lies in it: Object 3 of Group 1
            // never was.
            (at(1, 3), at(1, 3), ascending, &[]),
            (at(0, 1), at(0, 0), ascending, &[]),
            // Descending: after the Objects, at the first Group that was
            // let go of and is in the range.
            (at(0, 0), at(3, 0), descending, &["3", "2", "u0,0"]),
            (at(0, 1), at(3, 0), descending, &["3", "2", "u1,0"]),
        ];
        for (start, end, order, pieces) in cases {
            let answer = published.answer(start, end, order).unwrap();
            let expected: Vec<String> = pieces.iter().map(ToString::to_string).collect();
            assert_eq!(
                shown(&answer, order),
                expected,
                "{start:?} {end:?} {order:?}"
            );
        }
        // One Object over the limit alone is kept: it is the Largest Object.
        let mut published = Published::default();
        published.push(object(0, 5 * 1024 * 1024));
        assert_eq!(published.largest(), Some(at(0, 0)));

        // And 1024 Objects at most.
        let mut published = Published::default();
        for group in 0..1030 {
            published.push(object(group, 1));
        }
        let answer = published
            .answer(at(0, 0), at(8, 0), GroupOrder::Ascending)
            .unwrap();
        assert_eq!(
            shown(&answer, GroupOrder::Ascending),
            ["u5,0", "6", "7", "8"]
        );

        // A client that began at Group 10: in Descending order the
        // indicator does not go below what it published.
        let mut published = Published::default();
        for group in 10..=15 {
            published.push(object(group, 1024 * 1024));
        }
        let answer = |order| {
            let answer = published.answer(at(5, 0), at(13, 0), order).unwrap();
            shown(&answer, order)
        };
        assert_eq!(answer(GroupOrder::Ascending), ["u11,0", "12", "13"]);
        assert_eq!(answer(GroupOrder::Descending), ["13", "12", "u10,0"]);
    }
}
