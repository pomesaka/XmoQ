//! What this client has published of its track, kept to answer a FETCH with
//! (§10.12.3), and the range a FETCH gets of it.

use std::collections::VecDeque;

use xmoq_moqt::fetch::FetchObject;
use xmoq_moqt::name::Location;
use xmoq_moqt::params::GroupOrder;
use xmoq_track_serving::fetch::{Kept, Piece, pieces_with, plan_kept};

/// Objects kept at most. Provisional.
pub const MAX_KEPT_OBJECTS: usize = 1024;

/// Bytes of payload kept at most. Provisional.
pub const MAX_KEPT_BYTES: usize = 4 * 1024 * 1024;

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
    /// The first Group that was published. Those from it to the first one
    /// kept were let go of for the limits.
    first_group: Option<u64>,
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
            }
        }
    }

    /// What this client has of its track: the first Group it published,
    /// the first it keeps, and the Largest Object's (§10.7).
    fn kept(&self) -> Option<Kept> {
        Some(Kept {
            first: self.first_group?,
            first_kept: self.kept.front()?.group_id,
            largest: self.kept.back()?.group_id,
        })
    }

    /// A publisher that published Object 0 of every Group from
    /// `first_group` to `newest_group` and keeps those from
    /// `first_kept_group` on, whatever limit made it let go of the others:
    /// the three numbers a row of `fixtures/published-fetch/` has.
    #[cfg(test)]
    fn having(first_group: u64, newest_group: u64, first_kept_group: u64) -> Self {
        let kept: VecDeque<FetchObject> = (first_kept_group..=newest_group)
            .map(|group_id| FetchObject {
                group_id,
                subgroup_id: Some(0),
                object_id: 0,
                publisher_priority: 128,
                properties: vec![],
                payload: vec![0x61, 0x62],
            })
            .collect();
        Self {
            bytes: kept.len() * 2,
            kept,
            first_group: Some(first_group),
        }
    }

    /// What a Standalone FETCH for `start` to the End Location `end` gets
    /// (§10.12.3): every Object it keeps in the range, in the Group Order
    /// that was asked for, and an End of Unknown Range indicator for those
    /// of the range it let go of, as the gatherer and the transcriber answer
    /// (`plan_kept` of `xmoq-track-serving`, where the reasons are).
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
        // No limit but what is kept: the answer holds no more than that.
        let planned = plan_kept(start, end, self.kept(), u64::MAX, group_order)?;
        let mut objects: Vec<FetchObject> = planned
            .groups
            .map(|(first, last)| {
                self.kept
                    .iter()
                    .filter(|object| (first..=last).contains(&object.group_id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if group_order == GroupOrder::Descending {
            objects.reverse();
        }
        Ok(Answer {
            end_location: planned.end_location,
            pieces: pieces_with(objects, planned.left_out),
        })
    }
}

#[cfg(test)]
mod tests {
    use xmoq_moqt::fetch::{
        FetchEvent, FetchHeader, FetchStreamDecoder, FetchStreamEncoder, RangeEnd,
    };

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

    /// `group,object`.
    fn location_of(text: &str) -> Location {
        let (group, object) = text.split_once(',').unwrap();
        at(group.parse().unwrap(), object.parse().unwrap())
    }

    /// The table `@xmoq/msf` reads as well for the publisher of a catalog:
    /// the same range gets the same answer from both.
    #[test]
    fn a_fetch_gets_what_the_shared_table_says() {
        let table = include_str!("../../../fixtures/published-fetch/answers.tsv");
        let mut rows = 0;
        // Every row that differs, so that one run names them all.
        let mut differing = Vec::new();
        for row in table.lines() {
            let columns: Vec<&str> = row.split('\t').collect();
            let [
                name,
                first,
                newest,
                first_kept,
                start,
                end,
                order,
                answer,
                end_location,
                pieces,
            ] = columns[..]
            else {
                panic!("a row of 10 columns: {row}");
            };
            let group = |text: &str| text.parse::<u64>().unwrap();
            let published = Published::having(group(first), group(newest), group(first_kept));
            let order = match order {
                "ascending" => GroupOrder::Ascending,
                "descending" => GroupOrder::Descending,
                other => panic!("{name}: the order {other}"),
            };
            let got = match published.answer(location_of(start), location_of(end), order) {
                // INVALID_RANGE (0x11).
                Err(0x11) => ("invalid_range", "-".to_string(), "-".to_string()),
                Err(code) => panic!("{name}: refused with {code:#x}"),
                Ok(answer) => {
                    let at = answer.end_location;
                    let shown = shown(&answer, order);
                    let pieces = if shown.is_empty() {
                        "-".to_string()
                    } else {
                        shown.join(" ")
                    };
                    ("ok", format!("{},{}", at.group, at.object), pieces)
                }
            };
            if got != (answer, end_location.to_string(), pieces.to_string()) {
                differing.push(format!("{name}: {got:?}"));
            }
            rows += 1;
        }
        assert_eq!(differing, Vec::<String>::new());
        // A table that lost rows would pass.
        assert_eq!(rows, 47);
    }

    #[test]
    fn nothing_published_has_no_range() {
        // INVALID_RANGE (0x11).
        assert_eq!(
            Published::default().answer(at(0, 0), at(1, 0), GroupOrder::Ascending),
            Err(0x11)
        );
    }

    #[test]
    fn what_is_over_the_limits_is_let_go_of_oldest_first() {
        assert_eq!((MAX_KEPT_OBJECTS, MAX_KEPT_BYTES), (1024, 4 * 1024 * 1024));
        let shown_of = |published: &Published, end| {
            let answer = published
                .answer(at(0, 0), end, GroupOrder::Ascending)
                .unwrap();
            shown(&answer, GroupOrder::Ascending)
        };
        // Groups 0 to 5, 1 MiB each: the two oldest are let go of for the 4
        // MiB, and the Largest Object stays what it is.
        let mut published = Published::default();
        for group in 0..=5 {
            published.push(object(group, 1024 * 1024));
        }
        assert_eq!(published.kept().map(|kept| kept.largest), Some(5));
        assert_eq!(shown_of(&published, at(4, 0)), ["u1,0", "2", "3", "4"]);

        // One Object over the limit alone is kept: it is the Largest Object.
        let mut published = Published::default();
        published.push(object(0, 5 * 1024 * 1024));
        assert_eq!(published.kept().map(|kept| kept.largest), Some(0));
        assert_eq!(shown_of(&published, at(0, 1)), ["0"]);

        // And 1024 Objects at most.
        let mut published = Published::default();
        for group in 0..1030 {
            published.push(object(group, 1));
        }
        assert_eq!(shown_of(&published, at(8, 0)), ["u5,0", "6", "7", "8"]);
    }
}
