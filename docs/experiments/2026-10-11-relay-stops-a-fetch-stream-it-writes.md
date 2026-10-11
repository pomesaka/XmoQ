# The relay stops a fetch stream it writes itself

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

A fetch stream the relay writes itself (the answer to a FETCH with FILL_TIMEOUT 0) was handed to the I/O side as bytes and written in one go, and nothing could stop it: a subscriber that cancelled its FETCH got the stream all the same (§5.2: "It MUST reset the bidi request stream and unidirectional data stream associated with the FETCH"), and so did one whose track was found malformed (§2.4.2: "reset any fetch streams with Status Code MALFORMED_TRACK"). The stream is now handed over as pieces, written one after the other, and can be ordered stopped. Check that a FETCH with FILL_TIMEOUT 0 is answered as before through the real relay, and compare the relay core before and after with the tool that feeds both the same events.

## Environment

The own relay and the Rust client built from the working copy this record is part of. Loopback, port 48600, macOS.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"
# The publisher: Object 0 of the Groups 0 to 3.
$C --mode publish --payload-hex 6162 --timeout-secs 60 --answer-fetch
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fill-timeout-ms 0

T="python3 spikes/relay-core-diff/compare.py"
$T @- disk --world fetch --kinds --touch fetch     # also split-fetch, shared-fetch, full
```

## Observed results

**A FETCH with FILL_TIMEOUT 0 was answered as before.** The client printed `FETCH_OK ... end_group=3 end_object=1`, `FETCH_RANGE_END ... kind=unknown group=3 object=0` and `FETCH_ENDED ... how=fin code=none objects=0`. The relay's log line for the stream is now `WRITE_FETCH_STREAM session=1 request_id=0 pieces=1`; it had `bytes=` before. One run.

**The cores before and after differ in one thing: the new one orders such a stream stopped.** 20,000 random sequences of 80 events a world; "before" is the change this one is built on.

| World | Sequences that differ | At a request stream that is reset or stopped | At a track found malformed | Bystanders treated differently |
|---|---|---|---|---|
| `fetch` | 455 | 329 | 126 | - |
| `split-fetch` | 1,019 | 838 | 181 | 0 of 304,966 actions |
| `shared-fetch` | 1,019 | 838 | 181 | 0 of 273,013 actions |
| `full` (no FETCH) | 0 | - | - | - |

In every sequence that differs, the first difference is that the new core has `AbortFetchWrite` among its actions, one or more, and everything else of that step as the old core had it: with the code `CANCELLED` (0x1) where the subscriber's request stream was reset or stopped, and `MALFORMED_TRACK` (0x12), before the other actions, where the track was found malformed. That was checked over the lines the tool prints for the kinds, by a script, and not by eye.

The old core handed the stream over as bytes, and the two share the type of that action; the tool reads the old core's one line that makes the bytes as the piece they encode (`add_old_core` in `compare.py`). The bytes the I/O side writes for an indicator are now made there. A test of it compares them with the first six bytes of `fixtures/moqt-18-by-hand/fetch-stream-indicator-first.hex` (`05 00 81 0c 03 01`), a file both implementations read.

## Limitations and blockers

- **No run stopped a stream the relay was writing.** The stream carries one End of Range indicator, a few bytes, which the transport takes at once whether the subscriber reads or not: a cancellation cannot arrive while the relay still writes, on loopback. What the relay does then is a test of its core (the order to stop, and what follows the report of it) and of the task that writes (on an in-memory connection whose window is 64 bytes: a subscriber that takes nothing, one that stops the stream, an order to stop with two codes, one that is there before the task has run, one that comes with the end of the last write, and a subscriber that takes 50 pieces slower than the wait for one of them).
- The task that writes was not run on a loopback connection of its own. What it uses of a connection (a fetch stream opened with its header, a write that waits, a reset while it waits) is in the tests `xmoq-moqt-io` runs against both kinds of connection.
- "The first difference" is all the comparison says of a sequence; see [the record of that tool](2026-10-11-relay-core-diff-with-fetch.md).
- A subscriber that takes nothing is cut later than before: it had 10 seconds for the whole stream, and now has 10 for the stream to be opened and 10 more for each piece, 20 for a stream of one indicator. No run had a subscriber that took nothing.
