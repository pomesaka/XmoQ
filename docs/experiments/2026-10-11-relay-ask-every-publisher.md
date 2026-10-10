# The relay asks every publisher of a matching namespace

- Date: 2026-10-11
- Commit SHA: not fixed when this was written. Built on `065753f3eb3ce5b76e8830a32ff288856a8b8ad2`; compared with `main` at `ed70d28082d8`, which differs from that commit in nothing the relay's core and the session state are made of
- Roadmap item: Phase 0b, several publishers of one track in the relay: "send the SUBSCRIBE to every matching publisher" and "subscribe to a publisher that announces a namespace later"

## Objective

§9.5: "Relays MUST send SUBSCRIBE messages to all matching publishers", and "When a relay receives an authorized PUBLISH_NAMESPACE for a namespace that matches one or more existing subscriptions to other upstream sessions, it MUST send a SUBSCRIBE to the publisher that sent the PUBLISH_NAMESPACE for each matching subscription". Until now the relay asked the one publisher that had announced first, and did nothing when a namespace was announced.

What changes:

1. **A SUBSCRIBE for a track without a source goes to every publisher of a matching namespace**, in the order they announced, a session once.
2. **A SUBSCRIBE for a track that has a source goes to them as well**, after its SUBSCRIBE_OK; the draft can be read either way (implementation notes, row on which publishers the relay asks).
3. **A publisher that announces a namespace is asked for every track of it that is subscribed to or waited for.**
4. No more are asked than the track has room for: 4, with its sources and what is asked already. The publisher that sends PUBLISH_NAMESPACE is asked before the ones that were left out earlier. Leaving a matching publisher unasked for want of room is against the MUST and is a known gap, one of five the implementation notes list for this step.
5. A publisher that was asked and did not become a source is remembered by the track, 16 of them, and is not asked again until it announces a namespace of the track anew.
6. A publisher on which the relay has 64 requests open is passed over, where the subscriber was refused when that publisher was the one to ask.

## Environment

macOS 14.4 (23E214), arm64, loopback only. `xmoq-relay` and `xmoq-moqt-client` 0.0.0. The client was not changed.

## Commands

```sh
mise run check
mise run interop

C="python3 spikes/relay-core-diff/compare.py main disk"
$C --world no-announce --random-only
$C --world one-publisher --random-only
for world in full split-one shared split-steady shared-steady; do
  $C --world $world --touching --touch asks
done
$C --world shared --touching --touch asks --raw-numbers  # has to fail
python3 spikes/relay-core-diff/compare.py --same disk --world full

R="node spikes/moqtail/browser/run.mjs --relay own --direction rust-to-rust --rust-publisher own --rust-subscriber own"
$R --ask-all two-announcers
$R --ask-all announcer-and-publisher
$R --ask-all late-announcer
```

`no-announce` is the world `full` in which nobody announces a namespace. In `one-publisher` one session does all the announcing and all the publishing. `--random-only` is new: the exit status is decided by the random sequences alone, since the tests of the crate that were added or changed differ between the versions by design. An event the change is about (`--touch asks`) is any event from the time on when one session has announced a namespace and another has announced one or sent PUBLISH.

What was expected was written down before the change was made: no random sequence that differs in `no-announce` and in `one-publisher`; none that differs before an event the change is about in `full`; and nothing different for the bystanders in the four worlds that have them, in each of which the half of the bystanders has one session that announces and publishes.

## Observed results

**Passed.**

| World | Sequences | With an event the change is about | In which the versions differ | Differ before such an event | Events in which the bystanders were treated differently (actions for them) |
|---|---|---|---|---|---|
| `no-announce` | 20,000 | not looked at | 0 | | no bystanders |
| `one-publisher` | 20,000 | not looked at | 0 | | no bystanders |
| `full` | 20,000 | 19,857 | 11,154 | 0 | no bystanders |
| `split-one` | 20,000 | 19,850 | 4,112 | 0 | 0 (345,034) |
| `shared` | 20,000 | 19,850 | 4,208 | 0 | 0 (307,814) |
| `split-steady` | 20,000 | 19,850 | 4,112 | 0 | 0 (348,805) |
| `shared-steady` | 20,000 | 19,850 | 4,208 | 0 | 0 (311,452) |

