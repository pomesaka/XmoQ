# The relay takes a track from several publishers that send PUBLISH

- Date: 2026-10-10
- Commit SHA: not fixed when this was written. Compared with `main` at `450b9df8dd91`
- Roadmap item: Phase 0b, take one track from several publishers. The third of the steps. The relay still asks one publisher only, and two cases of a PUBLISH are still refused (below)

## Objective

§9.3: "Relays MUST handle Objects for the same Track from multiple publishers and forward them to matching Established subscriptions." §9.5: "When it receives an authorized PUBLISH message for a Track that has Established downstream subscriptions, it MUST respond with PUBLISH_OK." Until now a second PUBLISH for a track was refused.

What changes, and is meant to be all that changes:

1. A PUBLISH for a track that has a source is accepted, up to 4 sources a track; the fifth is refused with `EXCESSIVE_LOAD`.
2. A Subgroup is taken from the source that sends it first. A stream of another source for it is stopped, whether the first is open, was cut short or ended with a FIN.
3. A source that ends while another is left ends alone. The track ends with its last source.
4. A malformed track ends for every source: the one that sent what was malformed is cancelled with `MALFORMED_TRACK`, the others with `CANCELLED`.
5. A subscriber that comes gets the Track Properties of the source that came last, and the largest Location over all sources.
6. When the last subscriber of a track goes, the relay cancels the SUBSCRIBEs it sent for the track, and the track stays if it has a source that came with PUBLISH.
7. Once a track has had several sources at a time, a stream of a Group that is 8 or more behind the newest is stopped, also after the track is back to one source. For a track that only ever had one source at a time it passes, as before.

What does not change yet: a PUBLISH for a track the relay has asked another publisher for and has no answer about, and one for a track whose sources have all said PUBLISH_DONE, are refused with `UNINTERESTED` as before. So is one from a publisher that said PUBLISH_DONE for the track and whose last streams the relay still awaits, while the track goes on with another source: the first version of this step accepted it, and the track then had two sources on one session, of which the older one took the newer one's streams with it when it had drained. Three reviewers found that, each on their own; the test that holds it now failed on that version. So no track with a source has a SUBSCRIBE of the relay that is still unanswered, which a test holds.

The reasons are in [implementation notes](../moqt-implementation-notes.md), section Relay.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Own relay and client | `xmoq-relay` and `xmoq-moqt-client` 0.0.0. The client has a new flag, `--first-group N`, for the Group ID of the first Object it sends |
| moqtail peers | the moqtail Rust client (`moqtail` crate 0.15.0), as publisher and as subscriber |

Provisional setting: 4 sources a track.

## Commands

```sh
mise run check
mise run interop

# The new core next to the one on main.
T="--touching --touch publishers"
python3 spikes/relay-core-diff/compare.py main disk --world one-publisher $T --spikes
python3 spikes/relay-core-diff/compare.py main disk --world full $T
python3 spikes/relay-core-diff/compare.py main disk --world split-one $T
python3 spikes/relay-core-diff/compare.py main disk --world shared $T
python3 spikes/relay-core-diff/compare.py main disk --world shared $T --raw-numbers  # has to fail
python3 spikes/relay-core-diff/compare.py --same disk --world full

# Two publishers of one track, with real peers. Each of the 8 pairings three times.
node spikes/moqtail/browser/run.mjs --relay own --direction rust-to-rust \
  --rust-publisher own|moqtail --second-rust-publisher own|moqtail --rust-subscriber own|moqtail
```

The worlds of the random events (`spikes/relay-core-diff/fuzz.rs`): in `one-publisher`, one session is the only one that sends PUBLISH, PUBLISH_NAMESPACE or data streams, so no track can get a second source. `split-one` has two halves that share nothing: sessions 1 and 2 both publish one track, session 3 publishes another and session 4 subscribes to it. `shared` is `split-one` in which session 4 also subscribes to the track of the first half. An event the change is about (`--touch publishers`) is a PUBLISH for a track that another session published before in the sequence, or that comes after another session announced a namespace.

