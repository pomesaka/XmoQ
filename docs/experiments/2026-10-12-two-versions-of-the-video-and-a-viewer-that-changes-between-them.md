# Two versions of the video, and a viewer that changes between them

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, "Measure latency, drops, stalls and recovery under loss". A trial; nothing is decided by it, and it is off unless asked for.

## Objective

A video that needs more room than the link has falls behind, and the relay's `--delivery-patience-ms` only leaves frames out. Another remedy: the publisher offers the picture a second time, smaller, and the viewer plays that while its link is narrow. Does that keep the picture moving and in time on a link whose room changes, against one version alone, and what does it cost?

Expected, written before the runs: with one version the picture falls behind at 2 and 1 Mbit/s and shows few frames; with two the viewer goes down within a few seconds of the link getting narrow, shows nearly every frame of the small version in time, tries the large one again every few seconds (a short standstill each time while the link is still narrow), and is back on the large one some seconds after the link has room again.

## What was built

- **Publisher** (`/publish?ladder=1`): the same frames are encoded twice, 640x360 at the bitrate asked for and 320x180 at 500 kbit/s, as the tracks `video` and `video-low`. In the catalog both have `altGroup` 1 (MSF-01 §5.2.12: "alternate versions of one-another ... A subscriber typically subscribes to one"), and `renderGroup` 1. §4.2 has such tracks "MUST be time-aligned": both encoders get each frame with the same time, the same first Group ID and the same key frame interval, and **a key frame one track is asked for is made on both** (`wantsKeyFrame` and `encode(frame, { keyFrame })` of `createVideoTrackEncoder`, new), so that equally numbered Groups begin with the same frame.
- **Viewer** (`/watch?abr=1`): plays the version with the highest bitrate of the alternate group, and every 250 ms asks `createQualitySwitch` of `@xmoq/media` (new, `src/core`, with table tests) which to play, with how much later than the fastest frame the frames of the last second came (their median). **Down** one version at once when that is over 300 ms or no frame came; **up** one when it was under 100 ms for 5 s; nothing for 2 s after a change; a try upwards that fails within 5 s doubles the wait before the next, up to 8 times. All values provisional.
- **A change is at a Group's start**: the page subscribes to the other version without a Joining FETCH and goes on playing the one before until the first frame of the new one comes, then cancels the one before. Both go into the one playout, by their times.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback; publisher, relay and viewer on the one machine |
| Viewer | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; buffer 150 ms |
| Relay | own, Cubic, without and with `--delivery-patience-ms 500` |
| Link | `spikes/lossy-link/proxy.mjs` between viewer and relay: 50 ms each way, no loss, `queue` 100 ms, `rate` 3,000 kbit/s for 15 s, 2,000 for 15 s, 1,000 for 15 s, 3,000 for 30 s (set while the run goes on, with `/set?rate=`) |
| Sent | `?picture=busy&bitrate=2500000`: about 2.65 Mbit/s at 640x360, 30 frames and a Group a second; the small version about 0.6 Mbit/s as encoded (1,870 KiB in 25 s); Opus, 67 kbit/s |

## Commands

```sh
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 [--delivery-patience-ms 500]
# /publish?picture=busy&bitrate=2500000[&ladder=1] and /watch?via=45386&buffer=150[&abr=1]; a script sets the rate and reads the page's report
```

## Result

Two runs each. Frames shown in each part of the run (of 450, 450, 450, 900), and how late the picture came at the part's end (the median of the last 200 frames).

| Relay | Versions | 3 Mbit/s | 2 Mbit/s | 1 Mbit/s | 3 Mbit/s again | Stood still (over 100 ms): times, in all, longest | Sound: pieces too late, breaks |
|---|---|---|---|---|---|---|---|
| no patience | one | 344, 437 | 123, 139 (347, 404 ms) | 20, 16 (481, 483 ms) | **49, 52 (920, 899 ms)** | 125 and 114 times, 56.7 and 54.4 s, 1.4 s | 1,110 and 1,103; 252 and 290 |
| no patience | two | 434, 437 | 407, 397 (62 ms) | 274, 286 (74, 72 ms) | 865, 861 | 53 and 42 times, 9.1 and 7.6 s, 0.5 s | 277 and 90; 59 and 49 |
| 500 ms | one | 422, 452 | 278, 285 (525, 527 ms) | 131, 141 (664, 683 ms) | 815, 846 (87, 126 ms) | 78 and 67 times, 12.6 and 10.7 s, 0.35 s | 14 and 19; 1 and 3 |
| 500 ms | two | 443, 431 | 401, 382 (60, 61 ms) | 400, 405 (70, 72 ms) | 901, 908 | 16 and 15 times, 3.0 and 3.2 s, 0.6 s | 18 and 0; 2 and 0 |

