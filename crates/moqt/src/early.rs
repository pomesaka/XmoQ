//! Objects of subgroup streams whose Track Alias is not known yet: what a
//! subscriber does with them, as `@xmoq/moqt` does
//! (`packages/moqt/src/core/incoming-subgroups.ts`). The shared rows are in
//! `fixtures/early-objects/`.
//!
//! §11.4.2: "If an endpoint receives a subgroup with an unknown Track Alias,
//! it MAY abandon the stream, or choose to buffer it for a brief period to
//! handle reordering with the control message that establishes the Track
//! Alias."
//!
//! WHY held while a `SUBSCRIBE` of this endpoint waits for its answer, of
//! the two the draft allows: the `SUBSCRIBE_OK` and the first Objects come
//! on different streams, and the Objects can be read first, when a relay
//! answers a `SUBSCRIBE` it held for a publisher (`RENDEZVOUS_TIMEOUT`)
//! and passes on that publisher's first Object at once. Abandoned, those
//! Objects are lost: the line of a chat that is said once, the first words
//! of a transcript. WHY NOT held without a `SUBSCRIBE` waiting: no answer
//! can then give the alias, and the stream is the peer's mistake.
//! WHY NOT for a time: the answer it waits for has a time of its own (the
//! request's), after which the `SUBSCRIBE` is no longer Pending and what
//! is held is let go of.

use std::collections::VecDeque;

use crate::data::SubgroupObject;
use crate::kvp::Value;
use crate::session::{Session, SubscribedAlias};
use crate::stream_reset_code;

/// The bytes held for all the streams of a session together. **Provisional**,
/// as `maxHeldSubgroupBytes` of `@xmoq/moqt`. WHY 1 MiB: room for the first
/// Group of the demo's video at 1.5 Mbit/s (a key frame of about 200 KB) or
/// many small Objects (a chat's lines are at most 512 bytes), while a peer
/// that sends under aliases it never gives costs a session no more.
pub const MOST_HELD_BYTES: usize = 1024 * 1024;

/// What a held Object is counted at besides its payload, so that Objects
/// without one are bounded too; as `@xmoq/moqt` counts.
const OBJECT_OVERHEAD: usize = 64;
/// What each Object Property is counted at besides the bytes of its value.
const PROPERTY_OVERHEAD: usize = 16;

/// The bytes `object` is counted at while it is held.
#[must_use]
pub fn held_bytes(object: &SubgroupObject) -> usize {
    let properties: usize = object
        .properties
        .iter()
        .map(|property| {
            PROPERTY_OVERHEAD
                + match property.value() {
                    Value::Varint(_) => 0,
                    Value::Bytes(bytes) => bytes.len(),
                }
        })
        .sum();
    OBJECT_OVERHEAD + object.payload.len() + properties
}

/// What becomes of an Object, by what its Track Alias is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// To the subscription.
    Deliver,
    /// Held until its alias is known ([`EarlyObjects::settle`]).
    Hold,
    /// Its stream is stopped with this Stream Reset Error Code, and what it
    /// held is let go of.
    Stop(u32),
}

/// What becomes of an Object of an alias in `alias`, while `held` bytes are
/// held for the session, where it is counted at `bytes`.
#[must_use]
pub fn arrival(
    alias: SubscribedAlias<'_>,
    subscribe_pending: bool,
    held: usize,
    bytes: usize,
) -> Arrival {
    match alias {
        SubscribedAlias::Established(_) | SubscribedAlias::Draining(_) => Arrival::Deliver,
        // §11.1: Objects of a subscription that is over are not wanted.
        SubscribedAlias::Retired => Arrival::Stop(stream_reset_code::CANCELLED),
        SubscribedAlias::Unknown if !subscribe_pending => {
            Arrival::Stop(stream_reset_code::CANCELLED)
        }
        // WHY INTERNAL_ERROR, as `@xmoq/moqt`: the limit is this side's.
        SubscribedAlias::Unknown if held.saturating_add(bytes) > MOST_HELD_BYTES => {
            Arrival::Stop(stream_reset_code::INTERNAL_ERROR)
        }
        SubscribedAlias::Unknown => Arrival::Hold,
    }
}