The run with real peers: the first publisher publishes, then the second; `PUBLISHED` from the second is printed after its PUBLISH_OK. A subscriber subscribes. Both publishers send the same Object, Object 0 of Group 0 with the same payload. The first publisher ends with PUBLISH_DONE. An own second publisher then sends its next Object, which is Group 1; the moqtail publisher of this spike sends one Object only, so the run waits 1.5 seconds in its place. The moqtail subscriber of this spike leaves after its first Object.

## Observed results

**Passed.**

### What was not to change did not

| World | Sequences | With an event the change is about | In which the versions differ | Differ before such an event | Events in which the bystanders were treated differently (actions for them) |
|---|---|---|---|---|---|
| `one-publisher` | 20,000 | 0 | 0 | 0 | no bystanders |
| `full` | 20,000 | 19,717 | 8,050 | 0 | no bystanders |
| `split-one` | 20,000 | 19,229 | 4,657 | 0 | 0 (345,323) |
| `shared` | 20,000 | 19,229 | 4,657 | 0 | 0 (308,099) |

In `one-publisher` the two versions did the same in every one of the 1,600,000 events. The 12 scenarios with real peers, each with one publisher a track, ran against a relay that holds both versions: no difference of any kind, and the payload matched in all 12.

In `shared`, with `--raw-numbers` the same run reports 156 events with a difference for the bystanders: the check can fail.

`--same`, world `full`: `b6e2d1a80b2e2b73` in three runs in processes of their own.

**Three things came out of making the bystander check exact**, each a way in which one track of a session shows in another:

- The relay numbers the requests it opens from one counter, like the streams it opens. They are compared by the session and their order on it now, as the streams are.
- The relay numbers the Track Aliases of a subscriber session as it accepts subscriptions. In `shared`, a subscription to the first track that one version accepted and the other refused moved the alias of the next subscription to the second track by one. Aliases are compared by which subscription they are now. A subscriber does not see more of this than the number.
- A message that is an error on a request stream closes the session (as the draft has it) in the version in which the request is still open, and is ignored in the one in which it is over. On a stream for the first track that is the change itself, and it takes the session's subscription to the second track with it. Session 4 of `shared` therefore sends no such message; what a protocol error on one track does to a session's other tracks is the same as before this step.

### The new behaviour

18 tests in `crates/relay-core/src/tests/sources.rs`. Besides the ones below: a publisher that publishes anew while its source drains; a Group that is too old to tell, with two sources and after one of them went; a source that is left when the one that was ahead goes; a publisher that publishes anew when its source has drained; two sources that chose the same Track Alias; a source that is a SUBSCRIBE of the relay and goes, in four ways; a malformed track with such a source; a stream of another source whose first Object names the Subgroup; that nothing is left when everyone has gone. The first nine: what two publishers send reaches the subscribers; the same Subgroup from two sources, for the three states of the first; a source that goes while another is left, for four ways of going; a track that ends when its last source has drained; the limit of 4 and the room a source leaves; a malformed track with two sources; Track Properties and the largest Location; a session that goes and has a track of its own and one it shared; a PUBLISH for a track the relay subscribed to upstream, and what becomes of that SUBSCRIBE when the last subscriber goes.

Two tests that had a second PUBLISH refused were given another reason for the refusal, a track whose publisher said PUBLISH_DONE, since the first reason is gone.

### Two publishers of one track, with real peers

| First publisher | Second publisher | Subscriber | Second PUBLISH accepted | Group 0 at the subscriber | Subscription goes on when the first has left | Group 1 from the second arrives |
|---|---|---|---|---|---|---|
| own | own | own | yes | once | yes | yes |
| own | own | moqtail | yes | once | not seen | not seen |
| own | moqtail | own | yes | once | yes | not sent |
| own | moqtail | moqtail | yes | once | not seen | not sent |
| moqtail | own | own | yes | once | yes | yes |
| moqtail | own | moqtail | yes | once | not seen | not seen |
| moqtail | moqtail | own | yes | once | yes | not sent |
| moqtail | moqtail | moqtail | yes | once | not seen | not sent |

