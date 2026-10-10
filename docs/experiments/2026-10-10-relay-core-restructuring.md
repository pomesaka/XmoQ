# The relay core before and after its restructuring answer the same events alike

- Date: 2026-10-10
- Commit SHA: the commit that adds this record, change `zvonrzssxuzz`. It was compared three times as it grew; the table under "Which version each check was run on" says which check saw which version
- Roadmap item: Phase 0b, take one track from several publishers; the restructuring prepares it and is meant to change no behaviour

## Objective

`crates/relay-core` was restructured: one map for the tracks that wait for a publisher's answer and the ones that have a source, the draining state moved to the source, one lookup for the relay's own requests, long functions split. The claim is that `Relay::handle` returns the same actions for the same events as before. The tests of the crate pass unchanged, but they only cover the sequences someone thought of.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Code | `xmoq-relay-core` and `xmoq-relay` 0.0.0 at the two commits above |
| Peers of the runs with real peers | `xmoq-moqt-client`; the moqtail Rust client (`moqtail` crate 0.15.0); moqtail-ts (`moqtail` npm 0.13.0) in Chromium 156.0.8078.4 through `playwright` 1.64.0 |

## Method

`spikes/relay-core-diff/compare.py <before> <after>` builds a relay core that holds both versions: `lib.rs` and `routing.rs` of the revision before become a module next to the code of the revision after, and `Relay::handle` feeds every event to both and compares the two lists of actions. `events.rs` is that of the revision after and is shared, which works because the restructuring did not touch it. A difference is counted as *content* when the two lists differ as multisets, and as *order* when they hold the same actions in another order. In tests a content difference panics.

What is fed through it:

1. The tests of `xmoq-relay-core` (59) and of `xmoq-relay` (32), which drives the core through its hub on the in-memory connection.
2. Random events, `spikes/relay-core-diff/fuzz.rs`: 20,000 sequences of 80 events from fixed seeds. An event is one of 25 kinds, about one of 4 sessions, 2 tracks in one namespace, 2 Track Aliases, 4 data streams a session, 6 requests of the relay and 12 streams to subscribers. Request IDs and stream numbers of a session go up as a real peer's would, so most requests are valid; the rest of the choices are free, so many events are about things the relay does not know.
3. With `--spikes`, the 12 scenarios of `spikes/moqtail/browser/run.mjs` against a relay built that way: routing by namespace, a PUBLISH that crosses the relay's SUBSCRIBE, and a moqtail-ts publisher.

`--mutate` checks the tool: it changes one decision of the revision after (the Error Code after an upstream timeout) and the run has to report content differences.

## Commands

```sh
python3 spikes/relay-core-diff/compare.py main zvonrzssxuzz --spikes
python3 spikes/relay-core-diff/compare.py main zvonrzssxuzz --mutate
```

For every event the tool compares, in this order, and counts the first that differs: *content*, the actions as a multiset; *state*, `Relay::counts()` after the event; *sequence*, the actions for each session in order, leaving out `StopDataStream` and leaving out events that drop a session; *order*, the whole list in order. The comparisons of state and sequence were added after the first two versions had been compared.

## Which version each check was run on

The restructuring was compared in three states: the first version (commit `cf8602633717`, against `main` at `604236b9be84`); the version after the review asked for requests that are kept apart from sources (the working directory at that time); and the final version, which is the one this record is committed with (`ba68fa2439ab` when it was compared, against `main` at `902a4afe94c6`). `relay-core` did not change on `main` in between.

| Check | First version | After the review | Final version |
|---|---|---|---|
| Tests of both crates and 1,600,000 random events, content | 0 differences (order 347 by hand, 371 with the tool) | 0 (order 368) | 0 (order 354, 358 and 361 in three runs) |
| The same, state and sequence | not compared | not compared | 0 and 0 |
| The 12 scenarios with real peers, content | 0 | 0, on the version before its last comments were added | 0 |
| `mise run interop`, 45 pairings, twice, content | 0, by hand | not run | **not run** |
| The relay's log before and after, three runs each | compared (below) | not run | **not run** |
| `--mutate` | 11 content differences, 7 failed tests | not run | not run |
| `mise run check` and `mise run interop` without the comparison | passed | passed | passed |

