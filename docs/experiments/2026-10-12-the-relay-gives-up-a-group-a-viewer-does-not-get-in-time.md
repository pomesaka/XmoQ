# The relay gives up a Group a viewer does not get in time

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay (latency); open question on whether a slow subscriber loses whole old Groups and stays subscribed

## Objective

[The record before](2026-10-12-cubic-and-bbr-over-a-link-with-too-little-room.md) ended with a video that falls seconds behind when the link has less room than it, because nothing leaves frames out. Here the relay does: a try of resetting the stream of a Group the viewer does not have in time, and going on with the next. Material for the open question; nothing is decided by it.

## What the draft allows

Read in draft-ietf-moq-transport-18:

- §8: "Publishers can, at their discretion, discontinue forwarding Objects before either of the timeouts occurs, subject to stream closure and ordering constraints described in Section 11.4.3." So a relay may give a stream up of its own accord, with no `SUBGROUP_DELIVERY_TIMEOUT` or `OBJECT_DELIVERY_TIMEOUT` from anybody.
- §11.4.3: "If a sender closes the stream before delivering all such objects to the QUIC stream, it MUST reset the stream. This includes, but is not limited to: Either of the delivery timeouts defined in Section 8". §3.3.3: "DELIVERY_TIMEOUT (0x2): A delivery timeout (Section 8) was exceeded for this stream."
- §10.11: "Note that some objects in the subscribed track might never be delivered, because a stream was reset, or never opened in the first place, due to the delivery timeouts (see Section 8)." The subscription goes on.

## What was built

`xmoq-relay --delivery-patience-ms N`, off without the flag. When the relay has written the last Object of a subgroup stream and its FIN, it waits N ms for the viewer's transport to acknowledge all of the stream; if it has not, the stream is reset with `DELIVERY_TIMEOUT` (0x2), and the log has `DELIVERY_TIMED_OUT session= out=`. The streams after it are not touched. This is the shape of §8's `SUBGROUP_DELIVERY_TIMEOUT` with a value of the relay's own.

- It holds for every subgroup stream the relay writes, of every track, not for video alone.
- WHY the time is counted from the FIN and not from each Object: the relay's QUIC send window is 4 MiB, so a write of an Object never waits, and an Object's age when it is written says nothing about the viewer.
- The parameters `SUBGROUP_DELIVERY_TIMEOUT` and `OBJECT_DELIVERY_TIMEOUT` of a subscriber or a track are still not acted on.
- The in-memory connection of the tests does not give a stream up (the default of `SendStream::finish_within` only finishes). No test covers the new path; it was run over QUIC alone, below.

## Commands

```sh
[PATIENCE_MS=500] CONGESTION=cubic|bbr RATE=<kbit/s> QUEUE=100 spikes/latency-loss/run.sh 50 0 fan-720-2500k.h264 group 750 1
```

The link, the video (2,542 kbit/s, 25 frames and one Group a second) and how late a frame is counted are those of the record before. One viewer, the Rust client, which takes 750 Objects or stops when its time is over.

## Observed results

Two runs each; one run for the rows marked so. "Frames that came" is of the Groups between the viewer's first and last. Without the flag every frame comes, late.

| Link | Control | Patience | Median late, ms | 95th, ms | Streams reset | Frames that came |
|---|---|---|---|---|---|---|
| 3 Mbit/s | Cubic | 500 (1 run) | 84 | 141 | 0 | all |
| 3 Mbit/s | BBR | 500 (1 run) | 85 | 331 | 0 | all |
| 2.5 Mbit/s | Cubic | none | 567–574, growing | 1,329–1,343 | | all |
| 2.5 Mbit/s | Cubic | 500 | 429–435 | 602–603 | 26–27 of 31 | 96% |
| 2 Mbit/s | Cubic | none | 3,960–3,967, growing | 11,932–12,014 | | all |
| 2 Mbit/s | Cubic | 500 | 550–552 | 795–807 | 40 of 40 | 74–75% |
| 2 Mbit/s | Cubic | 200 (1 run) | 405 | 543 | 40 of 40 | 74% |
| 2.5 Mbit/s | BBR | none | 1,105–2,528 | 3,172–6,607 | | all |
| 2.5 Mbit/s | BBR | 500 | 809–829 | 1,142–1,191 | 39–41 | 59–63%; 1–3 Groups with no frame |
| 2 Mbit/s | BBR | none | 7,552–7,701 | 13,655–13,994 | | all |
| 2 Mbit/s | BBR | 500 | (534–1,083) | | 43 of 43 | **20 and 28 Objects of the first two or three Groups, then none to the end of the run** |
| 2 Mbit/s | BBR | 200 (1 run) | 735 | 1,003 | 43 of 43 | 36%; 4 Groups with no frame |

- **With Cubic it does what it is for.** The video stays about half a second behind and no longer falls further; at 2 Mbit/s every Group loses its last 6 to 8 frames, at 2.5 Mbit/s about one. The lateness does not grow from quarter to quarter of a run (548/551/558/549 ms at 2 Mbit/s).
- **With BBR it makes things worse.** BBR sends more than the link takes whether or not a stream was given up (9,000 to 24,000 datagrams dropped by the queue in a run), so the streams after a reset are late as well; at 2 Mbit/s the viewer got nothing after the first two or three Groups, in both runs, while the relay reset 43 streams.
- With room to spare nothing is reset.
- **The Rust viewer goes on** after a reset stream (`STREAM_ABORTED uni#N code=0x2` in its log) with the Groups after it, and its subscription stays.
- What comes of a Group that is cut is its beginning: the key frame and the frames after it, in order.

## Not measured

- A page as the viewer (the demo), and what its decoder shows when the end of a Group is missing.
- Sound. A track of sound would be given up by the same rule.
- More than one viewer, a link whose rate changes, a real network.
- Other shapes of giving up: by the age of an Object (which needs a small send window), or giving up a Group as soon as the next one begins.
- NewReno.

## Next action

The question stays open. What a decision needs besides this: whether the patience is the relay's own, or comes from the parameters of §8; and what a page does with a Group without its end.