Each row three times. The 24 runs were done twice, before and after the review of this step. The first time all 24 had these results. The second time 22 had; in the other two, one of own and moqtail publishers with an own subscriber and one of two moqtail publishers with a moqtail subscriber, the moqtail second publisher exited before it reported its Object as sent (next section), and the run was counted as failed. In the relay's log of each run the second publisher's stream for Group 0 is stopped (`STOP_DATA`, code `0x1`).

"Not seen": the moqtail subscriber of this spike has left by then. "Not sent": the moqtail publisher of this spike sends one Object.

### A publisher whose stream is stopped while it writes

In three runs in all (one before the first 24, two among the second 24), a moqtail second publisher exited with an error before it reported its Object as sent; in 6 runs of that pairing in between it did not. It is a race, and it is what a second publisher has to expect: the relay stops its stream for a Subgroup the other one sent first, and the STOP_SENDING can arrive before the publisher has finished the stream. With a payload of 200,053 bytes (`--payload-padding 200000`) the stop arrives in time in every run that was tried:

| Second publisher | What it did when its stream for Group 0 was stopped |
|---|---|
| own Rust client | Printed `STOP_SENDING_RECEIVED data#0 code=0x1` and `SENT`, sent Group 1 when asked, and ended with PUBLISH_DONE. The subscriber got Group 0 once and Group 1 |
| moqtail Rust client of this spike | Exited with `Error: [SendDataStream::finish], [stream closed or stopped]`. `moqtail` 0.15.0 returns the transport's error from `send_object` and `finish` on a stream that was stopped (`src/transport/data_stream_handler.rs`), and the spike's client ends on any error. Its session closes, and the relay lets go of it as a source |

One run each. The own client had this weakness in part before this step: a stop that arrived while it was still writing the stream ended the run with an error, and only one that arrived after the write was taken as no error. It is now no error in both places, and the stream is reset when it was not written out (§11.4.3: "The publisher SHOULD respond with a reset"). The run above does not tell which of the two places the stop was seen in.

`@xmoq/moqt` was read and not run: a STOP_SENDING forgets the stream and resets it, `write()` on it fails with `SubgroupClosedError`, `close()` does nothing, and the publication goes on.

`--first-group 7`, one own publisher and one own subscriber: the subscriber printed `OBJECT group=7`.

`mise run check` ended with `OK` for all four parts (98 tests of the core), and `mise run interop` with `46 of 46 pairings ended as expected`.

## Limitations and blockers

No blocker.

- **The two publishers of every run sent the same Object under the same Group ID.** Publishers whose Groups are numbered differently, or that send different content under the same IDs, were not run. Nothing in the relay compares what two sources send (known gap).
- **A publisher that is 8 Groups or more behind another is covered by tests of the core only**, while the other is there and after it went. Its streams for those Groups are stopped; with real peers that was not run.
- **A publisher that sends a Group ID far ahead of the others silences them**: their Groups count as too old, and their streams are stopped. Before the seventh change it made the relay pass their Subgroups on twice. Known gap, under authorization.
- **Of the six changes, the runs with real peers show the first three, each in its simplest case**: a second PUBLISH is accepted, the second copy of a Subgroup is stopped, and a publisher that ends with PUBLISH_DONE leaves the track to the other. A publisher that is killed, the limit of 4, a malformed track, Track Properties and a track of both kinds of source are covered by the tests of the core only.
- **No run had a browser on either side.**
- The moqtail peers are the clients of this repository's spike, which send and take one Object. What moqtail's own tools do as one of two publishers was not looked at.
- In `shared`, what is held alike for session 4 is its subscription to the second track, without its Track Alias, and session 4 sends no message that is an error on a request stream.
- The limit on streams that wait to be written is counted per subscriber session in the I/O side, and is not part of the comparison (an open question for Phase 1).
- The comparison covers the core. The hub was not changed.

## Open questions raised

- How many sources a track may have: in [open questions](../open-questions.md).
