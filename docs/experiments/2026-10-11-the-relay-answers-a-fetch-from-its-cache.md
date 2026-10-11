# The relay answers a FETCH from its cache of the latest Group

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay keeps the latest Group of each track and answers a FETCH from it where it can

## Objective

The relay now keeps the Objects of the latest Group of every track and answers a FETCH whose range lies in that Group itself, where it has every Object ID of the Group from 0 to the end of the range or knows it not to exist; any other FETCH goes to a publisher as before (implementation notes, row on the relay's cache). Run, through the real relay, a FETCH the cache answers from a publisher that answers no FETCH, one it does not answer, and a Joining FETCH with offset 0; and compare the relay core before and after with the tool that feeds both the same events.

## Environment

The own relay and the Rust client built from the working copy this record is part of, on both ends. Loopback, port 48600, macOS. Every run has a relay and a publisher started anew.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"

# The publisher, without --answer-fetch: it refuses every FETCH with NOT_SUPPORTED. Its driver writes
# `send` four times from 3 s on (Object 0 of the Groups 0 to 3) and once more 6 s later (Group 4).
$C --mode publish --payload-hex 6162 --timeout-secs 60

# The client under test, 5 s after the publisher started: the latest Group is 3.
$C --mode fetch --fetch-start 3,0 --fetch-end 3,0                                   # 1
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0                                   # 2
$C --mode subscribe --objects 1 --join relative:0                                   # 3
# 4 and 5: the payload is 400 KiB.
$C --mode fetch --fetch-start 3,0 --fetch-end 3,0 --fetch-read-after-ms 15000       # 4
$C --mode fetch --fetch-start 3,0 --fetch-end 3,0                                   # 5

T="python3 spikes/relay-core-diff/compare.py"
$T @- disk --world fetch --kinds --touch fetch     # also split-fetch, shared-fetch, full
```

## Observed results

One run each.

| Run | The client printed | The relay's log |
|---|---|---|
| 1 | `FETCH_OK ... end_group=3 end_object=1`, `FETCH_OBJECT ... group=3 object=0 subgroup=0 ... payload_hex=6162`, `FETCH_ENDED ... how=fin code=none objects=1` | `SEND session=1 stream=1 FETCH_OK`, `WRITE_FETCH_STREAM session=1 request_id=0 pieces=1`; no `OPEN_REQUEST ... FETCH` |
| 2 | `FETCH_REFUSED ... error_code=0x10` | `OPEN_REQUEST session=0 own=0 FETCH`, `RECV session=0 own=0 REQUEST_ERROR error_code=0x3`, `SEND session=1 stream=1 REQUEST_ERROR error_code=0x10` |
| 3 | `SUBSCRIBED`, `FETCH_OK ... end_group=3 end_object=1`, `FETCH_OBJECT ... group=3 object=0`, `FETCH_ENDED ... how=fin code=none objects=1`, then `OBJECT group=4 object=0` on the subscription | `WRITE_FETCH_STREAM session=1 request_id=2 pieces=1`; no `OPEN_REQUEST ... FETCH` |
| 4 | The same as run 1 with `payload_len=409600`, 15 s after it started | as in run 1 |
| 5 | The same at once | as in run 1 |

**Run 1: the relay answered from its cache, and asked nobody.** The publisher answers no FETCH, and the client got the Object of the latest Group; the End Location asked for, {3, 0}, is the whole Group, and the answer has {3, 1} (§10.13). **Run 2: a range that begins in an earlier Group went to the publisher**, whose `NOT_SUPPORTED` the relay passes on as `DOES_NOT_EXIST`, as before the change. **Run 3: a Joining FETCH with offset 0 got the latest Group from the cache**, and the subscription the Group after it.

**Run 4: a subscriber that did not read for 15 seconds was not cut, and got everything.** The transport took the 400 KiB at once, as it took the 800 KiB of an earlier record ([run 15 there](2026-10-11-rust-client-answers-a-fetch.md)); the relay's wait of 10 seconds for a piece never began.

**The cores before and after differ in the FETCH requests the cache answers.** 20,000 random sequences of 80 events a world; "before" is the change this one is built on.

| World | Sequences that differ | Bystanders treated differently |
|---|---|---|
| `fetch` | 8 | - |
| `split-fetch` | 1 | 0 of 304,966 actions |
| `shared-fetch` | 1 | 0 of 273,013 actions |
| `full` (no FETCH) | 0 | - |

In all ten, the first difference is that the new core answers a FETCH with FETCH_OK and a stream it writes itself where the old one did something else: asked a publisher (8, one of them a Joining FETCH that was held for its SUBSCRIBE), or refused with `DOES_NOT_EXIST` (2). In those two the FETCH came from the session that is the track's only publisher, which the relay never asks; the cache answers it all the same.

## Limitations and blockers

- **The random events rarely make a FETCH the cache can answer**: ten sequences of 60,000. A FETCH there has to lie in the latest Group and find its Objects from 0 on. What the cache answers and what it leaves to a publisher is a table test of the cache and of the relay core.
- **No run stopped a stream the relay was writing, and none had a subscriber cut for taking nothing.** One Object is the most a Group of the Rust publisher has, and the transport takes it whole. Both are tests on in-memory connections: of the task that writes, and of the hub from a FETCH to the reset of its stream when the subscriber cancels.
- A Group of several Objects, of several Subgroups, with a Prior Object ID Gap or an End of Group, a Group over the limits, a track with MAX_CACHE_DURATION, and a publisher whose LARGEST_OBJECT is ahead of what it sent were not run against a peer: the Rust publisher sends Object 0 of a Group and nothing else. They are tests of the cache and of the relay core.
- **No other implementation has read a fetch stream the relay wrote with Objects on it**: `moqtail`'s clients, the browser page and `mlmsub` were not run against the cache here. The bytes are a test against the decoder of `crates/moqt`. `mise run interop` has pairings in which a Joining FETCH for a catalog is now answered by the relay instead of the page; they ended as expected, and which of the two answered was not looked at in their logs.
- What the relay holds of a track goes with its last publisher: a FETCH after that is refused as before.
- "The first difference" is all the comparison says of a sequence; see [the record of that tool](2026-10-11-relay-core-diff-with-fetch.md).
