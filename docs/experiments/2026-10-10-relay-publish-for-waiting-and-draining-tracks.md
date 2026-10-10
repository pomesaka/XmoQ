# The relay takes a PUBLISH for a track that waits for an answer or drains

- Date: 2026-10-10
- Commit SHA: not fixed when this was written. Compared with `main` at `05dff21d6e3b`
- Roadmap item: Phase 0b, take one track from several publishers. The fourth of the steps

## Objective

After [the step before](2026-10-10-relay-several-publish-sources.md) a PUBLISH was still refused in three cases. This step takes it in two of them.

1. **A track the relay has asked a publisher for and has no answer about.** A PUBLISH from another session is accepted: the track has its source, and the subscribers that waited get SUBSCRIBE_OK. What the relay asked the first publisher stays asked.
2. **A track whose sources have all said PUBLISH_DONE**, while their last streams are awaited. A PUBLISH from another session is accepted, and the track goes on for the subscribers it has.

With the first, a track that has a source can have a SUBSCRIBE of the relay that is not answered yet, which was not so before. What becomes of it:

| The publisher that was asked | The relay |
|---|---|
| accepts (SUBSCRIBE_OK) | has a further source. Where the track has 4 already, it cancels the SUBSCRIBE and stops the streams that came ahead of the answer |
| refuses, does not answer in 5 seconds, or goes | lets go of the request. No subscriber is told: the track has a source |
| answers `DUPLICATE_SUBSCRIPTION` | waits 5 seconds for the publisher's PUBLISH, as for a track without a source |
| sends PUBLISH for the track | cancels its SUBSCRIBE, which gives way (§5.1), and has the PUBLISH as a further source |

And when the last subscriber of the track goes, what is still asked is cancelled.

The third case stays: a publisher that said PUBLISH_DONE for a track and publishes it anew while the relay still waits for its streams is refused with `UNINTERESTED`.

The step was made in two parts, a change of the code's shape that alters nothing and then the change of behaviour, and each was compared on its own.

## Environment

macOS 14.4 (23E214), arm64, loopback only. `xmoq-relay` and `xmoq-moqt-client` 0.0.0. The client has a new flag, `--serve-delay-ms N`, for how long a SUBSCRIBE it serves waits for its SUBSCRIBE_OK.

## Commands

```sh
mise run check
mise run interop

# Part 1, the shape: every world, any difference fails the run.
for world in v1 one-publisher full split-one shared; do
  python3 spikes/relay-core-diff/compare.py main disk --world $world
done

# Part 2, the behaviour.
T="--touching --touch waiting-or-draining"
for world in one-publisher full split-one shared; do
  python3 spikes/relay-core-diff/compare.py main disk --world $world $T
done
python3 spikes/relay-core-diff/compare.py main disk --world shared $T --raw-numbers  # has to fail
python3 spikes/relay-core-diff/compare.py --same disk --world full

# A publisher that is asked and answers late, and another that publishes meanwhile.
node spikes/moqtail/browser/run.mjs --relay own --direction rust-to-rust \
  --rust-publisher own --rust-subscriber own --publish-while-asked
```

An event the change is about (`--touch waiting-or-draining`) is a PUBLISH that comes after another session announced a namespace, or after any PUBLISH_DONE. That is wider than the two cases, which only makes the check stricter.

The run with real peers: an own publisher announces the namespace and is started with `--serve-delay-ms 4000 --first-group 1`. An own subscriber subscribes, and the driver waits for the relay's `OPEN_REQUEST`. A second own publisher sends PUBLISH for the track and, when asked, Group 0. The first publisher answers after its 4 seconds and then sends Group 1.

## Observed results

**Passed.**

### Part 1: the shape

`on_publish` hands the decision to a function of its own (`take_publish`), the two places that took LARGEST_OBJECT out of a list of parameters use one function, and the relay's SUBSCRIBE gives way after the source is added instead of before.

