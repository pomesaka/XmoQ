# The relay takes the PUBLISH of a publisher whose earlier subscription still drains

- Date: 2026-10-10
- Commit SHA: `065753f3eb3ce5b76e8830a32ff288856a8b8ad2`. Compared with `main` at `eff75aaafcb4`
- Roadmap item: Phase 0b, take one track from several publishers. The fifth of the steps

## Objective

A publisher ends its subscription with PUBLISH_DONE and publishes the track anew, as an encoder that restarts does. For the publisher the first subscription is Terminated (§5.1). The relay still waits for the data streams the PUBLISH_DONE counted, and until now it refused the second PUBLISH for as long as it waited.

What changes:

1. **The PUBLISH is accepted.** A source of a track is told from another by its session and its Track Alias, where it was the session alone. Under another alias the two are two sources of the track; under the same alias the new one takes the place of the old, which is over at once.
2. **A Track Alias that is used again ends the source that had it**, also when the new subscription is to another track, and also when the relay cannot take the new subscription as a source. Before, the relay never let go of that source: the wait for its streams could not end, and its track stayed for good. That was so on `main` before several sources were taken, and was found by reading and with a throwaway test in this step; the last case was found in review.
3. A source that drains counts against the limit of 4 sources.
4. **The timer of a wait is told from the timer of an earlier wait under the same alias.** `StartDrainTimer` and `DrainTimeout` carry a number. Before, the first timer to run out ended whatever drained under the alias then, up to the whole wait early. A throwaway test showed the same on `main`: a subscription drains by its count, the alias is used again, the new subscription drains, and the timer of the first ends it.
5. **A subgroup stream is taken only from a session and alias that are a source of the track.** Before, a late stream under an alias that still drained went into a track of the same name that another publisher had made after the first ended, and stayed in it when its session went. Also so on `main`, by a throwaway test.
6. The streams of a subscription that are open when a new one takes its alias are not counted towards the Stream Count of the new one.
7. A malformed track is held against the source whose alias the stream carries, where it was the session.

The session state was changed before this (a refused PUBLISH leaves a draining subscription its alias), in a step of its own.

## Environment

macOS 14.4 (23E214), arm64, loopback only. `xmoq-relay` and `xmoq-moqt-client` 0.0.0. The client takes two new lines on stdin, `republish` and `republish-same-alias`: it ends its subscription with PUBLISH_DONE and publishes the track anew, under the next Track Alias or the same one. The PUBLISH_DONE has the Stream Count of a publisher that cannot count (§10.11: "it MUST set Stream Count to 2^62 - 1"), though this client can: every stream it sends has ended before the PUBLISH_DONE goes out, and with the exact number the relay would not wait, which is the state this step is about.

## Commands

```sh
mise run check
mise run interop

T="--touching --touch republish"
for world in one-publisher full split-steady shared-steady split-one shared; do
  python3 spikes/relay-core-diff/compare.py main disk --world $world $T
done
python3 spikes/relay-core-diff/compare.py main disk --world shared-steady $T --raw-numbers  # has to fail
python3 spikes/relay-core-diff/compare.py --same disk --world full

R="node spikes/moqtail/browser/run.mjs --relay own --direction rust-to-rust --rust-publisher own --rust-subscriber own"
$R --republish same
$R --republish other
```

An event the change is about (`--touch republish`) is a PUBLISH by a session that sent PUBLISH for the track before or that said PUBLISH_DONE for anything, and a SUBSCRIBE_OK or a subgroup stream of a session that said PUBLISH_DONE. A timer that runs out has the number of the last timer the new version started for the alias, or, every fourth event, of the first; the older version is patched to take the event without the number. `split-steady` and `shared-steady` are `split-one` and `shared` in which the publisher of the second half never says PUBLISH_DONE, so that nothing there can publish anew while it drains.

What was expected was written down before the change was made: no sequence that differs before such an event, in any world; nothing different for the bystanders in the two steady worlds; and something different for them in `split-one` and `shared`, whose second half can publish anew.

## Observed results

**Passed.**

