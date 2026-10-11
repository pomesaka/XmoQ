# A FETCH that was served leaves its request stream to the subscriber

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

The pairing of `mise run interop` in which `mlmsub`, the subscriber of `moqlivemock`, reads a catalog through the own relay with a Joining FETCH failed now and then: `reading catalog: peer finished its half of the request stream`. The relay finished its direction of the request stream right after it had passed the fetch stream on to its FIN; the two streams arrive apart, and `moqtransport` v0.14.0 fails a FETCH whose request stream is finished before it has read the fetch stream to its end. The relay, and the Rust client where it answers a FETCH, now leave their direction open until the subscriber's has ended, or 3 seconds have passed (§5.2 asks nobody to finish the request stream). Count how often the pairing fails before and after, and compare the relay core before and after with the tool that feeds both the same events.

## Environment

The own relay and the page of `interop/` built from the working copy this record is part of; `mlmsub` of `moqlivemock` v0.16.1 as `scripts/ci/interop.sh` builds it, with the patch of `interop/peers/moqlivemock/`. Loopback, port 48700, macOS. "Before" is the same working copy without the change to the relay and to the Rust client.

## Commands

```sh
scripts/ci/interop.sh --base-port 48100     # for the builds, and every pairing: twice after the change
# 100 times, counting the runs that exit with an error:
(cd interop && bun catalog.ts --catalog to-mlmsub --port 48700)

T="python3 spikes/relay-core-diff/compare.py"
$T @- disk --world fetch --kinds --touch fetch     # also split-fetch, shared-fetch, full
```

## Observed results

**Before, 5 of 100 runs failed; after, none of 100.** All five failed with `peer finished its half of the request stream`.

| | Runs | Failed |
|---|---|---|
| Before | 100 | 5 |
| After | 100 | 0 |

One run after the change with the relay's log, the times taken where the lines were read: the relay printed `FETCH_PIPE_ENDED session=1 stream=2 finished`, then `RESET_RECEIVED session=1 stream=2 code=0x1`, and `FINISH_REQUEST session=1 stream=2` only after that, all within 10 ms. `mlmsub` resets its request stream when it has read the fetch stream, and the relay finishes its direction then.

**Every pairing of `mise run interop` ended as expected after the change**, 104 of 104, in two runs (101.6 and 82.9 seconds; 82.8 before). No pairing has a client of `moqtail` that sends a FETCH, so none shows what the relay does towards a subscriber that never ends its direction; the pairings with a FETCH took as long as before. In the longer run a pairing without a FETCH took 9.2 seconds where it takes 0.1; why was not looked into.

**The own clients were answered as before.** An own Rust client that fetched four Objects from an own publisher through the relay printed `FETCH_OK`, four Objects and `FETCH_ENDED ... how=fin code=none objects=4`; the publisher printed `FETCH_SERVED ... how=fin code=none objects=4`. One with FILL_TIMEOUT 0 got its indicator and `how=fin`. One run each. The client leaves when it has read the fetch stream, so the relay's FIN was not seen in these.

**The cores before and after differ in one thing: where the old one finished the request stream of a FETCH that was served, the new one starts the wait.** 20,000 random sequences of 80 events a world.

| World | Sequences that differ | When the relay's own stream was written | When a stream that was passed on ended | With a FETCH_OK that came after the stream had ended | Bystanders treated differently |
|---|---|---|---|---|---|
| `fetch` | 1,445 | 1,274 | 124 | 47 | - |
| `split-fetch` | 2,459 | 2,362 | 70 | 27 | 0 of 304,966 actions |
| `shared-fetch` | 2,459 | 2,362 | 70 | 27 | 0 of 273,013 actions |
| `full` (no FETCH) | 0 | - | - | - | - |

In every sequence that differs, the first difference is that the last action of the step is `StartFetchEndTimer` where the old core had `FinishRequest`, and the actions before it are the same.

## Limitations and blockers

- **The cause was not shown, only its end.** That `moqtransport` reads the FIN of the request stream first in the runs that failed was read in its source by another session, and fits the message; no trace of a failing run was taken here.
- 100 runs with no failure do not show that none is left: before the change the rate was 5 in 100 here, and 5 in 40 in the session that found it.
- **The random events of the tool do not have the end of the wait**: the old core does not know the event. What the new core does when the subscriber ends its direction, when the 3 seconds are over, and when the session goes while it waits is a test of the core.
- No run had a subscriber that never ends its direction: the 3 seconds, and the 2 after them, ran in no real run. In the Rust client the wait is a test of the task of the request stream, with shorter times.
- `@xmoq/moqt`, where it answers a FETCH, is changed by another session; nothing of it was run here.
- "The first difference" is all the comparison says of a sequence; see [the record of that tool](2026-10-11-relay-core-diff-with-fetch.md).

## The table of how a served FETCH ends, read by the tests

Added after the runs above, with `fixtures/peer-fetch/served-end.tsv`. Its outcomes were written from the rule before any test read them.

- The relay's test (`crates/relay/src/tests.rs`, over the hub and in-memory connections) had all ten rows as written.
- The Rust client's test (`crates/moqt-client/src/serving.rs`) did not, and two mistakes of the client were put right; no row changed. One: the task of a request stream waited for ever where the client let go of its queue of messages and ended the stream in one step. The other: a direction the subscriber had stopped at the moment the fetch stream was written was finished, where it is to be reset.
- **The table's test holds the first of the two and not the second.** A reviewer took each correction out again: without the first, the table's test fails; without the second it passes, and `an_answer_that_cannot_be_written_still_ends_the_request_stream_once` fails. The table has no row for a STOP_SENDING that came before the fetch stream was written.
- In the Rust client's test the `update` step sent nothing at first, so the `update` column was read and not checked. It now hands an answer over for the stream and checks that none is written.
