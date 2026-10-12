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

## Later the same day: the relay gives a Group up when the next one begins

"Not done" above had it: the page throws away what comes of a Group after the next has begun, so a relay that stops sending it saves what the link carries of it. The relay has that now as `--older-groups give-up`, with `--older-groups-grace-ms` for how long the older Group may still go on (both off by default; #160, #168). The same run as above, two runs each. Expected, written before the runs: more frames than with the patience of 500 ms, since the Group is given up at once and not half a second after its end, a picture less far behind, and nothing dropped by the page as `late`; the sound as before.

| Relay | Link | Frames shown of 600 | Picture came after, ms (at 20 s) | Stood still (over 100 ms) | Dropped by the page as `late` | Sound: pieces sounded of 1,000, breaks |
|---|---|---|---|---|---|---|
| no flag | 2.5 Mbit/s | 328, 338 | 565, 570, growing | 23, 18 times | 219, 213 | 975, 981; 1, 0 |
| patience 500 ms | 2.5 Mbit/s | 444, 437 | 420, 413 | 6, 7 | 89, 84 | 974, 976; 2, 1 |
| give up, no grace | 2.5 Mbit/s | 475, 530 | 199, 181 | 19, 12 | 0, 0 | **870, 864; 50, 47** |
| give up, grace 100 ms | 2.5 Mbit/s | 518, 521 | 254, 251 | 9, 6 | 1, 0 | 970, 976; 2, 0 |
| no flag | 2 Mbit/s | 126, 129 | 726, 730, growing | 73, 71 | 308, 306 | 964, 954; 2, 3 |
| patience 500 ms | 2 Mbit/s | 352, 349 | 527, 521 | 20, 26 | 77, 62 | 969, 963; 2, 1 |
| give up, no grace | 2 Mbit/s | 377, 412 | 303, 303 | 27, 18 | 0, 0 | **898, 855; 39, 45** |
| give up, grace 100 ms | 2 Mbit/s | 415, 415 | 345, 349 | 12, 12 | 0, 0 | 966, 960; 0, 1 |

The rows without a flag and with patience are as they were in the morning (327 and 444 frames at 2.5 Mbit/s, 128 and 347 at 2 Mbit/s).

- **Giving older Groups up shows more frames and a picture less far behind than the patience does**: with 100 ms of grace 518 and 521 of 600 at 2.5 Mbit/s against 444 and 437, and 415 against about 350 at 2 Mbit/s; the picture 250 and 350 ms behind against 420 and 525. The page dropped nothing as `late`: what it would have thrown away was not sent.
- **Not as expected: with no grace the sound breaks.** 39 to 50 breaks and 855 to 898 pieces sounded of 1,000, where every other row has 0 to 3 breaks and 954 or more. The rule holds for every track, and a Group of sound is ten pieces on one stream: when the next Group begins, the end of the one before is often not through, and is given up. With 100 ms of grace the sound is as without the flag.
- With no grace the picture is nearer still (about 190 and 300 ms) and the frames are fewer and less even from run to run (475 and 530; 377 and 412).
- The picture still stands still at the end of a Group that was cut, 6 to 12 times in 20 s with the grace, 250 ms at most.

**What can be said from this.** On a link narrower than the video, the three experiments rank the same at both rates: giving older Groups up with a grace of 100 ms shows the most frames and keeps the sound; the patience of 500 ms shows fewer and is further behind; nothing shows the fewest and falls behind without end. Giving up with no grace is the nearest to live and breaks the sound, as long as sound is ten pieces a Group. What is not known: any of them with loss in place of a narrow link, where a Group that is late is late for another reason; with BBR; with more than one viewer; and what a picture that stands still several times in 20 s looks like against one that is whole and late. With `?abr=1` on top, one run is in [the record of the two versions](2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md): the viewer went down later, as the picture was less far behind.