/// What becomes of the Objects held for an alias once something changed:
/// a `SUBSCRIBE_OK`, a `SUBSCRIBE` that ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// They go to the subscription, in the order they came.
    Release,
    /// They stay held.
    Keep,
    /// Their stream is stopped with this code, and they are let go of.
    Stop(u32),
}

/// What becomes of Objects held for an alias that is now `alias`.
#[must_use]
pub fn settle(alias: SubscribedAlias<'_>, subscribe_pending: bool) -> Settle {
    match arrival(alias, subscribe_pending, 0, 0) {
        Arrival::Deliver => Settle::Release,
        Arrival::Hold => Settle::Keep,
        Arrival::Stop(code) => Settle::Stop(code),
    }
}

/// One held Object, with what the caller needs of it again.
#[derive(Debug)]
struct Held<T> {
    stream: u64,
    track_alias: u64,
    bytes: usize,
    item: T,
}

/// The Objects of a session held for an alias that is not known yet, in
/// the order they came. `T` is how the caller has them (an event of its
/// readers).
#[derive(Debug)]
pub struct EarlyObjects<T> {
    held: VecDeque<Held<T>>,
    bytes: usize,
}

impl<T> Default for EarlyObjects<T> {
    fn default() -> Self {
        Self {
            held: VecDeque::new(),
            bytes: 0,
        }
    }
}

/// What came of an Object given to [`EarlyObjects::arrive`].
#[derive(Debug)]
pub enum Arrived<T> {
    /// To the subscription now.
    Deliver(T),
    /// Held.
    Held,
    /// The stream is to be stopped with this code; what it held was let go of.
    Stop {
        /// The stream.
        stream: u64,
        /// Its Stream Reset Error Code.
        code: u32,
    },
}

/// What [`EarlyObjects::settle`] let go of.
#[derive(Debug)]
pub struct Settled<T> {
    /// Objects whose alias is known now, in the order they came.
    pub released: Vec<T>,
    /// Streams to stop, each once, with their code.
    pub stopped: Vec<(u64, u32)>,
}

impl<T> EarlyObjects<T> {
    /// An Object of `stream` under `track_alias`, counted at `bytes`
    /// ([`held_bytes`]).
    pub fn arrive(
        &mut self,
        session: &Session,
        stream: u64,
        track_alias: u64,
        bytes: usize,
        item: T,
    ) -> Arrived<T> {
        // A stream that holds Objects goes on being held, so that its
        // Objects stay in their order.
        let alias = if self.held.iter().any(|held| held.stream == stream) {
            SubscribedAlias::Unknown
        } else {
            session.subscribed_alias(track_alias)
        };
        match arrival(alias, session.has_pending_subscribe(), self.bytes, bytes) {
            Arrival::Deliver => Arrived::Deliver(item),
            Arrival::Hold => {
                self.bytes += bytes;
                self.held.push_back(Held {
                    stream,
                    track_alias,
                    bytes,
                    item,
                });
                Arrived::Held
            }
            Arrival::Stop(code) => {
                self.forget(stream);
                Arrived::Stop { stream, code }
            }
        }
    }

    /// The stream ended. One that was reset lets go of what it held, as
    /// `@xmoq/moqt` does; one that ended with its FIN keeps it.
    pub fn stream_closed(&mut self, stream: u64, reset: bool) {
        if reset {
            self.forget(stream);
        }
    }