- **With one version and no patience the viewer does not come back** when the link has room again: 49 and 52 of 900 frames in the last 30 s, 0.9 s behind and falling, and the sound broken through the run (the sound shares the link with a video that fills it). With the relay's patience it comes back, having shown a third to two thirds of the frames while the link was narrow, half a second or more behind.
- **With two versions the picture stays in time** (60 to 75 ms behind on the small version) and nearly whole: 382 to 407 of 450 frames at 2 Mbit/s. At 1 Mbit/s it is whole with the relay's patience (400, 405) and not without (274, 286).
- **The changes.** 6 to 8 a run. Down 1 to 2 s after the link got narrow; the first frame of the other version 200 to 600 ms after the change was decided (twice 1.1 s), and the picture stood still for 0 to 580 ms around a change. Back on the large version 16 to 23 s after the link had room again, since the two tries upwards while it was narrow (at about 8 and 21 s into the narrow part) had made the wait 20 s; each of those tries cost a standstill of 0.2 to 0.5 s and two changes. In one run without patience the viewer went down again 6 s after it was back, and ended on the small version.
- **Not as expected: a change down in the first seconds of two of the four runs with two versions**, at 3 Mbit/s, and up again 6 s later. The video needs nearly all of 3 Mbit/s, and the first Groups came late.
- **Not as expected: without the relay's patience the tries upwards hurt more than a standstill**: at 1 Mbit/s the page showed 274 and 286 of 450 frames, and 90 to 277 pieces of sound came too late. What the relay had taken of the large version for the viewer still went over the link after the viewer had left that version. With patience this did not show.
- The decoder did not fail in any run; the session stayed.
- **CPU of the publishing page's browser** (all its processes, by `ps`, 20 s, two runs): 21% and 17% of one core with one encoder, 28% and 19% with two. Both encoders made the same number of frames and key frames (757 and 25). What the system's encoder does outside the browser's processes is not in these numbers.

## Not done

- Whether equally numbered Groups of the two tracks begin with the same frame was not read from the Objects; the encoders made the same count of key frames.
- More than one viewer. A viewer that subscribes makes the publisher begin a Group on both tracks when it is the relay's first subscriber of a track, which is every change here; with other viewers on the track the change waits for the next Group, up to a second.
- A third version, loss, BBR, a real network, a viewer in a terminal.
- A better judge than lateness: the tries upwards are blind, and what they cost is above. An estimate of the link's room (from what the transport says, or from how fast a Group came) was not tried.
- How the change looks: the small picture is drawn into the same canvas, scaled by the page's style. Nobody looked.

## Later the same day: two viewers, one behind a narrow link

Does a viewer whose link gets narrow change alone, and how long does a change take while somebody else watches? Expected, written before the runs: the viewer with room stays on the large version and misses nothing; the other goes down within a second or two, at once (nobody is on the small version, so the relay subscribes to it and the publisher begins a Group), and when it goes up it waits for the next Group of the large version, up to a second, playing the small one meanwhile.

As above, with the relay's `--delivery-patience-ms 500`; two viewers with `?abr=1`, one through the proxy (50 ms each way; no limit for 10 s, 2,000 kbit/s for 20 s, no limit for 25 s) and one straight to the relay. Two runs.

| Viewer | Room (300 frames) | 2 Mbit/s (600) | Room again (750) | Changes | Stood still (over 100 ms) |
|---|---|---|---|---|---|
| Narrow link | 303, 303 | 562, 568, on `video-low`, 53 and 59 ms behind | 757, 757, on `video` | 4 and 4 | 7 and 5 times, 1.5 and 1.2 s in all, 0.4 s at most |
| With room | 304, 303 | 606, 606 | 757, 757 | none | never |

- **The expectations held.** The viewer with room showed every frame and never stood still, its picture as late as before (within a millisecond), while the other changed four times.
- **Down** 1.2 s after the link got narrow, the first frame of the small version 300 to 360 ms after that was decided, the picture standing still for 230 and 300 ms.
- **Up while somebody else is on the large version takes longer**: at the try upwards 7.5 s into the narrow part, the first frame came after 1.3 s (the next Group of a track the relay already has), where it took 0.2 to 0.6 s with one viewer. The small version went on meanwhile; the picture stood still for 150 and 180 ms. The try failed, and the viewer was down again 2 s later (150 ms).
- **Up for good** 0.4 and 0.7 s after the link had room again, the first frame 180 and 420 ms after that, with no standstill.
- The publisher made 61 key frames on each track in 60 s: the changes of one viewer cost the other at most one or two key frames out of turn, when the small version got its first subscriber.
- The narrow viewer's sound: 22 and 23 pieces too late and 4 and 5 breaks in 55 s; the other's 0 pieces and 0 and 1 breaks.

Not done here: more viewers on either version, a viewer that leaves the small version as its last subscriber and comes back (the relay then asks the publisher anew), the relay without patience.