| World | Sequences | With an event the change is about | In which the versions differ | Differ before such an event | Events in which the bystanders were treated differently (actions for them) |
|---|---|---|---|---|---|
| `one-publisher` | 20,000 | 19,959 | 420 | 0 | no bystanders |
| `full` | 20,000 | 19,712 | 535 | 0 | no bystanders |
| `split-steady` | 20,000 | 19,942 | 562 | 0 | 0 (348,799) |
| `shared-steady` | 20,000 | 19,921 | 562 | 0 | 0 (311,451) |
| `split-one` | 20,000 | 19,974 | 1,140 | 0 | 2,101 (345,014), as expected |
| `shared` | 20,000 | 19,964 | 1,140 | 0 | 2,075 (307,797), as expected |

In `shared-steady`, with `--raw-numbers` the same run reports 17 events with a difference for the bystanders: the check can fail. `--same`, world `full`: `6cc0c2e3f2f4b591` in three runs.

**The first run of `full` had one sequence that differed before an event the change is about.** It was read before anything else was done. The event was a SUBSCRIBE_OK to a SUBSCRIBE of the relay, under the Track Alias of a subscription of the same session that drained, for another track: the second change above, reached by SUBSCRIBE_OK where the definition of the events had only PUBLISH. The older version kept the source of the first track; the new one ends it. The definition was widened, and a test of the core was written for that way in. **After the fifth change above, `full` had two such sequences**, both a subgroup stream that the older version took into a track its session was no source of; the definition was widened again, to the subgroup streams of a session that said PUBLISH_DONE.

### Tests

129 tests of the core. Added in `tests/sources.rs`: a publisher that publishes anew under another alias while its source drains; under the same alias; what ends one of the two sources of a session; a session that goes with two sources of a track; the same Subgroup from the two, and one that was cut short and is sent again under either alias; a draining source and the limit; an alias that is used again for another track, by a PUBLISH, by a SUBSCRIBE_OK, and by a SUBSCRIBE_OK the track cannot take; a stream of the subscription before, which is not counted and not waited for, and one that comes late, which is counted (a known gap, pinned); the timer of an earlier wait; a stream of a session that is no source of the track; which source a malformed track is held against; a PUBLISH that takes the place of a draining subscription of the relay. In `tests.rs`: a stream sent ahead for a refused PUBLISH (a known gap, pinned).

Four tests changed, all because a PUBLISH they needed refused is accepted now. One, which had a publisher refused while its source drained, became the first of the above. Three use the refusal that is left, a track with 4 sources. The tests that start or answer a drain timer have its number.

### With real peers

| Run | The publisher's first PUBLISH stream ended with | Second PUBLISH_OK | Group 1 at the subscriber | Subscription ended for the subscriber |
|---|---|---|---|---|
| `--republish same`, 3 runs | a reset, code `0x1`, at once | yes | yes | no |
| `--republish other`, 3 runs | a reset, code `0x1`, when the relay's wait of 2 seconds ran out | yes | yes | no |

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `55 of 55 pairings ended as expected`.

## Limitations and blockers

No blocker.

- **The publisher of the runs with real peers says it cannot count its streams, though it can**, to keep the relay waiting. A publisher whose last stream is in fact still under way when it publishes anew was not run; nothing at hand sends that way.
- **Which of two outcomes a publisher gets that says PUBLISH_DONE and publishes anew at once depends on a race**: accepted for the track as it is when a counted stream is still under way, and a new track, after the subscribers were sent PUBLISH_DONE, when none is. The runs above show the first, by construction. The second is what the relay did before this step and still does.
- **A publisher that publishes anew is expected to go on with its Group IDs, and the relay does not hold it to that.** On the session of the source that drains, one that counts from 0 is heard at once, under the same alias and under another, and the subscribers see the Group IDs go back; on another session it is accepted and not heard until it has caught up (implementation notes, row on a PUBLISH for a track the relay knows already). The own client goes on with its numbers through a `republish`.
- **A stream of the first subscription that comes after the alias was taken is counted for the new one** (implementation notes, known gaps). A test pins it.
- The timers of waits that are over are not stopped: each runs out and is dropped by its number. The hub bounds the timers of a session as before (1,024 at a time), and each lasts the 2 seconds of a wait. A timer asked for beyond the bound runs out at once (implementation notes, known gaps); read, not run.
- The runs had own Rust clients only: no browser, no moqtail peer.
- An alias that is used again for another track is covered by tests of the core and by the random events, not by a run with real peers.
- The change to the session state that came before this step cannot be seen by the comparison, which gives both versions of the core the same session state.
- The comparison covers the core. The hub passes the number of a timer on and is otherwise unchanged; its log lines are the same.