    /// Lets go of, or keeps, what is held, by what the session knows now of
    /// each alias.
    pub fn settle(&mut self, session: &Session) -> Settled<T> {
        let pending = session.has_pending_subscribe();
        let mut settled = Settled {
            released: Vec::new(),
            stopped: Vec::new(),
        };
        let mut kept = VecDeque::new();
        for held in std::mem::take(&mut self.held) {
            match settle(session.subscribed_alias(held.track_alias), pending) {
                Settle::Release => {
                    self.bytes -= held.bytes;
                    settled.released.push(held.item);
                }
                Settle::Keep => kept.push_back(held),
                Settle::Stop(code) => {
                    self.bytes -= held.bytes;
                    if !settled
                        .stopped
                        .iter()
                        .any(|(stream, _)| *stream == held.stream)
                    {
                        settled.stopped.push((held.stream, code));
                    }
                }
            }
        }
        self.held = kept;
        settled
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    fn forget(&mut self, stream: u64) {
        let bytes = &mut self.bytes;
        self.held.retain(|held| {
            let keep = held.stream != stream;
            if !keep {
                *bytes -= held.bytes;
            }
            keep
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::kvp::KeyValuePair;
    use crate::name::FullTrackName;

    /// A table of `fixtures/early-objects/`, which `@xmoq/moqt` reads too.
    pub(crate) fn rows(file: &str) -> Vec<Vec<String>> {
        let path = format!(
            "{}/../../fixtures/early-objects/{file}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{path}: {error}"))
            .lines()
            .map(|row| row.split('\t').map(str::to_string).collect())
            .collect()
    }

    fn alias_of<'a>(name: &str, track: &'a FullTrackName) -> SubscribedAlias<'a> {
        match name {
            "established" => SubscribedAlias::Established(track),
            "draining" => SubscribedAlias::Draining(track),
            "retired" => SubscribedAlias::Retired,
            "unknown" => SubscribedAlias::Unknown,
            other => panic!("an alias the test does not know: {other}"),
        }
    }

    fn code_of(outcome: &str) -> u32 {
        match outcome {
            "stop-cancelled" => stream_reset_code::CANCELLED,
            "stop-internal-error" => stream_reset_code::INTERNAL_ERROR,
            other => panic!("an outcome the test does not know: {other}"),
        }
    }

    fn track() -> FullTrackName {
        FullTrackName::new(
            crate::name::TrackNamespace::new(vec![b"t".to_vec()]).unwrap(),
            b"n".to_vec(),
        )
        .unwrap()
    }

    /// The rows `@xmoq/moqt` reads as well.
    #[test]
    fn an_object_is_delivered_held_or_stopped_where_the_shared_table_says() {
        let track = track();
        let rows = rows("arrival.tsv");
        assert_eq!(rows.len(), 7);
        for row in rows {
            let [name, alias, pending, held, bytes, outcome] = row.as_slice() else {
                panic!("a row of six columns: {row:?}");
            };
            let expected = match outcome.as_str() {
                "deliver" => Arrival::Deliver,
                "hold" => Arrival::Hold,
                other => Arrival::Stop(code_of(other)),
            };
            assert_eq!(
                arrival(
                    alias_of(alias, &track),
                    pending == "yes",
                    held.parse().unwrap(),
                    bytes.parse().unwrap()
                ),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn what_is_held_is_released_kept_or_stopped_where_the_shared_table_says() {
        let track = track();
        let rows = rows("settle.tsv");
        assert_eq!(rows.len(), 5);
        for row in rows {
            let [name, alias, pending, outcome] = row.as_slice() else {
                panic!("a row of four columns: {row:?}");
            };
            let expected = match outcome.as_str() {
                "release" => Settle::Release,
                "keep" => Settle::Keep,
                other => Settle::Stop(code_of(other)),
            };
            assert_eq!(
                settle(alias_of(alias, &track), pending == "yes"),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn a_held_object_is_counted_as_the_shared_table_says() {
        let rows = rows("held-bytes.tsv");
        assert_eq!(rows.len(), 5);
        for row in rows {
            let [name, payload, properties, bytes] = row.as_slice() else {
                panic!("a row of four columns: {row:?}");
            };
            let properties = if properties == "-" {
                Vec::new()
            } else {
                properties
                    .split(',')
                    .map(|property| match property.strip_prefix('b') {
                        Some(length) => {
                            KeyValuePair::bytes(1, vec![0; length.parse().unwrap()]).unwrap()
                        }
                        None => KeyValuePair::varint(2, 0).unwrap(),
                    })
                    .collect()
            };
            let object = SubgroupObject {
                object_id: 0,
                properties,
                status: crate::data::ObjectStatus::Normal,
                payload: vec![0; payload.parse().unwrap()],
            };
            assert_eq!(
                held_bytes(&object),
                bytes.parse::<usize>().unwrap(),
                "{name}"
            );
        }
    }
}
