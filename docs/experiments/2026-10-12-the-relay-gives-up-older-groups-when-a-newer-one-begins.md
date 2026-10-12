# The relay gives up the older Groups when a newer one begins

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay (latency); open question on whether a slow subscriber loses whole old Groups and stays subscribed

## Objective

[The first try](2026-10-12-the-relay-gives-up-a-group-a-viewer-does-not-get-in-time.md) gave a Group up when the viewer did not have it 500 ms after its last Object. A page still dropped 70 to 85 frames a run as late with it: the page leaves the rest of a Group out as soon as Object 0 of the next has come. Here the relay goes by the same event: another shape to compare, nothing decided.

## What was built

`xmoq-relay --older-groups give-up`, `keep` without the flag. When the relay opens a subgroup stream of a Group towards a subscriber, the streams of the **older Groups of that track** towards that subscriber that the subscriber does not have all of yet are reset with `DELIVERY_TIMEOUT` (0x2), and the log has `GROUP_GIVEN_UP session= out= group= for_group=`. The subscription goes on.

- It holds for every track: a video with a Group on a stream, and a sound with ten Objects on a stream. Streams of the same Group are left alone, so a mapping with an Object on a stream loses nothing of the Group that is under way.
- For a reset to reach a stream that was written whole, the writer keeps such a stream until the receiver's transport has acknowledged all of it (`SendLimits::held_until_delivered`, on with the flag alone). Without that a stream that is written is the transport's: up to 4 MiB of it wait there, and no reset reaches them.
- The draft: §8, "Publishers can, at their discretion, discontinue forwarding Objects before either of the timeouts occurs, subject to stream closure and ordering constraints described in Section 11.4.3", and §11.4.3, "If a sender closes the stream before delivering all such objects to the QUIC stream, it MUST reset the stream". **Which code such a reset has, the draft does not say**: `DELIVERY_TIMEOUT` is "A delivery timeout (Section 8) was exceeded for this stream" and no timeout was set here; `CANCELLED` is "The stream was cancelled by either endpoint". `DELIVERY_TIMEOUT` was taken, as in the first try; it is an open point.
- No test covers it; it was run over QUIC, below.

## Commands

```sh
[PATIENCE_MS=500 | OLDER_GROUPS=give-up] CONGESTION=cubic RATE=<kbit/s> QUEUE=100 spikes/latency-loss/run.sh 50 0 fan-720-2500k.h264 group 750 1
```

The link, the video (2,542 kbit/s, 25 frames and one Group a second) and the counting are those of the first try. One Rust viewer, Cubic, 50 ms each way, a queue of 100 ms.

## Observed results

Two runs each. "Frames that came" is of the Groups between the viewer's first and last.

| Link | Rule | Median late, ms | 95th, ms | Latest, ms | Frames that came |
|---|---|---|---|---|---|
| 2.5 Mbit/s | none | 599 and 2,035, growing | 1,497–3,799 | 2,040–4,522 | all, late |
| 2.5 Mbit/s | 500 ms after the last Object | 433–436 | 603 | 684–701 | 95–96% |
| 2.5 Mbit/s | when a newer Group begins | 214–215 | 257–263 | 431–450 | 93–94% |
| 2 Mbit/s | none | 3,934–3,939, growing | 11,960–12,002 | 16,068–16,134 | all, late |
| 2 Mbit/s | 500 ms after the last Object | 554–556 | 797–805 | 886–896 | 74–75% |
| 2 Mbit/s | when a newer Group begins | 338–339 | 419–424 | 492–520 | 72–74% |

- **The new rule halves the lateness of the first one and loses about as many frames**: what the first one sends in its 500 ms is the end of a Group that the viewer then gets late, and the beginning of the next Group waits behind it.
- No Group came without its beginning in any run; what is lost is the end of a Group.
- **With room to spare the relay resets every Group all the same** (30 of 30 at 3 Mbit/s and without a limit, one run each): when the next Group begins, the last frame of the one before was written 40 ms ago and cannot be acknowledged yet over a path of 100 ms there and back. The viewer got every frame and saw no reset in those runs (the data and the FIN were there before the reset). A packet of that last frame that is lost would not be sent again, so on a path that loses a little this rule costs frames the first one would not. Not measured.

## Not measured

- A page as the viewer, and the demo: `demo/serve.ts` has no way to pass the flag yet.
- Sound, several viewers, BBR, a path that loses.
- A rule in between: giving the older Group a short time after the newer one begins.

## Next action

The question stays open, and so does the code of such a reset.
