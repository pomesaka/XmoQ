# The Rust client answers a Standalone FETCH, through the own relay

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay answers a FETCH, from the publisher or from a cache

## Objective

`xmoq-moqt-client` as a publisher now answers a Standalone FETCH for its track from the Objects it sent (`--answer-fetch`, in `--mode publish` and `--mode publish-namespace`). Run a FETCH from one own client through the own relay to another: Standalone, and the two kinds of Joining FETCH, which the relay asks of the publisher as a Standalone one. And run, against a real peer, the waits of the relay for a publisher that is slow, which only tests on in-memory connections had shown: the publisher can now be made slow in each place the relay waits (`--fetch-answer-delay-ms`, `--fetch-stream-delay-ms`, `--fetch-write-every-ms`).

## Environment

The own relay and the Rust client built from the working copy this record is part of, on both ends. Loopback, port 48600, macOS. Every run has a relay and a publisher started anew. Each line of the relay and of the clients was given the time since its process started, by a filter on its output; the times below are differences within one process.

## Commands

```sh
target/debug/xmoq-relay --port 48600 --cert-file spikes/moqtail/cert/cert.pem --key-file spikes/moqtail/cert/key.pem
C="target/debug/xmoq-moqt-client --url https://127.0.0.1:48600 --cert-sha256 <sha256> --namespace xmoq/phase0 --track object"

# The publisher. Its driver writes `send` four times, 300 ms apart, from three seconds after the start,
# and keeps stdin open. <mode> is publish or publish-namespace.
$C --mode <mode> --payload-hex 6162 --timeout-secs 60 --answer-fetch <more flags>
# With publish-namespace, a first subscriber, which the Objects are sent to:
$C --mode subscribe --objects 4

# The client under test, five seconds after the publisher started.
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0                              # 1
$C --mode fetch --fetch-start 1,0 --fetch-end 2,0 --group-order descending     # 2
$C --mode subscribe --objects 1 --join relative:1                              # 3
$C --mode subscribe --objects 1 --join absolute:1                              # 4
$C --mode fetch --fetch-start 2,0 --fetch-end 9,0                              # 5
$C --mode fetch --fetch-start 7,0 --fetch-end 9,0                              # 6
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fetch-cancel-after-objects 1   # 7, publisher: --fetch-write-every-ms 500
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0                              # 8, publisher: --fetch-answer-delay-ms 6000
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0                              # 9, publisher: --fetch-stream-delay-ms 12000
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fill-timeout-ms 2000       # 10, publisher as in 9
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0                              # 11, publisher: --fetch-write-every-ms 3000
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0                              # 12, publisher: --fetch-write-every-ms 7000
$C --mode fetch --fetch-start 0,0 --fetch-end 3,0 --fetch-count 16             # 13
# 14 and 15: the payload is 100 KiB (--payload-hex with 102,400 bytes), sent 20 ms apart.
$C --mode fetch --fetch-start 0,0 --fetch-end 49,0                             # 14, after 50 Objects
$C --mode fetch --fetch-start 0,0 --fetch-end 39,0 --fetch-read-after-ms 15000 # 15, after 40 Objects
$C --mode subscribe --objects 1 --join relative:1 --join-at-once               # 16: 30 times, the publisher stays 2 s
```

Runs 1 to 6 were made with the publisher in each of the two modes. Runs 7 to 9 and 12 as well. Runs 10, 11, 14, 15 and 16 with `--mode publish` only, run 13 with `--mode publish-namespace` only.

## Observed results

One run each, but for runs 16 and 17 (30 and 6). Runs 1, 7 and 15 were made again after the client was changed to stop the subscriber's direction of a request stream it resets, and to tell its session when its answer has ended; they came out the same, but for which of the relay's reset and STOP_SENDING the publisher saw first.

**Runs 1 to 6: the client got the Objects of its range, from a publisher in either mode.** The publisher had sent Object 0 of the Groups 0 to 3.

| Run | The publisher printed | The client printed |
|---|---|---|
| 1 | `FETCH_ASKED ... start_group=0 start_object=0 end_group=3 end_object=0`, `FETCH_ANSWERED ... end_group=3 end_object=1`, `FETCH_SERVED ... how=fin code=none objects=4` | `FETCH_OK ... end_group=3 end_object=1`, the Objects of Groups 0, 1, 2, 3, `FETCH_ENDED ... how=fin code=none objects=4` |
| 2 | `FETCH_ANSWERED ... end_group=2 end_object=0`, `objects=2` | Group 2, then Group 1 |
| 3 | `FETCH_ASKED ... start_group=2 start_object=0 end_group=3 end_object=1`, `objects=2` | Groups 2 and 3 on the fetch stream |
| 4 | `FETCH_ASKED ... start_group=1 start_object=0 end_group=3 end_object=1`, `objects=3` | Groups 1, 2, 3 |
| 5 | `FETCH_ANSWERED ... end_group=3 end_object=1`, `objects=2` | `FETCH_OK ... end_group=3 end_object=1`, Groups 2 and 3 |
| 6 | `FETCH_ANSWERED ... error_code=0x11` | `FETCH_REFUSED ... error_code=0x11` (`INVALID_RANGE`) |

In run 1 the End Location that was asked for, {3, 0}, is the whole of Group 3, which is beyond the Largest Object {3, 0}; the answer has {3, 1} (§10.13). In runs 3 and 4 the relay worked out the range from the largest Location it knew for the track and asked the publisher with a Standalone FETCH. With `--mode publish-namespace` it knew that Location from the Objects that had passed through it to the first subscriber: the publisher's SUBSCRIBE_OK had no LARGEST_OBJECT, because the relay's SUBSCRIBE came before the first Object.

