# The relay compares an Object that comes again with what its cache holds

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay's cache (an Object that comes again with other content, and a Group that contradicts itself)

## Objective

The relay now compares every Object it is about to pass on with what its cache holds of the Object's Group. It ends the track as malformed where the draft names the case, and gives the Group up in its cache where the draft allows what the publisher did ([notes](../moqt-implementation-notes.md), row on an Object that comes again). This changes what the relay does with Objects it passed on before. The record says how much, and in which sequences of events.

No publisher at hand sends such Objects: the Rust client and the pages send every Object once, and the `moqtail` clients of `interop/` send Object 0 of each Group. So nothing here ran over a connection. The comparison is between two versions of the relay core fed the same events.

## Environment

`spikes/relay-core-diff/compare.py`, which builds the relay core of two revisions into one crate and feeds both the same events: before, the parent of the working copy (the cache without the comparison); after, the working copy this record is part of. macOS.

## Commands

```sh
for world in contradictions split-contradictions shared-contradictions full fetch; do
  python3 spikes/relay-core-diff/compare.py @- disk --world $world --kinds --touch contradictions
done
```

The three worlds named `contradictions` are new. In them one Object in four of the random events has another payload, one in six is an End of Group status, and one in five of the rest carries a Prior Object ID Gap; with four Object IDs and three Groups the same Location comes again and again. `full` and `fetch` are worlds the tool had: every Object has the same payload, and the streams of a Group are of several Subgroups. `--touch contradictions` counts as an event the change is about: an Object at a Location (Group and Object ID, on any track) where an Object came before in the sequence, an Object that is a status or has a property, and any Object of a Group in which one of those came before.

Each run is 20,000 sequences of 80 events.

## Observed results

| World | Sequences with an event the change is about | Sequences in which the versions differ | Of those, before such an event | Events in which sessions outside the change were treated differently |
|---|---|---|---|---|
| `contradictions` | 11,476 | 304 | 0 | not applicable |
| `split-contradictions` | 5,410 | 79 | 0 | 0 of 332,022 actions |
| `shared-contradictions` | 5,410 | 79 | 0 | 0 of 295,705 actions |
| `full` | 7,787 | 70 | 0 | not applicable |
| `fetch` | 4,719 | 80 | 31 | not applicable |

The sequences that differ, by their first difference:

| World | The new version ends the track as malformed | The actions are the same and what the relay holds is not (a Group the cache gave up) | The new version leaves out a second `AbortFetchWrite` for one FETCH |
|---|---|---|---|
| `contradictions` | 220 | 84 | 0 |
| `split-contradictions` | 55 | 24 | 0 |
| `shared-contradictions` | 55 | 24 | 0 |
| `full` | 70 | 0 | 0 |
| `fetch` | 47 | 0 | 33 |

- The Objects of `contradictions`: 51,474 as they were, 17,345 with another payload, 16,097 End of Group, 12,006 with a gap.
- **Ended as malformed**: on an Object in all but one sequence of each `contradictions` world, where it is on a PUBLISH that lets the relay read streams it held. The old version sent the Object on, or did nothing with it. The tool does not say by which of the rules a track ended.
- **`full` and `fetch` have no status, no gap and one payload, and 70 and 47 of their sequences end a track all the same.** Two streams of different Subgroups of one Group bring the same Object ID there, each with a Subgroup ID the relay is sure of: a second Object at a Location with another Subgroup ID (§9.1). The tool makes such streams at random; no publisher of `interop/` does.
- **The 31 sequences of `fetch` that differ before an event the change is about** are among the 33 of the last column: the working copy also tells the I/O side once, and not twice, to give up a fetch stream it writes (a cancellation that is reported as two ends of the request stream, or a track found malformed after a cancellation). That is another change than the comparison, and `--touch contradictions` does not count its events.
- The tests of `xmoq-relay-core` and `xmoq-relay` ran in each: 248 (the 247 of the crate and the random events) and 70 passed.

What the comparison does to publishers that follow the draft was read by a reviewer, with sequences run against the working copy, and two cases ended a track that should go on. Both are changed, and are tests now:

- A Subgroup sent again from its middle on a stream that leaves the Subgroup ID to its first Object (Objects 4 and 5, a reset, and Objects 5 and 6 on a stream without FIRST_OBJECT): the relay read 5 as the Subgroup ID of the second stream. Such a Subgroup ID is not compared any more.
- A status above an End of Group status (an End of Track, or a second End of Group): no longer malformed.

Nineteen changes to the comparison and to the telling once were tried against the tests of `xmoq-relay-core`; eighteen made a test fail. The one that did not (an Object at the very Object ID of an End of Group counted as one after it) changes nothing: that Object ID is taken by the status, which is looked up first.

## Limitations and blockers

- **Nothing ran over a connection.** No client here can send an Object twice, a status, or a Prior Object ID Gap on a stream of its own choosing.
- One sequence in 60 to 300 differs: few of the random sequences get an Object to a subscriber at all.
- The numbers above are of the second version of the comparison, after the review; the first was run the same way, and differed in 311, 80, 80, 70 and 47 sequences.
- The first run of `full` counted one sequence that differed before an event the change is about. It was the counting: a stream number that came with a second header while the first stream was open was taken for a stream of the second header's Group. With every header of a stream number counted, it is 0.
- The tool did not take the `fixtures/` directory along, which a test of `xmoq-relay` reads since the table of how a served FETCH ends, nor the cache module of the old revision. Both are in the tool now.

## Decision or next action

The comparison is in. What it does not find (other Groups than the latest, tracks that are not cached, an Object after an End of Track) is among the known gaps of the notes.
