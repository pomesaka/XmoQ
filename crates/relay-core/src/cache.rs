//! The cache of a track (§9.1): the Objects of its latest Group, to answer a
//! FETCH with that lies wholly in it.
//!
//! WHY the latest Group and no more: it is what a Joining FETCH with offset
//! 0 asks for, the way a subscriber gets a catalog and a viewer that comes
//! late its first picture. Everything else is asked of a publisher, as
//! before.
//!
//! What goes in is what the relay passes on to subscribers: an Object of a
//! stream it took the Subgroup from, whether or not anybody is subscribed.
//! Which of several sources a Subgroup is taken from is therefore decided
//! where it was (`Subgroups`), and not here.
//!
//! An Object that goes in is compared with what is kept of its Group, and
//! can show the track to be malformed ([`Cache::insert`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use xmoq_moqt::data::{ObjectStatus, SubgroupObject};
use xmoq_moqt::kvp::{KeyValuePair, Value};
use xmoq_moqt::name::Location;

/// Bytes of payload and of Object Properties cached for one track at most.
///
/// Provisional. A Group of video is a key frame and what depends on it: two
/// seconds at 20 Mbit/s are 5 MB.
pub(crate) const MAX_CACHED_BYTES_OF_A_TRACK: usize = 8 * 1024 * 1024;

/// Objects cached for one track at most. Provisional: more than two minutes
/// of 30 frames a second with an Object a frame.
pub(crate) const MAX_CACHED_OBJECTS_OF_A_TRACK: usize = 4_096;

/// Bytes cached for all tracks together at most. Provisional.
///
/// WHY nothing is evicted for it: which track to take from would be one
/// more thing to decide, and a track that does not fit is served as every
/// track was before there was a cache.
pub(crate) const MAX_CACHED_BYTES: usize = 256 * 1024 * 1024;

/// Prior Object ID Gap (§12.9).
const PRIOR_OBJECT_ID_GAP: u64 = 0x3e;

/// `MAX_CACHE_DURATION` (§12.3), a Track Property.
const MAX_CACHE_DURATION: u64 = 0x04;

/// Immutable Properties (§12.7): Track Properties in a property.
const IMMUTABLE_PROPERTIES: u64 = 0x0b;

/// Whether the Objects of a subscription whose publisher gave these Track
/// Properties may be cached.
///
/// WHY not with `MAX_CACHE_DURATION`: "If present, the relay MUST NOT start
/// forwarding any individual Object received through this subscription or
/// fetch after the specified number of milliseconds has elapsed since the
/// beginning of the Object was received" (§12.3). An Object that is not kept
/// is not forwarded late. WHY NOT keep it for that long: the relay core
/// knows no time. Provisional.
///
/// WHY not with Immutable Properties either: "When looking for the value of
/// a property, processors MUST search both the mutable properties and the
/// contents of Immutable Properties" (§12.7), so a `MAX_CACHE_DURATION` can
/// be in there, and the relay does not read what Immutable Properties
/// contain. Provisional as well: it keeps a publisher that uses them for
/// anything from being cached.
pub(crate) fn may_be_cached(track_properties: &[KeyValuePair]) -> bool {
    !track_properties
        .iter()
        .any(|property| matches!(property.ty(), MAX_CACHE_DURATION | IMMUTABLE_PROPERTIES))
}

/// An Object as the cache keeps it, with what a fetch stream says of it
/// beside what its subgroup stream said (§9.1: "A cache MUST store all
/// fields of an Object defined in Section 11.2.1").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedObject {
    /// Group ID.
    pub group_id: u64,
    /// Subgroup ID.
    pub subgroup_id: u64,
    /// Publisher Priority: the one of its Subgroup.
    pub publisher_priority: u8,
    /// The Object, shared with the subgroup streams it went out on.
    pub object: Arc<SubgroupObject>,
}

impl CachedObject {
    /// What it counts against the limits in bytes.
    fn size(&self) -> usize {
        let properties: usize = self
            .object
            .properties
            .iter()
            .map(|property| match property.value() {
                Value::Varint(_) => 8,
                Value::Bytes(bytes) => bytes.len(),
            })
            .sum();
        self.object.payload.len().saturating_add(properties)
    }

    /// The Object IDs before this one that it says do not exist (§12.9:
    /// "the number of Objects prior to the current Object that do not and
    /// will never exist").
    fn prior_gap(&self) -> u64 {
        self.object
            .properties
            .iter()
            .find_map(|property| match property.value() {
                Value::Varint(gap) if property.ty() == PRIOR_OBJECT_ID_GAP => Some(*gap),
                _ => None,
            })
            .unwrap_or(0)
    }
}

/// What an Object that was to be cached shows of its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Fine,
    /// The track is malformed (§2.4.2), by this Object.
    Malformed,
}

/// The Group a cache keeps.
#[derive(Debug)]
struct Group {
    id: u64,
    /// By Object ID. Empty once the Group is given up.
    objects: BTreeMap<u64, CachedObject>,
    bytes: usize,
    /// The Object IDs of the Objects whose Subgroup ID the relay is not
    /// sure of (see [`Cache::insert`]).
    unsure_of_subgroup: BTreeSet<u64>,
    /// The Group does not fit the limits, or has something the cache does
    /// not answer for: no FETCH is answered from it. The next Group starts
    /// anew.
    given_up: bool,
}