| World | Events | Content | State | Sequence | Order | Bystanders |
|---|---|---|---|---|---|---|
| `v1` | 1,600,000 | 0 | 0 | 0 | 0 | none in this world |
| `one-publisher` | 1,600,000 | 0 | 0 | 0 | 0 | none in this world |
| `full` | 1,600,000 | 0 | 0 | 0 | 0 | none in this world |
| `split-one` | 1,600,000 | 0 | 0 | 0 | 0 | 0 |
| `shared` | 1,600,000 | 0 | 0 | 0 | 0 | 0 |

The 101 tests of the core and the 33 of the relay passed under the comparing build. This was run on the code as it was between the two parts; no revision holds that state.

### Part 2: the behaviour

| World | Sequences | With an event the change is about | In which the versions differ | Differ before such an event | Events in which the bystanders were treated differently (actions for them) |
|---|---|---|---|---|---|
| `one-publisher` | 20,000 | 18,440 | 0 | 0 | no bystanders |
| `full` | 20,000 | 19,660 | 1,784 | 0 | no bystanders |
| `split-one` | 20,000 | 19,640 | 344 | 0 | 0 (345,323) |
| `shared` | 20,000 | 19,589 | 352 | 0 | 0 (308,099) |

In `one-publisher` the versions did the same in every event, as was to be expected before the run: one session is the only one that announces and publishes there, so no PUBLISH comes from a session other than the one that was asked, and a PUBLISH of that session for its own draining track is refused in both versions, for another reason in the new one and with the same messages.

In `shared`, with `--raw-numbers` the same run reports 7 events with a difference for the bystanders: the check can fail.

`--same`, world `full`: `59ca32ae587afc75` in three runs in processes of their own.

### Tests

109 tests of the core; ten were added and three changed.

- Added, in `tests/routing.rs`: the eight ways what was asked can end while the track has a source (a table); an answer that comes when the track has 4 sources; the last subscriber going while something is asked; the track ending with its last source while something is asked; two subscribers that waited, one with Forward State 0. In `tests/sources.rs`: a track whose source drains and that goes on with a publisher that comes. After the review, four more: a timer for what was asked that fires late; a malformed track that has something asked; a publisher that came to a draining track and goes first; a publisher that takes over a draining track and counts its Groups from 0.
- Changed: the test that had a PUBLISH for a waiting track refused became the first of the above. Two tests that needed a PUBLISH that is refused, and had used one for a draining track, use the case that is left: a publisher that publishes anew while its own source drains.

### With real peers

Three runs, the same in each: the subscriber had no SUBSCRIBE_OK when the second publisher's PUBLISH was sent, and had it right after, before the first publisher had answered; it got Group 0 from the second publisher once, and Group 1 from the first once that one had answered.

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `46 of 46 pairings ended as expected`.

## Limitations and blockers

No blocker.

- **Of the table above, only the first row was run with real peers**, and of the two cases only the first. A publisher that refuses, does not answer, answers `DUPLICATE_SUBSCRIPTION` or goes while another has published, the limit of 4, and a PUBLISH for a draining track are covered by the tests of the core and the random events. A draining track lasts as long as a counted stream is under way, which a real client does not hold open on purpose.
- **A publisher that takes over a draining track has to go on with its Group IDs.** One that counts from 0 where the old source was at Group 50 is accepted and has every stream stopped until it reaches Group 43. That follows from the step before (a Group that is 8 or more behind the newest is stopped once a track has had several sources) and is new in effect here: such a PUBLISH used to be refused. A test of the core holds it; it was not run with real peers.
- A SUBSCRIBE for a track whose one source drains is refused with `DOES_NOT_EXIST` also while the relay waits for an answer it asked for earlier. The subscriber is not held for that answer.
- The run with real peers had own clients only, and no browser.
- The comparison of part 1 was run on a state of the code that no revision holds.
- The random events of `full`, `split-one` and `shared` differ between the versions in a part of the sequences, by design; that the differences are the intended ones is shown for the first in each sequence, and for the bystanders throughout.
- The comparison covers the core. The hub was not changed.