The interop pairings with the comparison were made by hand, by making the comparing build the default for the time of the run. The tool cannot repeat that: the interop runner takes the relay from `target/debug`.

## Observed results

**For no event did the two versions return a different set of actions, or hold something different afterwards.** In 354 to 371 of the 1,600,000 random events, depending on the run, they returned the same actions in another order.

| Fed through both versions (final version) | Content | State | Sequence | Order |
|---|---|---|---|---|
| Tests of `xmoq-relay-core`, 59 | 0 | 0 | 0 | 0 |
| Tests of `xmoq-relay`, 32 | 0 | 0 | 0 | 0 |
| 1,600,000 random events | 0 | 0 | 0 | 358 |
| The 12 scenarios with real peers | 0 | not compared | not compared | 0 |

The 12 scenarios were run on the final version with the tool as it was before it compared state and sequence.

### The order differences

The tool sorts an order difference into one of two places by what the event and its actions look like: a session is dropped (the event is `SessionClosed`, or an action is `CloseSession`), or the actions hold two or more `StopDataStream`. Every one of them fell into one of the two, and none was left over. That is the tool's sorting, not a proof; what backs it is the *sequence* comparison, which found that outside the events that drop a session nothing but the place of `StopDataStream` actions differed.

In both places, in both versions alike, the core walks a `HashMap` and pushes an action per entry. The order of a `HashMap` is random per process, so two cores in one process differ here although their code is the same, and one core differs from run to run.

| Place | What has no fixed order | Of 358 |
|---|---|---|
| `release_source`, when a track ends | The `StopDataStream` actions for the streams of the track's source, when two or more are open | 265 |
| `drop_session`, when a session goes | The tracks the session was the source or a subscriber of, when there are two or more: the PUBLISH_DONE and the resets for one track before or after those of another | 93 |

No test of either crate reaches one of the two with more than one entry, which is why the tests show no order difference.

### The relay's log before and after

This was done for the first version only. The relay's log lines of the 12 scenarios and of the interop pairings that go through the own relay (41 logs) were collected three times with the code before and three times after, and compared after removing the numbers that differ per run.

| Compared | Lines on one side only |
|---|---|
| In the three runs after, never in the three before | 8 |
| In the three runs before, never in the three after | 8 |
| In one run before, never in the other two before | 8, 3, 2 |
| In one run after, never in the other two after | 2, 4, 5 |

The 8 lines seen only after are all `CONNECTION_ENDED` lines of another kind (the peer closed first, or the relay did), which the I/O side reads from the connection. The same code differs from run to run by as much, so this comparison says no more than that it found no difference.

## Limitations and blockers

No blocker.

- **Three kinds of event are never fed**: a control message (`ControlMessage`), a session that failed on the I/O side (`SessionFailed`), and a SUBSCRIBE_OK with a Mandatory Track Property, which leads to `UnsupportedTrack`. And some states are reached seldom: a timeout while the relay waits for a PUBLISH that a publisher announced with `DUPLICATE_SUBSCRIPTION` happened 4 times in the 1,600,000 events (counted by the reviewer of this change, not by the tool), so that the two versions agree there rests on 4 events.
- **The random events are a small world.** Four sessions, two tracks, one namespace (and the empty one), two Track Aliases. No Object carries properties, no PUBLISH carries LARGEST_OBJECT, no message is malformed at the level of bytes (the core gets decoded messages). States that need more than 80 events to reach, more than 4 sessions, or more than 64 of something (the limits on waiters, namespaces and own requests) are not reached.
- A sequence is not required to be one the I/O side can produce: an event may name a stream that was never opened. That widens what is compared, and both versions ignore such events alike, but it also spends events on nothing.
- The 12 scenarios and the interop pairings are short runs with one or two Objects.
- The comparison is between two versions of the core only. The hub was not changed and is not compared.
- The count of order differences is not repeatable; the counts of content, state and sequence differences, 0, are.
- The final version was not run through the interop pairings with the comparison, and its log was not compared; see the table of versions.
- The log comparison used three runs a side; a rare line could be missing from all three on one side by chance.

## Open questions raised

- Should the two places be given a fixed order (stream number, track name)? What a peer sees there, the order of STOP_SENDING on different streams and of PUBLISH_DONE on different request streams, has no guaranteed order on the wire either, but a relay that acts the same for the same input is easier to compare and to debug.
