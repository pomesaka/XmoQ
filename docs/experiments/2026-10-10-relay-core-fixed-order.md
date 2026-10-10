# The relay core returns the same actions in the same order for the same events

- Date: 2026-10-10
- Commit SHA: the commit that adds this record. Compared with `ba68fa2439ab`, the restructured core before this change
- Roadmap item: Phase 0b, take one track from several publishers; this is a preparation, so that runs of the relay can be compared

## Objective

[The comparison of the core before and after its restructuring](2026-10-10-relay-core-restructuring.md) found two places where the order of the actions depended on the run: the core walked a `HashMap` and pushed an action per entry. They now go by stream number and by track name. Two things are to be shown: that the same events lead to the same actions in the same order from run to run, and that nothing but the order changed.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64 |
| Code | `xmoq-relay-core` 0.0.0 |

## What was changed

| Place | Order now |
|---|---|
| `release_source`: the `StopDataStream` actions for the open streams of a source whose track ends | By stream number |
| `drop_session`: the tracks a session that goes was the source of, and the ones it subscribed to | By track name, as the tracks it waited for already were |

No other place of the core pushes actions while walking a `HashMap` or a `HashSet`, or picks "the first" of one: the remaining walks only count or look up (`cut_short` in `end_track`, the out streams of a session in `drop_session`), collect into a sorted set first (`drop_routing_of`), or go over a `Vec`, a `VecDeque` or a `BTreeSet`, whose order is the order of arrival or of the key. `matching_publishers` goes over the announcements in the order they were made.

## Commands

```sh
python3 spikes/relay-core-diff/compare.py --same ba68fa2439ab   # before
python3 spikes/relay-core-diff/compare.py --same disk           # after, the working directory
python3 spikes/relay-core-diff/compare.py ba68fa2439ab disk
mise run check
mise run interop
```

`--same` runs 1,600,000 random events (20,000 sequences from fixed seeds) through one version, three times in processes of their own, and prints a digest of every list of actions the relay returned, in order.

## Observed results

**Passed.**

| Version | Digest of the three runs | |
|---|---|---|
| Before | `0c1a6bc7dec81c93`, `0235e52ac23231c8`, `90dc1e02d270d59a` | three different orders; the tool ended with `DIFFERENT: the order of the actions depends on the run` and exit code 1 |
| After | `310423e486b57a5a`, three times | the same actions in the same order; `SAME: the three runs returned the same actions in the same order` |

Side by side with the version before, the version after differed in the content of no list of actions, and in the order of 381 (284 where a track ends with several streams of its source open, 97 where a session is dropped that has to do with several tracks): the two places, and no other.

Three tests of `xmoq-relay-core` now have an order as their expectation, each with enough entries that an order left to chance would pass once in 120 or 720 runs: six open streams of a track that ends, five tracks of a publisher that goes, five tracks the relay subscribed to for a subscriber that goes.

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `45 of 45 pairings ended as expected`.

## Limitations and blockers

No blocker.

- The random events reach the two places with two tracks and four data streams a session at most, and cover what the record linked above lists; a place that orders by a `HashMap` only in a state they do not reach would not show in the digest. The claim that there is no such place rests on reading the code.
- This is about the core. The order in which the hub carries out actions on different streams of a connection, and the order in which a peer sees them, are not fixed by it: QUIC gives none between streams.

## Open questions raised

None.