impl Group {
    fn give_up(&mut self, in_use: &mut usize) {
        *in_use = in_use.saturating_sub(self.bytes);
        self.objects.clear();
        self.unsure_of_subgroup.clear();
        self.bytes = 0;
        self.given_up = true;
    }
}

/// The cache of one track.
///
/// Behind a pointer, so that a track that keeps nothing is no larger for
/// it.
#[derive(Debug, Default)]
pub(crate) struct Cache(Option<Box<Group>>);

impl Cache {
    /// Takes an Object the relay passes on, and says whether it shows the
    /// track to be malformed. `in_use` is the number of bytes all caches of
    /// the relay hold, which this keeps.
    ///
    /// An Object of a Group before the one that is kept is not taken; one of
    /// a later Group replaces what is kept.
    ///
    /// **Against what is kept of its Group, the Object is one of these:**
    ///
    /// * The Object that is there under its Object ID, again. With another
    ///   Subgroup ID, Publisher Priority or payload the track is malformed
    ///   (§9.1: "An endpoint that receives a duplicate Object with a
    ///   different Forwarding Preference, Subgroup ID, Priority or Payload
    ///   MUST treat the track as Malformed"). With other Object Properties
    ///   it is not ("Object Properties can be added, removed or updated"),
    ///   and the one that came first stays ("Relays MAY ignore subsequently
    ///   received Objects").
    /// * An Object after the End of Group (§2.4.2: "An Object is received in
    ///   a Group whose Object ID is larger than the final Object in the
    ///   Group"), an Object, and not a status, inside a gap that a later
    ///   Object announced, or
    ///   one whose own gap covers an Object that came (§12.9: "an Object ID
    ///   within a previously communicated gap", "a Prior Object ID Gap
    ///   covering an Object it previously received"): malformed.
    /// * A status where an Object is, or an End of Group below Objects that
    ///   came: not malformed, since an Object "can transition from existing
    ///   to not existing" (§9.1, §2.1) and the publisher may have taken it
    ///   back. The cache does not say which of the two it is: the Group is
    ///   given up, and answers no FETCH.
    /// * An Object at the very Object ID of an End of Group: "an endpoint
    ///   can receive an Object after it has already recorded that the Object
    ///   does not exist ... This is not a protocol error and the Track is
    ///   not malformed" (§2.1), and "A caching relay SHOULD NOT cache or
    ///   forward the Object in this case" (§9.1). It is not kept, and the
    ///   Group is given up.
    ///
    /// `sure_of_subgroup` is false for an Object of a stream that leaves
    /// its Subgroup ID to its first Object and does not say that this
    /// Object is the first of the Subgroup: "the Subgroup ID is the Object
    /// ID of the first Object transmitted in this Subgroup" (§11.4.2), not
    /// in this stream, and `FIRST_OBJECT` says whether "the first object in
    /// this subgroup stream is the first object published in the
    /// subgroup". A Subgroup sent again from its middle on such a stream
    /// has the Subgroup ID it had, which the relay cannot read off it. Such
    /// a Subgroup ID is not held against the track, and a Group that has
    /// one answers no FETCH, since a fetch stream would have to state it.
    ///
    /// WHY the gap is malformed where the End of Group below Objects is
    /// not, though both say of an Object that came that it does not exist:
    /// §12.9 names the one, and nothing names the other.
    ///
    /// WHY only against what is kept: an earlier Group, a Group that was
    /// given up and a track that is not cached have nothing to compare
    /// with. An Object the track is malformed for is not kept (§2.4.2:
    /// "Object(s) triggering Malformed Track status MUST NOT be cached").
    ///
    /// WHY a Group that does not fit is given up as a whole, and Objects
    /// are not dropped from its start: a Group that lacks its first Objects
    /// answers no FETCH (see [`Self::answer`]), so keeping the rest of it
    /// would hold memory for nothing.
    ///
    /// An End of Track is kept as an End of Group is: no Object is from it on,
    /// in its Group or after (§11.2.1.1). An Object past it is not seen here:
    /// the track found it malformed before (`Track::past_its_end`).
    pub(crate) fn insert(
        &mut self,
        cached: CachedObject,
        sure_of_subgroup: bool,
        in_use: &mut usize,
    ) -> Verdict {
        let Some(group) = self.group_of(cached.group_id, in_use) else {
            return Verdict::Fine;
        };
        if group.given_up {
            return Verdict::Fine;
        }
        let object_id = cached.object.object_id;
        let status = cached.object.status;
        // Its own gap covers an Object that came.
        let gap_from = object_id.saturating_sub(cached.prior_gap());
        if group.objects.range(gap_from..object_id).next().is_some() {
            return Verdict::Malformed;
        }
        if let Some(there) = group.objects.get(&object_id) {
            let both_objects =
                status == ObjectStatus::Normal && there.object.status == ObjectStatus::Normal;
            if both_objects {
                let subgroups_compare =
                    sure_of_subgroup && !group.unsure_of_subgroup.contains(&object_id);
                let same = (!subgroups_compare || there.subgroup_id == cached.subgroup_id)
                    && there.publisher_priority == cached.publisher_priority
                    && there.object.payload == cached.object.payload;
                return if same {
                    Verdict::Fine
                } else {
                    Verdict::Malformed
                };
            }
            // A status where an Object is, an Object where an End of Group
            // is, or the same status again.
            if status != there.object.status {
                group.give_up(in_use);
            }
            return Verdict::Fine;
        }
        let ended_at = group
            .objects
            .iter()
            .find(|(_, there)| ends_the_group(there.object.status))
            .map(|(id, _)| *id);
        if ended_at.is_some_and(|end| object_id > end) {
            // A status up there says nothing the End of Group did not say
            // (§11.2.1.1: no Objects exist from it on, in the Group or in
            // the track), and a publisher may end its track that way.
            if status == ObjectStatus::Normal {
                return Verdict::Malformed;
            }
            group.give_up(in_use);
            return Verdict::Fine;
        }
        // Inside a gap that an Object after it announced.
        let in_a_gap = group
            .objects
            .range(object_id..)
            .any(|(id, there)| id.saturating_sub(there.prior_gap()) <= object_id);
        if in_a_gap {
            // WHY a status is not held against the track, though §12.9 has
            // "an Object with an Object ID within a previously communicated
            // gap" and a status has an Object ID: the gap says that no
            // Object is there, and the status that none is from there on.
            if status == ObjectStatus::Normal {
                return Verdict::Malformed;
            }
            group.give_up(in_use);
            return Verdict::Fine;
        }
        let size = cached.size();
        let fits = group.objects.len() < MAX_CACHED_OBJECTS_OF_A_TRACK
            && group.bytes.saturating_add(size) <= MAX_CACHED_BYTES_OF_A_TRACK
            && in_use.saturating_add(size) <= MAX_CACHED_BYTES;
        // An End of Group or of Track below Objects that came: taken back,
        // or not.
        let below = ends_the_group(status) && group.objects.range(object_id..).next().is_some();
        if !fits || below {
            group.give_up(in_use);
            return Verdict::Fine;
        }
        group.bytes = group.bytes.saturating_add(size);
        *in_use = in_use.saturating_add(size);
        group.objects.insert(object_id, cached);
        if !sure_of_subgroup {
            group.unsure_of_subgroup.insert(object_id);
        }
        Verdict::Fine
    }

