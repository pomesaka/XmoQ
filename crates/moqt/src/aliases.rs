//! Track Aliases on the subscribing side (§11.1) and what becomes of them when a
//! subscription ends (§10.11).

use std::collections::{HashMap, VecDeque};

use crate::name::FullTrackName;

/// Track Aliases of ended subscriptions that are remembered, oldest dropped first.
///
/// WHY remember them: Objects can arrive after a subscription ended, and §11.1
/// says a subscriber SHOULD keep enough state to discard them quickly instead of
/// treating them as an unknown alias.
/// WHY NOT keep all of them, or keep them for a time: this crate has no clock,
/// and §10.11 lets a subscriber discard subscription state early. A fixed count
/// keeps the memory of a long session bounded.
pub(crate) const RETIRED_ALIAS_LIMIT: usize = 64;

/// Track Aliases without a subscription whose ended data streams are held,
/// oldest dropped first.
///
/// WHY hold them: a subgroup stream may be opened, read and closed before the
/// `SUBSCRIBE_OK` that assigns its alias is processed (§11.4.2 lets streams run
/// ahead, and a relay handles control and data in different tasks). Dropping
/// the count would leave `PUBLISH_DONE` waiting for a stream that already ended.
/// WHY a limit: a peer can open streams under aliases it never assigns.
pub(crate) const EARLY_CLOSED_ALIAS_LIMIT: usize = 64;

