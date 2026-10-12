# A delivery timeout, and a playout buffer, that follow how late the sound comes

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link that is narrow or loses packets. Two trials behind flags; nothing is decided, and the defaults are as they were (a fixed timeout of 150 ms on the video, a fixed buffer of 50 ms).
- **The code is not in `main`.** Neither trial did better than what is there, so only this record was taken. What "What was built" names (`createDeliveryTimeoutChooser`, `?videoTimeout=auto`, `autoFollows`, `?buffer=auto-sound`) does not exist in the repository.

## Objective

Two numbers of the watching page could follow the path, and the page does not know the path. What it can see is how much later than the fastest one each piece of sound comes: the sound is small, and nothing is asked to give it up.

1. **The delivery timeout on the video** (`?videoTimeout=auto`): what 95 of 100 pieces of sound of the last 10 seconds came within, from 50 to 500 ms ([where the rule is from](2026-10-12-which-delivery-timeout-a-viewer-asks-for-on-its-video.md)). Is it better than the fixed 150 ms?
2. **The automatic buffer** (`?buffer=auto`) goes by frames and pieces of sound together, and on a link narrower than the video it goes to its limit of 400 ms and stays (found with the viewer in a terminal, which has the same rule). Does it do better following the sound alone (`?buffer=auto-sound`)?

Expected, written before the runs. Of 1: on the narrow links it asks for less than 150 ms and the picture is a little nearer, with the same frames; under loss at a distance it asks for more and shows more frames than 150 ms does. Of 2: following the sound alone the buffer settles near the sound's lateness and not at the limit, and the picture is shown sooner.

## What was built

- `createDeliveryTimeoutChooser` (`@xmoq/media`, `src/core`, table tests): 150 ms until 10 seconds of sound are there; then the 95th of 100 of the sound's lateness over the last 10 s, in steps of 10 ms, from 50 to 500 ms; asked anew only where it changes by a quarter or more, and once in 10 seconds at most. **Every value provisional.**
- **Asking anew**: the relay takes no REQUEST_UPDATE for a subscription, **and no second SUBSCRIBE for a track from a session while the first is open** (DUPLICATE_SUBSCRIPTION, transport §5.1), so the way of the change of versions, where the new subscription overlaps the one before, is not open for the same track. The page ends its subscription to the video, waits for it to be over, and subscribes again with the new timeout and a Joining FETCH of the Group under way.
- `PlayoutClock.baseOf` takes whether an arrival counts for the automatic buffer, and `createPlayout` has `autoFollows: "sound"` (a trial; "both" when left out).

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag, Cubic, and for one row BBR; `spikes/lossy-link/proxy.mjs` between viewer and relay. Narrow: 2,000 kbit/s, queue 100 ms, the busy picture at 2.5 Mbit/s. Loss: 5% each way, no limit, the clock at 640x360. Sound: Opus, ten pieces a Group.

## Result

### 1. The timeout: fixed 150 ms against one that follows the sound

Buffer 150 ms. Each run is 40 s; counted are the 25 s from the fifteenth second on (750 frames, 1,250 pieces of sound), after the automatic one has had its 10 seconds. Three runs on the narrow links and with BBR, five with loss.

| Link | Each way | Timeout | Frames shown of 750 | Picture came after, ms | Asked for at the end, ms | Changes in 40 s | Sound: sounded of 1,250, breaks |
|---|---|---|---|---|---|---|---|
| 2 Mbit/s | 10 ms | fixed | 529, 525, 531 | 357, 349, 348 | 150 | | 1,242 to 1,245; 3 to 4 |
| 2 Mbit/s | 10 ms | auto | 530, 516, 532 | 292, 316, 298 | 60, 100, 60 | 1, 3, 1 | 1,236 to 1,250; 0 to 7 |
| 2 Mbit/s | 50 ms | fixed | 524, 527, 523 | 362, 364, 375 | 150 | | 1,249 to 1,250; 0 |
| 2 Mbit/s | 50 ms | auto | 521, 529, 529 | 350, 362, 339 | 100, 110, 100 | 1, 1, 1 | 1,250 to 1,251; 0 |
| 2 Mbit/s | 100 ms | fixed | 514, 503, 508 | 393, 409, 397 | 150 | | 1,245 to 1,250; 0 to 1 |
| 2 Mbit/s | 100 ms | auto | 506, 503, 504 | 373, 378, 384 | 100, 110, 110 | 1, 1, 1 | 1,245 to 1,251; 0 to 2 |
| 5% loss | 10 ms | fixed | 755 to 758 | 11 to 13 | 150 | | 1,250 to 1,251; 0 |
| 5% loss | 10 ms | auto | 756 to 758 | 13 to 15 | 50 (all five) | 1 each | 1,244 to 1,249; 0 to 2 |
| 5% loss | 50 ms | fixed | 440, 441, 524, 550, 567 | 98 to 146 | 150 | | 1,138 to 1,193; 12 to 27 |
| 5% loss | 50 ms | auto | 448, 503, 465, 439, 461 | 94 to 182 | 140 to 190 | 0, 0, 2, 3, 2 | 1,150 to 1,173; 19 to 33 |
| 5% loss | 100 ms | fixed | 112, 119, 113, 116, 113 | 404 to 546 | 150 | | 666 to 776; 125 to 139 |
| 5% loss | 100 ms | auto | 132, 143, 156, 119, 116 | 389 to 506 | 200 to 300 | 1, 1, 2, 1, 1 | 733 to 804; 122 to 148 |
| 5% loss, BBR | 100 ms | fixed | 611, 614, 551 | 105, 105, 105 | 150 | | 948 to 1,014; 73 to 187 |
| 5% loss, BBR | 100 ms | auto | 551, 618, 609 | 118, 105, 106 | 220, 200, 210 | 1, 1, 1 | 945 to 992; 68 to 97 |