    /// An Object of the Group `group_id` was passed on and cannot be kept:
    /// the Group answers no FETCH.
    pub(crate) fn leave_out(&mut self, group_id: u64, in_use: &mut usize) {
        if let Some(group) = self.group_of(group_id, in_use) {
            group.give_up(in_use);
        }
    }

    /// The Group that is kept, when it is the Group `group_id`; a later
    /// Group takes the place of the one that was kept. `None` for a Group
    /// before it.
    fn group_of(&mut self, group_id: u64, in_use: &mut usize) -> Option<&mut Group> {
        if self.0.as_ref().is_none_or(|group| group.id < group_id) {
            self.clear(in_use);
            self.0 = Some(Box::new(Group {
                id: group_id,
                objects: BTreeMap::new(),
                unsure_of_subgroup: BTreeSet::new(),
                bytes: 0,
                given_up: false,
            }));
        }
        self.0.as_deref_mut().filter(|group| group.id == group_id)
    }

    /// Whether the cache has the Objects of the Group `group_id`: it is the
    /// Group that is kept, and was not given up.
    pub(crate) fn keeps(&self, group_id: u64) -> bool {
        self.0
            .as_ref()
            .is_some_and(|group| group.id == group_id && !group.given_up)
    }

    /// The Group whose Objects the cache has.
    pub(crate) fn kept_group(&self) -> Option<u64> {
        self.0
            .as_ref()
            .filter(|group| !group.given_up)
            .map(|group| group.id)
    }

    /// Lets go of everything, for a track that ends or may not be cached
    /// any more.
    pub(crate) fn clear(&mut self, in_use: &mut usize) {
        if let Some(group) = self.0.take() {
            *in_use = in_use.saturating_sub(group.bytes);
        }
    }