In `shared`, with `--raw-numbers` the same run reports 375 events with a difference for the bystanders: the check can fail. `--same`, world `full`: `e49303814f1d25ee` in three runs.

In `no-announce` and `one-publisher` the comparison reports 11 tests of the crate that fail and 22 events that differ in them. Those are the tests this step added or changed, run against the older version too.

### Tests

144 tests of the core. In `tests/routing.rs`, the table of which publisher is asked became one of which publishers are asked, in what order; 3 of its 14 rows changed. Added: a session that announced two namespaces of the track; the first publisher to accept answers the subscriber and the next is one more source; the subscriber is refused when the last publisher has, with that one's code; a publisher that did not become a source is not asked again, until it announces again, and is forgotten with its session; a request the relay cancels itself is not held against the publisher; the room of a track (four cases); a publisher left out for want of room is asked when there is room; a publisher with 64 requests is passed over; a track remembers 16; a publisher that announces is asked for the tracks that are subscribed to, and not for one that has no subscriber or is in another namespace; a session that is a source is not asked when it announces; the publisher that announces is asked before one that was left out earlier, and nobody when the track has no room; a session is a source of a track, asked for it or remembered, and never two of them; 40 sessions that announce, are asked and go leave nothing.

### With real peers

Own Rust clients. Each run has a subscriber that gets Group 0 from one publisher and Group 1 from the other.

| Run | The relay's SUBSCRIBEs | Group 0 and Group 1 at the subscriber |
|---|---|---|
| `two-announcers`: both publishers only announce the namespace. 3 runs | 2, one to each | once each |
| `announcer-and-publisher`: one sends PUBLISH; the other had announced the namespace when the subscriber came. 3 runs | 1, to the one that announced | once each |
| `late-announcer`: one sends PUBLISH, the subscriber comes, then the other announces the namespace. 3 runs | 1, to the one that announced | once each |

The scenarios of the steps before were run once each and ended as before: one publisher by PUBLISH and by namespace, two publishers by PUBLISH, a PUBLISH while the relay waits for an answer, and a publisher that publishes anew under the same alias and under another.

`mise run check` ended with `OK` for all four parts. `mise run interop` was run twice on the final code. **The first run ended with `57 of 58`**: the pairing "moqtail relay: own-browser to own-browser, an Object with Object Properties, then PUBLISH_DONE" printed neither RESULT nor FAILED in 120 seconds. That pairing does not go through the own relay; the comparison of the world `full` and a run of `mise run check` were going on in the same working copy at the time, and another interop matrix on the same machine. The second run, with nothing else of this working copy running, ended with `58 of 58 pairings ended as expected`. **The machine slept during the first run**: the power log (`pmset -g log`) has it enter sleep at 01:50:53 and wake at 02:06:14 (+0900), and the run ended at 02:06:45, having taken more than 10 minutes by the wall clock where the matrix reports 38.9 seconds. A limit of 120 seconds that is measured by the wall clock runs out across such a sleep, which fits the pairing printing neither result. Nothing else about the pairing was looked into, and its log was overwritten by the second run.

## Limitations and blockers

No blocker.

- **The runs with real peers had own Rust clients only.** No browser, no `moqtail` peer: a pairing with two publishers that only announce is not in the interop matrix yet.
- **A track is asked of at most 4 publishers, counting its sources.** A fifth is asked when a subscriber comes or a namespace is announced while there is room, and not when room comes about. Covered by tests of the core, not by a run with real peers.
- **A publisher that did not answer in 5 seconds is not asked for that track again until it announces the namespace again.** The relay keeps no time after which it tries again.
- **A SUBSCRIBE for a track whose sources all drain is still refused**, against §9.5 (implementation notes, known gaps).
- What a track remembers of publishers that declined, the limit of 64 requests and the room of a track are covered by tests of the core and by the random events only.
- `--touch asks` takes nearly every sequence of `full` for one the change is about (19,857 of 20,000), so "differs before such an event" says little there. The two worlds without such events and the bystanders of the other four are what holds.
- **The random events do not reach a namespace announced while a track has less room than publishers waiting.** The counts of differing sequences were the same before and after the change that asks the announcing publisher first, so that order is held by the tests of the core alone.
- In `full` two events differ in the order of the actions alone, both when a session is dropped that has to do with several tracks, where either version walks a map of tracks.
- The comparison covers the core. The hub was not changed.