- **On the narrow links, as expected**: it asks for 60 to 110 ms, shows the same frames, and the picture is 15 to 55 ms nearer.
- **Under loss, not as expected: it shows no more frames.** It asks for more at a distance (140 to 190 ms at 50 ms each way, 200 to 300 at 100), as the round trip is longer, and the frames are within what the runs of the fixed timeout differ by; at 50 ms each way the fixed 150 ms has the two best runs. With Cubic at 100 ms each way both show next to nothing (the congestion control's matter); with BBR both show 551 to 618 of 750.
- **What it costs**: every change ends the subscription and makes it anew. The first frame of the new one came 90 to 880 ms after the change was decided (200 to 500 ms in most), and there were up to three changes in 40 s where the sound's lateness moves about the quarter that makes a change (5% loss at 50 ms each way: 190, 140, 180).
- **Found on the way**: the first version of the page closed its session at the first change. It took the end of the subscription it had ended itself for the end of the track. Mended before the runs above.

### 2. The buffer: frames and sound, the sound alone, and fixed

The video asks for its default timeout of 150 ms; 50 ms each way; 25 s counted after 6.5 s of playing; three runs each. The buffer is read every 5 s.

| Link | Buffer | Where the buffer stood, ms | Frames shown of 750 | Picture shown after, ms (came after) | Frames after their turn | Sound: too late, breaks |
|---|---|---|---|---|---|---|
| 2 Mbit/s | auto, both | 400 throughout (all three) | 548, 545, 541 | 482, 476, 486 (373) | 52, 53, 46 | 0; 0 |
| 2 Mbit/s | auto, the sound | 110 (230 at the first reading in two runs) | 522, 522, 535 | 384, 389, 370 (376) | 522, 504, 514 | 0; 0 |
| 2 Mbit/s | fixed 150 ms | 150 | 522, 526, 526 | 385, 372, 380 (373) | all | 0; 0 |
| 5% loss | auto, both | 360 to 400 | 615, 572, 604 | 460, 457, 459 (181 to 277) | 27, 32, 22 | 0 to 6; 0 to 5 |
| 5% loss | auto, the sound | 160 to 400, moving | 526, 542, 584 | 343, 436, 280 (186 to 236) | 71, 67, 64 | 16 to 24; 7 to 10 |
| 5% loss | fixed 150 ms | 150 | 485, 425, 351 | 214, 216, 217 (173 to 197) | 144, 150, 160 | 62 to 130; 23 to 41 |

- **Following the sound alone the buffer does not go to its limit on the narrow link**: 110 ms, which is what 99 of 100 pieces of sound come within there, and 10 ms more. As expected.
- **But the picture is not shown sooner by it than with any buffer shorter than the picture's own lateness, and it is no longer with the sound.** On that link a frame comes about 375 ms after it was made whatever is given up of it. With the buffer at 110 or 150 ms every frame is past its turn when it comes and is shown at once, about 380 ms after it was made, while the sound plays 110 or 150 ms after: **picture and sound are 230 to 270 ms apart**. With the buffer at 400 ms both wait for the picture: nearly every frame is in time, picture and sound are together, and both are about 480 ms behind. The buffer that follows everything is at its limit there because the picture is that late; that is what it is for, and it is also why it is not live.
- **Under loss both automatic buffers do better than the fixed 150 ms**, which loses 62 to 130 pieces of sound and has 23 to 41 breaks; the one that follows the sound moves more (160 to 400 ms) and leaves 7 to 10 breaks, the one that follows both stays near its limit and leaves 0 to 5.
- So the two rules answer different questions: how long to wait for the sound, and how long to wait for the picture. One number for both cannot do both on a link where the picture is late and the sound is not.

### The change of versions with the default timeout

`?ladder=1` and `?abr=1` with the page's default timeout of 150 ms, the relay with no flag, 3, 2, 1 and 3 Mbit/s as in [the record of the two versions](2026-10-12-two-versions-of-the-video-and-a-viewer-that-changes-between-them.md); two runs. Down 2 and 4 s after the link got narrow, where it was 1 to 2 s without a timeout: the picture stays under the 300 ms the change goes by for longer. 415 and 397 of 450 frames at 2 Mbit/s, 404 and 401 at 1 Mbit/s, back on the large version 16 and 17 s after the link had room, the picture standing still for 2.0 and 2.5 s in all, 2 and 3 breaks in the sound: as good as any of the runs before, or better. The judge was not changed. If the 2 to 4 s matter: a lower mark than 300 ms, or counting the Groups the relay gave up (a stream reset with DELIVERY_TIMEOUT), which is the link saying the same thing sooner.

`mise run check` and `mise run interop` (127 of 127) passed; `bun demo/check.ts` says what it said.

## What this says, and what it does not

- **The timeout that follows the sound is not worth its cost on what was run**: the fixed 150 ms shows as many frames everywhere, the gain is a picture 15 to 55 ms nearer on a narrow link, and every change is a pause. It would be another matter with a REQUEST_UPDATE that changes the timeout without ending the subscription, which the relay does not take.
- **For the buffer, "the sound alone" is not simply better.** It keeps the sound near and lets the picture trail on a narrow link; "both" keeps them together and late. Which is wanted is a matter of what the viewer is for (a talk is the sound; a game is the picture), and could be two buffers that are allowed to differ by a limit. Nothing of that was built. The rule of the two implementations and `fixtures/playout/` is as it was.
- Not known: any of it with a real network, with a viewer in a terminal, with a person watching and listening; the timeout's rule with other windows or shares.