/// What a Track Alias from a Subgroup Header refers to on the subscribing side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribedAlias<'a> {
    /// An Established subscription to this track.
    Established(&'a FullTrackName),
    /// A subscription that `PUBLISH_DONE` ended. Objects that were already on
    /// their way still belong to this track (§10.11). It lasts until as many data
    /// streams have ended as the `PUBLISH_DONE` counted or the caller gives up
    /// waiting.
    Draining(&'a FullTrackName),
    /// A subscription that is over. Its late Objects are to be discarded (§11.1).
    Retired,
    /// No subscription this session knows of. The stream may be ahead of its
    /// `SUBSCRIBE_OK`, or stray (§11.4.2).
    Unknown,
}

/// A track this endpoint subscribed to.
#[derive(Debug)]
struct SubscribedTrack {
    full_track_name: FullTrackName,
    /// Data streams of the subscription that have ended so far.
    closed_streams: u64,
    /// The Stream Count of the `PUBLISH_DONE` that ended the subscription. While
    /// set, the subscription is over and its last Objects are still let through.
    draining: Option<u64>,
}

/// The tracks this endpoint subscribed to, by the Track Alias the publisher chose.
#[derive(Debug, Default)]
pub(crate) struct SubscribedAliases {
    tracks: HashMap<u64, SubscribedTrack>,
    retired: VecDeque<u64>,
    /// `(alias, data streams ended)` for aliases no subscription holds yet.
    early_closed: VecDeque<(u64, u64)>,
}

impl SubscribedAliases {
    pub(crate) fn state(&self, alias: u64) -> SubscribedAlias<'_> {
        match self.tracks.get(&alias) {
            Some(track) if track.draining.is_some() => {
                SubscribedAlias::Draining(&track.full_track_name)
            }
            Some(track) => SubscribedAlias::Established(&track.full_track_name),
            None if self.retired.contains(&alias) => SubscribedAlias::Retired,
            None => SubscribedAlias::Unknown,
        }
    }

    /// Records an accepted subscription under `alias`.
    ///
    /// The publisher may reuse the alias of an ended subscription, draining or
    /// retired. From then on its Objects belong to the new track; a late Object
    /// of the old one can no longer be told apart. The caller rejects an alias
    /// that is still Established before calling this (§11.1).
    pub(crate) fn establish(&mut self, alias: u64, full_track_name: FullTrackName) {
        self.retired.retain(|retired| *retired != alias);
        let closed_streams = self.take_early_closed(alias);
        self.tracks.insert(
            alias,
            SubscribedTrack {
                full_track_name,
                closed_streams,
                draining: None,
            },
        );
    }

    /// `PUBLISH_DONE` arrived: the alias goes on until `stream_count` data
    /// streams have ended, which may already be the case.
    pub(crate) fn drain(&mut self, alias: u64, stream_count: u64) {
        let Some(track) = self.tracks.get_mut(&alias) else {
            return;
        };
        track.draining = Some(stream_count);
        self.retire_if_drained(alias);
    }

    /// A data stream carrying `alias` ended, by FIN or by reset.
    ///
    /// For an alias no subscription holds yet the stream is held, see
    /// [`EARLY_CLOSED_ALIAS_LIMIT`]. For a retired alias it is a late stream of
    /// a subscription that is over, and is not counted towards the next one.
    pub(crate) fn stream_closed(&mut self, alias: u64) {
        if let Some(track) = self.tracks.get_mut(&alias) {
            track.closed_streams = track.closed_streams.saturating_add(1);
            self.retire_if_drained(alias);
        } else if !self.retired.contains(&alias) {
            self.hold_early_closed(alias);
        }
    }

    /// The caller stopped waiting for a draining alias. Nothing happens to an
    /// alias that is not draining.
    pub(crate) fn drain_timeout(&mut self, alias: u64) {
        let draining = self
            .tracks
            .get(&alias)
            .is_some_and(|track| track.draining.is_some());
        if draining {
            self.retire(alias);
        }
    }

    /// Remembers as retired an alias that a subscription was assigned and never
    /// used, so that streams under it are known to be unwanted. An alias that a
    /// subscription holds, established or draining, is left alone.
    pub(crate) fn retire_if_unknown(&mut self, alias: u64) {
        if self.state(alias) == SubscribedAlias::Unknown {
            self.retire(alias);
        }
    }

    /// Ends the alias at once, remembering it so that late Objects are discarded.
    pub(crate) fn retire(&mut self, alias: u64) {
        self.take_early_closed(alias);
        self.tracks.remove(&alias);
        self.retired.retain(|retired| *retired != alias);
        self.retired.push_back(alias);
        if self.retired.len() > RETIRED_ALIAS_LIMIT {
            self.retired.pop_front();
        }
    }

    fn hold_early_closed(&mut self, alias: u64) {
        if let Some((_, count)) = self
            .early_closed
            .iter_mut()
            .find(|(held, _)| *held == alias)
        {
            *count = count.saturating_add(1);
            return;
        }
        self.early_closed.push_back((alias, 1));
        if self.early_closed.len() > EARLY_CLOSED_ALIAS_LIMIT {
            self.early_closed.pop_front();
        }
    }

    /// The streams held for `alias`, which are then no longer held.
    fn take_early_closed(&mut self, alias: u64) -> u64 {
        let count = self
            .early_closed
            .iter()
            .find(|(held, _)| *held == alias)
            .map_or(0, |(_, count)| *count);
        self.early_closed.retain(|(held, _)| *held != alias);
        count
    }

    fn retire_if_drained(&mut self, alias: u64) {
        let drained = self.tracks.get(&alias).is_some_and(|track| {
            track
                .draining
                .is_some_and(|stream_count| track.closed_streams >= stream_count)
        });
        if drained {
            self.retire(alias);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::TrackNamespace;

    const ALIAS: u64 = 7;

    fn track(name: &[u8]) -> FullTrackName {
        FullTrackName::new(TrackNamespace::new(vec![]).unwrap(), name.to_vec()).unwrap()
    }

    /// The states an alias goes through after `PUBLISH_DONE`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Draining,
        Retired,
    }

    #[derive(Debug, Clone, Copy)]
    enum Step {
        StreamClosed,
        DrainTimeout,
    }

    fn phase(aliases: &SubscribedAliases) -> Phase {
        match aliases.state(ALIAS) {
            SubscribedAlias::Draining(full_track_name) => {
                assert_eq!(full_track_name, &track(b"a"));
                Phase::Draining
            }
            SubscribedAlias::Retired => Phase::Retired,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn publish_done_lets_the_counted_streams_finish() {
        // (Stream Count, streams closed before PUBLISH_DONE, phase right after
        // it, then steps with the phase each leaves)
        type Case = (u64, u64, Phase, &'static [(Step, Phase)]);
        let cases: &[Case] = &[
            (0, 0, Phase::Retired, &[]),
            (1, 1, Phase::Retired, &[]),
            (
                1,
                0,
                Phase::Draining,
                &[(Step::StreamClosed, Phase::Retired)],
            ),
            (
                2,
                1,
                Phase::Draining,
                &[(Step::StreamClosed, Phase::Retired)],
            ),
            (
                3,
                0,
                Phase::Draining,
                &[
                    (Step::StreamClosed, Phase::Draining),
                    (Step::DrainTimeout, Phase::Retired),
                ],
            ),
            // 2^62 - 1: the publisher does not know how many streams it opened.
            (
                4_611_686_018_427_387_903,
                0,
                Phase::Draining,
                &[
                    (Step::StreamClosed, Phase::Draining),
                    (Step::DrainTimeout, Phase::Retired),
                ],
            ),
        ];

        for (stream_count, closed_before, after_done, steps) in cases {
            let name = format!("count {stream_count}, {closed_before} closed before");
            let mut aliases = SubscribedAliases::default();
            aliases.establish(ALIAS, track(b"a"));
            for _ in 0..*closed_before {
                aliases.stream_closed(ALIAS);
            }
            // A timeout before PUBLISH_DONE has nothing to end.
            aliases.drain_timeout(ALIAS);
            assert_eq!(
                aliases.state(ALIAS),
                SubscribedAlias::Established(&track(b"a")),
                "{name}"
            );

            aliases.drain(ALIAS, *stream_count);
            assert_eq!(phase(&aliases), *after_done, "{name}");
            for (index, (step, expected)) in steps.iter().enumerate() {
                match step {
                    Step::StreamClosed => aliases.stream_closed(ALIAS),
                    Step::DrainTimeout => aliases.drain_timeout(ALIAS),
                }
                assert_eq!(phase(&aliases), *expected, "{name}, step {index}");
            }
        }
    }

    #[test]
    fn an_ended_alias_may_be_given_to_another_track() {
        // (how the first subscription ended)
        type Ending = fn(&mut SubscribedAliases);
        let endings: [(&str, Ending); 2] = [
            ("draining", |aliases| aliases.drain(ALIAS, 1)),
            ("retired", |aliases| aliases.retire(ALIAS)),
        ];
        for (name, end) in endings {
            let mut aliases = SubscribedAliases::default();
            aliases.establish(ALIAS, track(b"a"));
            end(&mut aliases);
            aliases.establish(ALIAS, track(b"b"));
            assert_eq!(
                aliases.state(ALIAS),
                SubscribedAlias::Established(&track(b"b")),
                "{name}"
            );
            // The stream count of the old subscription does not carry over.
            aliases.stream_closed(ALIAS);
            assert_eq!(
                aliases.state(ALIAS),
                SubscribedAlias::Established(&track(b"b")),
                "{name}"
            );
        }
    }

    #[test]
    fn only_the_latest_retired_aliases_are_remembered() {
        // The expectations below are spelled out for a limit of 64.
        assert_eq!(RETIRED_ALIAS_LIMIT, 64);
        let mut aliases = SubscribedAliases::default();
        for alias in 0..=64 {
            aliases.establish(alias, track(b"a"));
            aliases.retire(alias);
        }
        let cases = [
            (0, SubscribedAlias::Unknown),
            (1, SubscribedAlias::Retired),
            (64, SubscribedAlias::Retired),
            (65, SubscribedAlias::Unknown),
        ];
        for (alias, expected) in cases {
            assert_eq!(aliases.state(alias), expected, "{alias}");
        }
    }

    #[test]
    fn events_for_an_unknown_alias_change_nothing() {
        let mut aliases = SubscribedAliases::default();
        aliases.stream_closed(ALIAS);
        aliases.drain(ALIAS, 0);
        aliases.drain_timeout(ALIAS);
        assert_eq!(aliases.state(ALIAS), SubscribedAlias::Unknown);
    }

    #[test]
    fn streams_closed_before_the_alias_is_established_are_counted() {
        // (streams closed early, Stream Count, phase right after PUBLISH_DONE)
        let cases = [
            (0, 1, Phase::Draining),
            (1, 1, Phase::Retired),
            (2, 2, Phase::Retired),
            (1, 2, Phase::Draining),
        ];
        for (closed_early, stream_count, expected) in cases {
            let mut aliases = SubscribedAliases::default();
            for _ in 0..closed_early {
                aliases.stream_closed(ALIAS);
            }
            aliases.establish(ALIAS, track(b"a"));
            aliases.drain(ALIAS, stream_count);
            assert_eq!(
                phase(&aliases),
                expected,
                "{closed_early} early, count {stream_count}"
            );
        }
    }

    #[test]
    fn early_streams_are_held_for_a_bounded_number_of_aliases() {
        // The expectations below are spelled out for a limit of 64.
        assert_eq!(EARLY_CLOSED_ALIAS_LIMIT, 64);
        let mut aliases = SubscribedAliases::default();
        for alias in 0..=64 {
            aliases.stream_closed(alias);
        }
        // (alias, phase after it is established and PUBLISH_DONE counts 1)
        let cases = [
            (0, Phase::Draining),
            (1, Phase::Retired),
            (64, Phase::Retired),
        ];
        for (alias, expected) in cases {
            aliases.establish(alias, track(b"a"));
            aliases.drain(alias, 1);
            let state = match aliases.state(alias) {
                SubscribedAlias::Draining(_) => Phase::Draining,
                SubscribedAlias::Retired => Phase::Retired,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(state, expected, "{alias}");
        }
    }

    #[test]
    fn a_late_stream_of_a_retired_alias_is_not_counted_for_the_next_subscription() {
        let mut aliases = SubscribedAliases::default();
        aliases.establish(ALIAS, track(b"a"));
        aliases.retire(ALIAS);
        aliases.stream_closed(ALIAS);
        aliases.establish(ALIAS, track(b"a"));
        aliases.drain(ALIAS, 1);
        assert_eq!(phase(&aliases), Phase::Draining);
    }

    #[test]
    fn an_assigned_and_unused_alias_is_remembered_as_retired() {
        let mut aliases = SubscribedAliases::default();
        aliases.retire_if_unknown(ALIAS);
        assert_eq!(aliases.state(ALIAS), SubscribedAlias::Retired);

        // An alias a subscription holds is left alone.
        aliases.establish(8, track(b"a"));
        aliases.retire_if_unknown(8);
        assert_eq!(aliases.state(8), SubscribedAlias::Established(&track(b"a")));
    }
}