    /// The Objects from `start` to `last`, both included, in the order of a
    /// fetch stream (§10.12.3: "Within each group, objects are sent in
    /// Object ID order; subgroup ID is not used for ordering"), when the
    /// cache can answer for every Location of that range. `None` where it
    /// cannot, and the FETCH is a publisher's to answer.
    ///
    /// It can when the range lies in the Group it keeps and every Object ID
    /// of that Group from 0 to the last one of the range is one it has, or
    /// one it knows not to exist:
    ///
    /// * an Object ID that a later Object's Prior Object ID Gap covers
    ///   (§12.9);
    /// * an Object ID at or after an Object with the status End of Group
    ///   (§11.2.1.1: "no objects with the specified Group ID and the Object
    ///   ID that is greater than or equal to the one specified exist").
    ///
    /// WHY every Object ID from 0, and not from `start`: an Object that is
    /// missing on a fetch stream reads as one that does not exist
    /// (§10.12.3: "Any gaps in the Group and Object IDs in the response
    /// stream indicate objects that do not exist"), and of one that is not
    /// here and has not been said not to exist the relay does not know: its
    /// stream may be behind. "If it encounters an object in the requested
    /// range that is not cached and has unknown status, the relay MUST pause
    /// subsequent delivery until it has confirmed the object's status
    /// upstream"; the relay asks upstream for the whole FETCH instead of
    /// pausing in it. From 0, so that one number says how far the Group is
    /// known, whichever Subgroups its Objects came on.
    ///
    /// WHY NOT the `END_OF_GROUP` bit of a Subgroup Header: it says that the
    /// Subgroup is the last of its Group, and nothing of the Object IDs
    /// below the ones it carries.
    pub(crate) fn answer(&self, start: Location, last: Location) -> Option<Vec<CachedObject>> {
        let group = self.0.as_deref().filter(|group| !group.given_up)?;
        // A fetch stream gives every Object its Subgroup ID.
        if !group.unsure_of_subgroup.is_empty() {
            return None;
        }
        if start.group != group.id || last.group != group.id || start > last {
            return None;
        }
        // The first Object ID that is not known yet; none when the Group
        // is known to its end.
        let mut unknown_from = Some(0u64);
        for (object_id, cached) in &group.objects {
            // A gap larger than the Object ID makes the track malformed
            // (§12.9), which is not found here: the cache does not answer.
            let covered_from = object_id.checked_sub(cached.prior_gap())?;
            if unknown_from.is_none_or(|unknown| covered_from > unknown) {
                break;
            }
            unknown_from = if ends_the_group(cached.object.status) {
                None
            } else {
                object_id.checked_add(1)
            };
            if unknown_from.is_none_or(|unknown| unknown > last.object) {
                break;
            }
        }
        if unknown_from.is_some_and(|unknown| unknown <= last.object) {
            return None;
        }
        // A fetch stream has no Object Status (§11.4.4.1): an End of Group
        // or of Track is said by the Objects that are missing after it.
        Some(
            group
                .objects
                .range(start.object..=last.object)
                .map(|(_, cached)| cached)
                .filter(|cached| cached.object.status == ObjectStatus::Normal)
                .cloned()
                .collect(),
        )
    }
}

