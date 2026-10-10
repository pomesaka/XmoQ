# The relay finds a Subgroup that comes again with another Publisher Priority

- Date: 2026-10-11
- Commit SHA: not fixed when this was written. Compared with `main` at `01e3a89fc5e6`
- Roadmap item: Phase 0b, several publishers of one track in the relay: what two publishers send for one Subgroup is compared, as far as the header goes

## Objective

**This changes what the relay does for a track with one publisher as well.** §2.4.2 has as its first example of a malformed track "An Object with a particular Subgroup ID is received, but its Publisher Priority is different from that of the previous Object with the same Subgroup ID". Until now the relay did not look: a second stream for a Subgroup passed, took the place of the first or was left, by the rules for the same source and for another, whatever its priority.

What changes:

1. **A stream for a Subgroup the relay remembers, with another Publisher Priority than the stream the Subgroup was taken from, makes the track malformed.** Every subscription ends with PUBLISH_DONE `MALFORMED_TRACK`; the stream is stopped with `MALFORMED_TRACK`, its source is cancelled with that code and every other source with `CANCELLED`. From the same source or another, and whether the first stream is open, was cut short or ended with a FIN.
2. The priority that is compared is the one in the header, or the default of the source where the header has none: the DEFAULT PUBLISHER PRIORITY of its PUBLISH or SUBSCRIBE_OK (§12.4), and 128 without one. A stream has the default of the source it came from when it opened.
3. A stream without a priority that arrives under a Track Alias whose subscription drains is not compared: it may be one sent ahead of a PUBLISH under that alias, with another default.

Nothing else of the header is compared, and nothing of the Objects.

## Environment

macOS 14.4 (23E214), arm64, loopback only. `xmoq-relay` and `xmoq-moqt-client` 0.0.0. The client has a new flag, `--publisher-priority <0..255|default>`; without it the streams have priority 0 in the header, as before.

## Commands

```sh
mise run check
mise run interop

C="python3 spikes/relay-core-diff/compare.py main disk"
for world in full one-publisher split-one shared; do
  $C --world $world --random-only
done
for world in full one-publisher split-one shared; do
  $C --world $world --priorities --touching --touch priority
done
$C --world shared --priorities --touching --touch priority --raw-numbers  # has to fail
python3 spikes/relay-core-diff/compare.py --same disk --world full --priorities

R="node spikes/moqtail/browser/run.mjs --relay own --direction rust-to-rust --rust-publisher own --rust-subscriber own"
$R --priorities differs
$R --priorities written-two-ways
```

The random subgroup streams all had priority 0 until now, so that nothing of this step was reached. `--priorities` is new: a stream has priority 0, 1 or none, by the numbers of the stream and its Group, so that the sequences are otherwise the ones they were; in the worlds with two halves only the streams of the first half vary. An event the change is about (`--touch priority`) is a subgroup stream of a Group that a stream was opened for before with another priority, whatever the track and the Subgroup.

What was expected was written down before the change was made: with one priority throughout, no random sequence that differs, in any world; with three, none that differs before an event the change is about, some that differ, also with one publisher, and nothing different for the bystanders.

## Observed results

**Passed.**

| World | Priorities | Sequences | With an event the change is about | In which the versions differ | Differ before such an event | Events in which the bystanders were treated differently (actions for them) |
|---|---|---|---|---|---|---|
| `full` | one | 20,000 | not looked at | 0 | | no bystanders |
| `one-publisher` | one | 20,000 | not looked at | 0 | | no bystanders |
| `split-one` | one | 20,000 | not looked at | 0 | | 0 (345,034) |
| `shared` | one | 20,000 | not looked at | 0 | | 0 (307,814) |
| `full` | three | 20,000 | 18,377 | 2,569 | 0 | no bystanders |
| `one-publisher` | three | 20,000 | 18,377 | 2,047 | 0 | no bystanders |
| `split-one` | three, first half | 20,000 | 17,896 | 1,059 | 0 | 0 (345,033) |
| `shared` | three, first half | 20,000 | 17,896 | 1,059 | 0 | 0 (307,813) |

In `shared` with three priorities, `--raw-numbers` reports 406 events with a difference for the bystanders: the check can fail. `--same`, world `full` with three priorities: `bcba4cbbf2fac176` in three runs.