**Runs 7 to 12: the relay waited for the publisher as long as the implementation notes say.**

| Run | The publisher | What happened, and when |
|---|---|---|
| 7 | writes an Object every 500 ms; the subscriber gives up after one | Publisher: `FETCH_SERVED ... how=request_reset code=0x1 objects=1`, in both modes |
| 8 | answers after 6 s | The client got `FETCH_REFUSED ... error_code=0x2` (`TIMEOUT`) 5.0 s after it sent the FETCH. The publisher's request was reset then: `FETCH_SERVED ... objects=0`, 5.0 s after `FETCH_ASKED`, with `how=request_reset` in one mode and `how=request_stopped` in the other |
| 9 | opens its stream 12 s after FETCH_OK; no FILL_TIMEOUT | The client got FETCH_OK at once, and `FETCH_ENDED ... how=request_reset code=0x6` (`UNKNOWN_OBJECT_STATUS`) 5.0 s later. Relay: `RESET_REQUEST ... code=0x6` |
| 10 | the same; FILL_TIMEOUT 2 s | The same, 2.0 s after the FETCH_OK |
| 11 | writes an Object every 3 s | Not cut: four Objects, `how=fin`, 12.0 s after the FETCH_OK |
| 12 | writes an Object every 7 s | Relay: `FETCH_PIPE_ENDED ... upstream_stalled`, 5.0 s after the first Object. Client: one Object, `FETCH_ENDED ... how=stream_reset code=0x6 objects=1` |

Whether the publisher sees the relay's reset of the request stream or its STOP_SENDING first (run 8) is a race between the two; it resets both of its streams for either.

**Run 13: 16 FETCH requests at once were all answered.** 16 lines `FETCH_SERVED ... how=fin code=none objects=4`.

**Run 14: Objects the publisher had let go of were said to be unknown.** 50 Objects of 100 KiB are about 4.9 MiB, more than the 4 MiB the client keeps; it had let go of the Groups 0 to 9. The client printed `FETCH_OK ... end_group=49 end_object=1`, `FETCH_RANGE_END ... kind=unknown group=9 object=0`, then the Objects of the Groups 10 to 49, `FETCH_ENDED ... how=fin code=none objects=40`.

**Run 15: a subscriber that did not read cost the publisher nothing but that FETCH.** The relay cut the subscriber 10.0 s after it began to pass the stream on (`FETCH_PIPE_ENDED ... downstream_stalled`, `RESET_REQUEST ... code=0x5`); the publisher printed `FETCH_SERVED ... how=request_stopped code=0x1 objects=15` at that moment: 15 of the 40 Objects had been taken, by the relay and the transport, before the stream stood still. The publisher had sent its 40 Objects on the subscription before the FETCH came.

With four Objects of 200 KiB the same subscriber cut nothing: the relay and the transport took the 800 KiB at once, the publisher printed `how=fin` within 20 ms, and the client read them 15 s later.

**Run 16: none of 30 Joining FETCH requests sent together with their SUBSCRIBE was refused.** All 30 clients printed `FETCH_ENDED ... how=fin code=none objects=2`. In 7 of the 30 the relay's log has `HOLD_REQUEST session=N stream=S FETCH`: it read the FETCH first and held it.

**Run 17, after the review: an answer that is late still comes whole.** The publisher with `--fetch-answer-delay-ms 1000`, three times in each mode: the client got FETCH_OK 1.0 s after its FETCH, four Objects and `FETCH_ENDED ... how=fin code=none objects=4` in all six. Before, the task of the request stream could finish the stream without the FETCH_OK when the fetch stream was written at the moment the answer was due (found in review with a test of that task; no run here had an answer that was late and came).

**Run 18, after the review as well: a Joining FETCH while Objects go out.** The publisher sent an Object of 100 KiB every 100 ms; a subscriber came with `--join relative:2`. The publisher printed `FETCH_ASKED ... start_group=16 start_object=0 end_group=18 end_object=1` and `objects=3`; the client got Groups 16, 17 and 18 on the fetch stream and 19, 20 and 21 on the subscription.

## Limitations and blockers

- **One run each, on one machine**, but for runs 16 and 17.
- **The limit of FETCH requests a publisher answers at once was not reached.** It was 16 in these runs and is 32 now. The own relay counts 16 it has asked one publisher for, and stops counting one before the client has heard of its end, so a 17th could reach the client; with 16 that was a refusal waiting to happen, and none was seen. The client's check is a test of `serving::Answers`.
- **A fetch stream that is stopped while the request stream stays open was not seen.** The relay ends both when a subscriber gives up, and which of the two the publisher sees first is a race (`how=request_reset`, `how=request_stopped`); `how=stream_stopped` appeared in no run. What the client decides for it is a row of the table test of `serving::step`.
- A REQUEST_UPDATE for a FETCH the publisher answers was not sent: no client here sends one.
- A Descending FETCH over Objects that were let go of was not run. Where the indicator stands then is a decision the draft does not make (implementation notes, row on an End of Range indicator in a Descending fetch); it is a test of `published::Published::answer`.
- `--join-at-once` was run with `--mode publish` only.
- moqtail's clients, the browser and `moqlivemock` were not run against this publisher. `mise run interop` has no pairing that passes `--answer-fetch` yet.