/// Whether no Object of the Group is from an Object with `status` on
/// (§11.2.1.1: End of Group, "no objects with the specified Group ID and the
/// Object ID that is greater than or equal to the one specified exist", and
/// End of Track, "no objects with the location that is equal to or greater
/// than the one specified exist").
fn ends_the_group(status: ObjectStatus) -> bool {
    matches!(status, ObjectStatus::EndOfGroup | ObjectStatus::EndOfTrack)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(group: u64, object: u64) -> Location {
        Location { group, object }
    }

    /// An Object with a payload of `len` bytes; `gap` is its Prior Object
    /// ID Gap, 0 for none.
    fn sized(group_id: u64, object_id: u64, gap: u64, len: usize) -> CachedObject {
        let properties = if gap == 0 {
            vec![]
        } else {
            vec![KeyValuePair::varint(PRIOR_OBJECT_ID_GAP, gap).unwrap()]
        };
        CachedObject {
            group_id,
            // Subgroups that take turns: the order is by Object ID alone.
            subgroup_id: object_id % 2,
            publisher_priority: 128,
            object: Arc::new(SubgroupObject {
                object_id,
                properties,
                status: ObjectStatus::Normal,
                payload: vec![0x61; len],
            }),
        }
    }

    fn object(group_id: u64, object_id: u64, gap: u64) -> CachedObject {
        sized(group_id, object_id, gap, 2)
    }

    fn status(group_id: u64, object_id: u64, status: ObjectStatus) -> CachedObject {
        CachedObject {
            group_id,
            subgroup_id: 0,
            publisher_priority: 128,
            object: Arc::new(SubgroupObject {
                object_id,
                properties: vec![],
                status,
                payload: vec![],
            }),
        }
    }

    /// The cache after these Objects came, and the bytes it holds.
    fn cache_of(objects: Vec<CachedObject>) -> (Cache, usize) {
        let mut cache = Cache::default();
        let mut in_use = 0;
        for object in objects {
            cache.insert(object, true, &mut in_use);
        }
        (cache, in_use)
    }

    /// The Objects of a Group that came, as (Object ID, Prior Object ID
    /// Gap).
    type Came = &'static [(u64, u64)];
    /// The first and the last Object ID a FETCH asks for.
    type Asked = (u64, u64);
    /// The Object IDs of an answer; `None` where the cache has none.
    type Answer = Option<&'static [u64]>;

    /// The Object IDs of an answer.
    fn ids(cache: &Cache, start: Location, last: Location) -> Option<Vec<u64>> {
        let objects = cache.answer(start, last)?;
        Some(
            objects
                .iter()
                .map(|cached| cached.object.object_id)
                .collect(),
        )
    }

    #[test]
    fn a_range_is_answered_when_every_object_id_up_to_its_last_is_known() {
        // (the Objects of Group 5 that came, as (Object ID, Prior Object ID
        //  Gap); the first and the last Object ID that are asked for; the
        //  Object IDs of the answer, `None` where the cache has none)
        let cases: [(Came, Asked, Answer); 15] = [
            (&[(0, 0), (1, 0), (2, 0)], (0, 2), Some(&[0, 1, 2])),
            (&[(0, 0), (1, 0), (2, 0)], (1, 1), Some(&[1])),
            (&[(0, 0), (1, 0), (2, 0)], (0, 0), Some(&[0])),
            // Object 3 has not come, and nothing says that it does not
            // exist.
            (&[(0, 0), (1, 0), (2, 0)], (0, 3), None),
            (&[(0, 0), (1, 0), (2, 0)], (3, 3), None),
            // In the order of the Object IDs, whatever order they came in.
            (&[(2, 0), (0, 0), (1, 0)], (0, 2), Some(&[0, 1, 2])),
            // The Group is not there from its first Object: also the part
            // that is there is not answered.
            (&[(1, 0), (2, 0)], (1, 2), None),
            // Object 1 is missing, and may still come.
            (&[(0, 0), (2, 0)], (0, 2), None),
            (&[(0, 0), (2, 0)], (2, 2), None),
            // Up to the Object before it, the Group is known.
            (&[(0, 0), (2, 0)], (0, 0), Some(&[0])),
            // Object 2 says that the one before it does not exist (§12.9).
            (&[(0, 0), (2, 1)], (0, 2), Some(&[0, 2])),
            (&[(0, 0), (2, 1)], (1, 1), Some(&[])),
            // Object 3 says so of Object 2 alone: Object 1 is not known.
            (&[(0, 0), (3, 1)], (0, 3), None),
            // The first Object says that the two before it do not exist.
            (&[(2, 2), (3, 0)], (0, 3), Some(&[2, 3])),
            // A gap larger than the Object ID is a malformed track, which
            // the cache does not answer for.
            (&[(1, 5)], (0, 1), None),
        ];
        for (came, (first, last), answer) in cases {
            let objects = came.iter().map(|(id, gap)| object(5, *id, *gap)).collect();
            let (cache, _) = cache_of(objects);
            assert_eq!(
                ids(&cache, at(5, first), at(5, last)).as_deref(),
                answer,
                "{came:?} {first} {last}"
            );
        }

        // An End of Group or of Track at Object 2: no Object ID from there
        // on exists (§11.2.1.1), and a fetch stream says so by what it
        // leaves out.
        for end in [ObjectStatus::EndOfGroup, ObjectStatus::EndOfTrack] {
            let ended = vec![object(5, 0, 0), object(5, 1, 0), status(5, 2, end)];
            let (cache, _) = cache_of(ended);
            // (the first and the last Object ID asked for; the answer)
            let cases: [(Asked, Answer); 5] = [
                ((0, 1), Some(&[0, 1])),
                ((0, 2), Some(&[0, 1])),
                ((1, 900), Some(&[1])),
                ((2, 900), Some(&[])),
                // Up to the last Object ID there is.
                ((0, u64::MAX), Some(&[0, 1])),
            ];
            for ((first, last), answer) in cases {
                assert_eq!(
                    ids(&cache, at(5, first), at(5, last)).as_deref(),
                    answer,
                    "{end:?} {first} {last}"
                );
            }
            // But not where an Object before it is missing.
            let (cache, _) = cache_of(vec![object(5, 0, 0), status(5, 2, end)]);
            assert_eq!(ids(&cache, at(5, 0), at(5, 2)), None, "{end:?}");
        }
    }

    #[test]
    fn a_range_outside_the_group_that_is_kept_is_not_answered() {
        let (cache, _) = cache_of(vec![object(5, 0, 0), object(5, 1, 0)]);
        // (the first Location; the last; whether the cache answers)
        let cases = [
            (at(5, 0), at(5, 1), true),
            (at(4, 0), at(5, 1), false),
            (at(4, 0), at(4, 9), false),
            (at(5, 0), at(6, 0), false),
            (at(6, 0), at(6, 0), false),
            // A range that ends before it starts.
            (at(5, 1), at(5, 0), false),
        ];
        for (start, last, answered) in cases {
            assert_eq!(
                cache.answer(start, last).is_some(),
                answered,
                "{start:?} {last:?}"
            );
        }
        // Nothing came.
        assert_eq!(Cache::default().answer(at(0, 0), at(0, 0)), None);
    }

    #[test]
    fn the_latest_group_is_kept_and_the_first_object_of_an_id() {
        // A later Group takes the place of the one that was kept; an Object
        // of the Group before it, which comes late, is not taken.
        let (cache, in_use) = cache_of(vec![
            object(5, 0, 0),
            object(5, 1, 0),
            object(6, 0, 0),
            object(5, 2, 0),
        ]);
        assert_eq!(ids(&cache, at(5, 0), at(5, 1)), None);
        assert_eq!(ids(&cache, at(6, 0), at(6, 0)), Some(vec![0]));
        // The two bytes of the one Object that is kept.
        assert_eq!(in_use, 2);

        // The Object that came first under an ID stays.
        let (cache, in_use) = cache_of(vec![sized(5, 0, 0, 3), sized(5, 0, 0, 7)]);
        let kept = cache.answer(at(5, 0), at(5, 0)).unwrap();
        assert_eq!((kept[0].object.payload.len(), in_use), (3, 3));

        // A Group of which an Object was passed on that could not be kept
        // answers nothing; the next Group starts anew.
        let (mut cache, mut in_use) = cache_of(vec![object(5, 0, 0)]);
        cache.leave_out(5, &mut in_use);
        cache.insert(object(5, 1, 0), true, &mut in_use);
        assert_eq!((ids(&cache, at(5, 0), at(5, 0)), in_use), (None, 0));
        // One of a Group before it changes nothing.
        cache.insert(object(6, 0, 0), true, &mut in_use);
        cache.leave_out(5, &mut in_use);
        assert_eq!(ids(&cache, at(6, 0), at(6, 0)), Some(vec![0]));
        // And one of a Group after it takes the place of what is kept.
        cache.leave_out(7, &mut in_use);
        assert_eq!((ids(&cache, at(6, 0), at(6, 0)), in_use), (None, 0));

        cache.insert(object(8, 0, 0), true, &mut in_use);
        assert_eq!(in_use, 2);
        cache.clear(&mut in_use);
        assert_eq!((ids(&cache, at(8, 0), at(8, 0)), in_use), (None, 0));
    }

    #[test]
    fn a_group_that_does_not_fit_is_given_up_and_the_next_one_starts_anew() {
        const MIB: usize = 1024 * 1024;
        // The expectations below are spelled out for these limits.
        assert_eq!(
            (
                MAX_CACHED_BYTES_OF_A_TRACK,
                MAX_CACHED_OBJECTS_OF_A_TRACK,
                MAX_CACHED_BYTES
            ),
            (8 * MIB, 4_096, 256 * MIB)
        );
        // Eight Objects of 1 MiB fit; one byte more does not, and the whole
        // Group goes.
        let mut cache = Cache::default();
        let mut in_use = 0;
        for id in 0..8 {
            cache.insert(sized(5, id, 0, MIB), true, &mut in_use);
        }
        assert_eq!(in_use, 8 * MIB);
        assert_eq!(
            ids(&cache, at(5, 0), at(5, 7)).map(|ids| ids.len()),
            Some(8)
        );
        cache.insert(sized(5, 8, 0, 1), true, &mut in_use);
        assert_eq!((ids(&cache, at(5, 0), at(5, 7)), in_use), (None, 0));
        // What comes of that Group afterwards is not kept either.
        cache.insert(sized(5, 9, 0, 1), true, &mut in_use);
        assert_eq!(in_use, 0);
        cache.insert(sized(6, 0, 0, 1), true, &mut in_use);
        assert_eq!(
            (ids(&cache, at(6, 0), at(6, 0)), in_use),
            (Some(vec![0]), 1)
        );

        // 4,096 Objects fit, and the next one does not.
        let mut cache = Cache::default();
        let mut in_use = 0;
        for id in 0..4_096 {
            cache.insert(sized(5, id, 0, 1), true, &mut in_use);
        }
        assert_eq!(in_use, 4_096);
        assert!(cache.answer(at(5, 0), at(5, 4_095)).is_some());
        cache.insert(sized(5, 4_096, 0, 1), true, &mut in_use);
        assert_eq!((ids(&cache, at(5, 0), at(5, 0)), in_use), (None, 0));

        // What the other tracks hold counts: with one byte left for all of
        // them, an Object of two bytes does not fit.
        let mut cache = Cache::default();
        let mut in_use = 256 * MIB - 1;
        cache.insert(sized(5, 0, 0, 1), true, &mut in_use);
        assert_eq!(in_use, 256 * MIB);
        cache.insert(sized(5, 1, 0, 1), true, &mut in_use);
        // The Group is given up, and what it held is free again.
        assert_eq!(in_use, 256 * MIB - 1);
        assert_eq!(ids(&cache, at(5, 0), at(5, 0)), None);

        // Object Properties count: a number as 8 bytes, and bytes as what
        // they are long.
        let (_, in_use) = cache_of(vec![object(5, 1, 1)]);
        assert_eq!(in_use, 10);
        let with_bytes = CachedObject {
            group_id: 5,
            subgroup_id: 0,
            publisher_priority: 128,
            object: Arc::new(SubgroupObject {
                object_id: 0,
                properties: vec![
                    KeyValuePair::bytes(0x0d, vec![7; 5]).unwrap(),
                    KeyValuePair::varint(0x10, 1).unwrap(),
                ],
                status: ObjectStatus::Normal,
                payload: vec![0x61; 3],
            }),
        };
        let (_, in_use) = cache_of(vec![with_bytes]);
        assert_eq!(in_use, 3 + 5 + 8);
    }

    /// What a test says of an Object: (Object ID; Prior Object ID Gap;
    /// the length of its payload, or `None` for an End of Group).
    type Said = (u64, u64, Option<usize>);

    fn said(group_id: u64, (object_id, gap, payload): Said) -> CachedObject {
        match payload {
            Some(len) => sized(group_id, object_id, gap, len),
            None => status(group_id, object_id, ObjectStatus::EndOfGroup),
        }
    }

    #[test]
    fn an_object_is_compared_with_what_is_kept_of_its_group() {
        use Verdict::{Fine, Malformed};
        const END: Option<usize> = None;
        let two = Some(2);
        // (the Objects of Group 5 that are kept; the Object that comes; what
        //  it shows of the track; whether the Group still answers a FETCH
        //  for its first Object)
        let cases: [(&[Said], Said, Verdict, bool); 24] = [
            // The same Object again.
            (&[(0, 0, two), (1, 0, two)], (1, 0, two), Fine, true),
            // With another payload: of another length, and of none.
            (
                &[(0, 0, two), (1, 0, two)],
                (1, 0, Some(3)),
                Malformed,
                true,
            ),
            (
                &[(0, 0, two), (1, 0, two)],
                (1, 0, Some(0)),
                Malformed,
                true,
            ),
            // With another Object Property and nothing else: not malformed,
            // and the one that came first stays.
            (&[(0, 0, two), (2, 1, two)], (2, 0, two), Fine, true),
            // A new Object after the ones that are kept, and before them.
            (&[(0, 0, two), (1, 0, two)], (2, 0, two), Fine, true),
            (&[(0, 0, two), (2, 0, two)], (1, 0, two), Fine, true),
            // After an End of Group: malformed. Below it: an Object like
            // any other.
            (&[(0, 0, two), (2, 0, END)], (3, 0, two), Malformed, true),
            (&[(0, 0, two), (2, 0, END)], (9, 0, two), Malformed, true),
            (&[(0, 0, two), (2, 0, END)], (1, 0, two), Fine, true),
            // At the very Object ID of the End of Group: not malformed, not
            // kept, and the Group answers nothing any more.
            (&[(0, 0, two), (2, 0, END)], (2, 0, two), Fine, false),
            // The End of Group again.
            (&[(0, 0, two), (2, 0, END)], (2, 0, END), Fine, true),
            // An End of Group where an Object is, and below Objects that
            // came: taken back, perhaps. Not malformed, and not answered.
            (&[(0, 0, two), (1, 0, two)], (1, 0, END), Fine, false),
            (&[(0, 0, two), (2, 0, two)], (1, 0, END), Fine, false),
            // After them it is the end of the Group.
            (&[(0, 0, two), (1, 0, two)], (2, 0, END), Fine, true),
            // A second End of Group after the first says nothing the first
            // did not: not malformed, and not answered.
            (&[(0, 0, two), (2, 0, END)], (4, 0, END), Fine, false),
            (&[(0, 0, two), (2, 0, END)], (3, 0, END), Fine, false),
            // Inside a gap that a later Object announced: Object 3 said
            // that 1 and 2 do not exist.
            (&[(0, 0, two), (3, 2, two)], (1, 0, two), Malformed, true),
            (&[(0, 0, two), (3, 2, two)], (2, 0, two), Malformed, true),
            // A status there says nothing the gap did not: not malformed,
            // and not answered.
            (&[(0, 0, two), (3, 2, two)], (2, 0, END), Fine, false),
            // Before the gap, and after the Object that has it.
            (&[(1, 0, two), (4, 2, two)], (0, 0, two), Fine, true),
            (&[(0, 0, two), (3, 2, two)], (4, 0, two), Fine, true),
            // A gap that covers an Object that came.
            (&[(0, 0, two), (1, 0, two)], (3, 2, two), Malformed, true),
            // Also on an Object that comes again, now with a gap.
            (
                &[(0, 0, two), (1, 0, two), (3, 0, two)],
                (3, 2, two),
                Malformed,
                true,
            ),
            // One that ends right after it.
            (&[(0, 0, two), (1, 0, two)], (3, 1, two), Fine, true),
        ];
        for (kept, comes, verdict, answers) in cases {
            let objects = kept.iter().map(|object| said(5, *object)).collect();
            let (mut cache, mut in_use) = cache_of(objects);
            let before = in_use;
            assert_eq!(
                cache.insert(said(5, comes), true, &mut in_use),
                verdict,
                "{kept:?} {comes:?}"
            );
            assert_eq!(
                cache.answer(at(5, 0), at(5, 0)).is_some(),
                answers,
                "{kept:?} {comes:?}"
            );
            // An Object the track is malformed for is not kept: it counts
            // for nothing, and is not there to be the same when it comes
            // again.
            if verdict == Malformed {
                assert_eq!(in_use, before, "{kept:?} {comes:?}");
                assert_eq!(
                    cache.insert(said(5, comes), true, &mut in_use),
                    Malformed,
                    "{kept:?} {comes:?}"
                );
            }
        }
    }

    #[test]
    fn an_end_of_track_after_the_end_of_a_group_is_not_held_against_the_track() {
        // Object 0 and End of Group at 1; then End of Track at 2 (§11.2.1.1:
        // each of the two says that nothing exists from it on).
        let (mut cache, mut in_use) = cache_of(vec![
            object(5, 0, 0),
            status(5, 1, ObjectStatus::EndOfGroup),
        ]);
        let end_of_track = status(5, 2, ObjectStatus::EndOfTrack);
        assert_eq!(cache.insert(end_of_track, true, &mut in_use), Verdict::Fine);
        assert_eq!((cache.answer(at(5, 0), at(5, 0)), in_use), (None, 0));
    }

    #[test]
    fn a_subgroup_id_the_relay_is_not_sure_of_is_not_held_against_the_track() {
        let first = || object(5, 1, 0);
        let of_subgroup_9 = || CachedObject {
            subgroup_id: 9,
            ..first()
        };
        let with_another_payload = || sized(5, 1, 0, 9);
        // (whether the relay is sure of the Subgroup ID of the Object that
        //  came first; the Object that comes again; whether it is sure of
        //  that one's; what it shows of the track)
        let cases = [
            (true, of_subgroup_9(), true, Verdict::Malformed),
            (true, of_subgroup_9(), false, Verdict::Fine),
            (false, of_subgroup_9(), true, Verdict::Fine),
            (false, of_subgroup_9(), false, Verdict::Fine),
            // The payload is compared all the same.
            (false, with_another_payload(), false, Verdict::Malformed),
            (true, with_another_payload(), false, Verdict::Malformed),
        ];
        for (sure_of_first, comes, sure, verdict) in cases {
            let (mut cache, mut in_use) = cache_of(vec![object(5, 0, 0)]);
            cache.insert(first(), sure_of_first, &mut in_use);
            let what = format!("{sure_of_first} {comes:?} {sure}");
            assert_eq!(cache.insert(comes, sure, &mut in_use), verdict, "{what}");
            // A Group with a Subgroup ID the relay is not sure of answers
            // no FETCH: a fetch stream states the Subgroup ID.
            assert_eq!(
                cache.answer(at(5, 0), at(5, 0)).is_some(),
                sure_of_first,
                "{what}"
            );
        }
    }

    #[test]
    fn an_object_that_comes_again_is_the_same_by_its_subgroup_its_priority_and_its_payload() {
        let first = || object(5, 1, 0);
        let other = |change: fn(&mut CachedObject)| {
            let mut object = first();
            change(&mut object);
            object
        };
        // (the Object that comes after `first`; what it shows of the track)
        let cases = [
            (first(), Verdict::Fine),
            (other(|object| object.subgroup_id = 9), Verdict::Malformed),
            (
                other(|object| object.publisher_priority = 7),
                Verdict::Malformed,
            ),
            // The same length, and another byte.
            (
                other(|object| {
                    let mut changed = (*object.object).clone();
                    changed.payload = vec![0x61, 0x62];
                    object.object = Arc::new(changed);
                }),
                Verdict::Malformed,
            ),
            // Object Properties alone.
            (
                other(|object| {
                    let mut changed = (*object.object).clone();
                    changed.properties = vec![KeyValuePair::varint(0x10, 7).unwrap()];
                    object.object = Arc::new(changed);
                }),
                Verdict::Fine,
            ),
        ];
        for (comes, verdict) in cases {
            let (mut cache, mut in_use) = cache_of(vec![object(5, 0, 0), first()]);
            assert_eq!(
                cache.insert(comes.clone(), true, &mut in_use),
                verdict,
                "{comes:?}"
            );
            // The one that came first is the one that is kept.
            let kept = cache.answer(at(5, 1), at(5, 1)).unwrap();
            assert_eq!(kept, [first()], "{comes:?}");
        }
        // Nothing is compared with a Group that is not kept: an earlier
        // one, and one that was given up.
        let (mut cache, mut in_use) = cache_of(vec![object(5, 0, 0), object(6, 0, 0)]);
        assert_eq!(
            cache.insert(sized(5, 0, 0, 9), true, &mut in_use),
            Verdict::Fine
        );
        cache.leave_out(6, &mut in_use);
        assert_eq!(
            cache.insert(sized(6, 0, 0, 9), true, &mut in_use),
            Verdict::Fine
        );
    }

    #[test]
    fn a_track_with_max_cache_duration_is_not_cached() {
        let other = KeyValuePair::varint(2, 5).unwrap();
        let duration = KeyValuePair::varint(MAX_CACHE_DURATION, 1_000).unwrap();
        // Immutable Properties (0x0B), whatever they contain: the relay
        // does not look into them.
        let immutable = KeyValuePair::bytes(0x0b, vec![0x02, 0x05]).unwrap();
        // (the Track Properties; whether its Objects may be cached)
        let cases = [
            (vec![], true),
            (vec![other.clone()], true),
            (vec![duration.clone()], false),
            (vec![other.clone(), duration], false),
            (vec![immutable.clone()], false),
            (vec![other, immutable], false),
        ];
        for (properties, cached) in cases {
            assert_eq!(may_be_cached(&properties), cached, "{properties:?}");
        }
    }
}