### Tests

159 tests of the core. Added in `tests/sources.rs`: another source sends the Subgroup with another priority (the whole list of actions); the six cases of how the first stream stands and who sends the second; the priority is the header's or the default of the source (ten rows, among them the same priority written two ways, a header that overrides a default, and a default above 255); a header that leaves the Subgroup ID to the first Object; one publisher that sends a Subgroup again; a stream that is open while its alias goes to a subscription with another default, which keeps the default it came with (two tests, one of them a Subgroup that was cut short and goes on), and a Subgroup sent again under the new default, which makes the track malformed; a stream without a priority under an alias that drains, which is not compared; the two sources of one session, each with its default; a publisher that accepted the relay's SUBSCRIBE with a default; a Subgroup the relay has forgotten, which is not compared; a late stream that is read with the default of the subscription that took its alias, and a stream sent ahead of a PUBLISH that is read before the PUBLISH_DONE, each of which ends a track nothing is wrong with; and a Subgroup without a priority that reaches the subscriber under the default of another source, which was so before (four known gaps, pinned by tests whose names begin with `known_gap_`).

No test that was there changed: none sent a Subgroup with two priorities.

**Found while the tests were written:** where the other priority is seen in the header, the stream that brought it was not stopped, since what ends a track stops the streams it took Objects from and this one had been left. It is stopped now, with `MALFORMED_TRACK`.

### With real peers

Own Rust clients: two publishers that send Group 0 one after the other, and a subscriber.

| Run | The subscriber | The relay to the PUBLISH stream of the publisher that sent second | To that of the first |
|---|---|---|---|
| `differs`: priorities 0 and 1. 3 runs | PUBLISH_DONE with status `0x12`; its subscription ended after one Object | reset, `0x12` | reset, `0x1` |
| `written-two-ways`: 128 in the header, and none. 3 runs | stays subscribed and gets the next Group of the first publisher | nothing | nothing |

In all three runs of `differs` the second publisher's stream arrived after the first had ended with a FIN. The scenarios of the steps before were run once each and ended as before.

`mise run check` ended with `OK` for all four parts, and `mise run interop` with `66 of 66 pairings ended as expected`, run with nothing else of this working copy going on and not across a sleep of the machine.

## Limitations and blockers

No blocker.

- **`--touch priority` takes about nine in ten sequences for ones the change is about** (18,377 and 17,896 of 20,000), so "differs before such an event" says little. What holds is the four rows with one priority, where nothing differs, and the bystanders.
- **The priority is compared only for a Subgroup the relay remembers**: the 8 newest Groups and 256 Subgroups of a track.
- **A track that nothing is wrong with can still be ended.** A publisher publishes anew under the same alias with another default, leaves the priority out of its headers, and a stream of the first subscription that sends a Subgroup again arrives after the PUBLISH: the relay reads it with the new default (implementation notes, known gaps). The same happens with a stream of the new subscription that is read before the PUBLISH_DONE of the first, which then has the default of the first. Found in review, with the two cases next to them that were mended: a stream that is open across such a PUBLISH, and one that is read before it.
- **Which of the two publishers is wrong the relay does not know.** The one whose stream came second is told `MALFORMED_TRACK`, and with it any publisher can end a track for all its subscribers by sending one stream (implementation notes, known gap on authorization).
- **A Subgroup without a priority still reaches the subscribers under the default of another source** where two sources have different defaults; the relay compares with the right default and passes the header on as it came (implementation notes, known gaps).
- The second and third example of §2.4.2, a Subgroup on two streams with different last Objects, are not looked for: the second stream is stopped unread. What the Objects hold is not compared.
- The clients do not look for any of this (read, in both implementations).
- The runs with real peers had own Rust clients only; in none of them was the first stream open or cut short when the second came. Those cases are covered by tests of the core.
- **The comparison can report more differences for the bystanders than there are.** A random event that lets a wait after PUBLISH_DONE run out carries the number the new version gave that wait. The numbers count up over the whole relay, so where the two versions start a different number of waits for the half the change is about, the same wait of a bystander has two numbers, and the older version drops the event the new one acts on. It cannot make the comparison report fewer. Here it reported none.
- The comparison covers the core. The hub was not changed.
