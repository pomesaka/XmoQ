# A page behind a link with too little room, and a relay that gives Groups up

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, "Measure latency, drops, stalls and recovery under loss"; open question on whether a slow subscriber loses whole old Groups and stays subscribed

## Objective

[The relay's `--delivery-patience-ms`](2026-10-12-the-relay-gives-up-a-group-a-viewer-does-not-get-in-time.md) was run with the Rust client as the viewer. Here the viewer is the demo's watching page: how many frames it shows, how late, what the sound does, and what the decoder shows when the end of a Group does not come.

Expected, written before the runs: without the flag the picture falls further behind through the run and every frame comes; with 500 ms it stays about half a second behind, fewer frames are shown (about 96% at 2.5 Mbit/s and 75% at 2 Mbit/s, as the Rust viewer had), the picture stands still at the end of a Group that was cut and no broken picture is decoded, the session stays, and the sound has gaps where a stream of sound was given up.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback; publisher, relay and viewer on the one machine |
| Viewer | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the watching page, buffer 150 ms |
| Relay | own, Cubic, without and with `--delivery-patience-ms 500` |
| Link | `spikes/lossy-link/proxy.mjs` between the viewer and the relay: 50 ms each way, no loss, `rate` 2,500 and 2,000 kbit/s, `queue` 100 ms. The publisher goes to the relay directly |
| Sent | `/publish?picture=busy&bitrate=2500000`: 640x360, 30 frames and one Group a second, about 2.65 Mbit/s; Opus, about 67 kbit/s, ten pieces a Group |

## Commands

```sh
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 [--delivery-patience-ms 500]
curl 'http://127.0.0.1:45387/set?delay=50&rate=2500&queue=100'   # and rate=2000
# a script of this run: /watch?via=45386&buffer=150 for 20 s (600 frames, 1,000 pieces of sound), read from the page's report
```

## Result

Two runs each, 20 s. "Came after" is the median of the last 200 frames, read at 5, 10, 15 and 20 s.

| Link | Patience | Frames shown of 600 | Picture came after, ms | Stood still (longer than 100 ms) | Dropped by the page | Sound: pieces sounded of 1,000, breaks |
|---|---|---|---|---|---|---|
| 2.5 Mbit/s | none | 325, 329 | 350, 401, 469, 552 and 339, 391, 462, 553 | 22, 23 times, 267 ms at most | `late` 225, 215 | 979, 975; 1, 1 |
| 2.5 Mbit/s | 500 ms | 442, 445 | 355, 421, 433, 428 and 353, 405, 432, 430 | 5, 6 times, 250 ms at most | `late` 83, 84 | 974, 983; 0, 0 |
| 2 Mbit/s | none | 128, 128 | 513, 602, 669, 732 and 529, 631, 678, 743 | 69, 71 times, 549 ms at most | `late` 305, 310 | 965, 955; 1, 1 |
| 2 Mbit/s | 500 ms | 344, 350 | 493, 505, 516, 515 and 489, 511, 523, 522 | 21, 18 times, 251 ms at most | `late` 69, 68 | 970, 963; 0, 2 |

- **Not as expected: the page leaves frames out without the flag as well, and more of them.** Its receiver begins a Group when that Group's Object 0 is there and drops what comes of the Group before afterwards (`late`). On a link with too little room the streams of two Groups are under way together, so the end of every Group comes after the next has begun: 215 to 310 Objects a run were dropped by the page, after the link had carried them. The relay with the flag does not send them, and what the link carries instead is shown: **more frames with the flag, not fewer** (444 against 327 at 2.5 Mbit/s, 347 against 128 at 2 Mbit/s).
- **Without the flag the picture falls further behind through the run** (350 to 552 ms at 2.5 Mbit/s, 513 to 743 ms at 2 Mbit/s in 20 s), though slower than the Rust viewer's, which takes every frame. **With it the picture stays** about 430 ms behind at 2.5 Mbit/s and about 520 ms at 2 Mbit/s.
- **What the decoder shows when the end of a Group is missing: the picture stands still until the next key frame.** The receiver hands on the frames of a Group in order from Object 0, so the decoder gets the beginning of the Group whole and nothing after the cut. The decoder did not fail in any run, and with the flag the picture stood still less often and for 250 ms at most. No picture was looked at by eye; a broken one would need a frame whose reference is missing, which the receiver does not hand on.
- **The session and the subscriptions stay**: "playing (session 1)" at the end of all eight runs. The relay logged `DELIVERY_TIMED_OUT` 74 times in the four runs with the flag, and none without. Which of them were streams of sound was not counted.
- **The sound is as without the flag**: 955 to 983 of 1,000 pieces sounded either way, 0 to 2 breaks, and it came about 90 to 160 ms after it was made in every run. A stream of sound is small and was through in time. Sound and picture are apart by as much as the picture is behind: every frame shown came after its turn (150 ms buffer).
- With the flag the page still dropped about 70 to 85 Objects a run as `late`: what came of a Group after the next began and before the relay gave it up.

## Not done

- BBR (asked for Cubic alone), loss, more than one viewer, a link whose rate changes.
- A picture judged by eye, and what half a second of standing still at every Group's end looks like against a picture that is whole and seconds late.
- Giving a Group up as soon as the next one begins, at the relay: the page throws away what comes after that anyway, so it would save what the link carries of it (70 to 85 Objects a run here).
- A buffer that follows (`?buffer=auto`): with 150 ms every frame was late.

## What it says about the receiver

Nothing had to be changed in `@xmoq/media` for the flag: a stream that is reset ends its Group for the receiver as a Group that does not go on, and the next Group begins. The cost of the receiver's rule without the flag is in the first point above.
