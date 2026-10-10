# The relay core keeps the sources of a track in a list

- Date: 2026-10-10
- Commit SHA: not fixed when this was written. Compared with `main` at `600a645687b1`
- Roadmap item: Phase 0b, take one track from several publishers. The second of the steps: the shape for several sources, with one source per track still

## Objective

A change that is meant to alter nothing the relay does. A track kept one source in a field; it keeps a list now, into which one source is put. What read the field reads the list in a way that holds for several: the source on a session, every source drained, every source a SUBSCRIBE of the relay. The two places that made a track with a source go through one (`Tracks::add_source`), and the four places where one source ends without having drained go through one (`Relay::source_gone`), which ends the track as before.

The step after this one takes a second source, and is then a change of behaviour only.

## Environment

macOS 14.4 (23E214), arm64, loopback only. Peers of the 12 scenarios as in [the record of the Subgroup selection](2026-10-10-relay-subgroup-selection.md).

## Commands

```sh
mise run check
mise run interop

# The core of the working directory next to the one of the commit before it.
python3 spikes/relay-core-diff/compare.py @- disk --world v1
python3 spikes/relay-core-diff/compare.py @- disk --world full --spikes
python3 spikes/relay-core-diff/compare.py @- disk --world split
python3 spikes/relay-core-diff/compare.py --same disk --world full
```

Without `--touching`: any event in which the two versions differ in content, state or sequence fails the run.

## Observed results

**Passed.**

| World | Events | Content | State | Sequence | Order | Tests of the two crates under the comparing build |
|---|---|---|---|---|---|---|
| `v1` | 1,600,000 | 0 | 0 | 0 | 0 | 82 and 33 passed |
| `full` | 1,600,000 | 0 | 0 | 0 | 0 | 82 and 33 passed |
| `split` | 1,600,000 | 0 | 0 | 0 | 0 | 82 and 33 passed |

The 12 scenarios with real peers against a relay that holds both versions: no difference of any kind, and the payload matched in all 12.

`--same`, world `full`: `c231e5c9a6f705f8` in three runs, the digest the version before had.

No expectation of a test was changed, and no test was added. `mise run check` ended with `OK` for all four parts (81 tests of the core), `mise run interop` with `46 of 46 pairings ended as expected`.

## Limitations and blockers

No blocker.

- **Nothing in this step puts a second source into the list**, so what was written for several sources ran with one only: no test and no random event reaches `Track::source_on` with two sources, a track of which one source drains and another does not, or `Tracks::add_source` for a track that has a source (which is refused with a debug assertion). The next step is where those run.
- `Relay::source_gone` ends the track whatever other sources it has. That is marked `FIXME` and is right only while a track has one.
- The comparison covers the core. The hub was not changed.
