# The demo plays picture and sound together, and shows the delay

- Date: 2026-10-11
- Commit SHA: the commit that changes the pages of the demo, together with this record; the runs were made on its working copy
- Roadmap item: Phase 2, a buffer for playback and audio with video

## Objective

Measure how long a frame and a piece of audio take from the publisher's clock to the watching page, and play the two tracks together by the times their Objects carry, with a small buffer. In the demo only; nothing of `@xmoq/media` changed.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the two pages in one browser, so on one clock |
| Relay | own |

## Commands

```sh
mise run check
bun demo/serve.ts --port 45381 --relay-port 45382
# a script outside the repository opened /publish, and /watch with four buffers, 9 seconds each, and clicked "sound"
```

## What the pages do

- **The publisher marks a moment in both tracks**: for the first 200 ms of every 2 seconds of its clock the picture has a white square and the sound is a beep of 0.6 of full scale, where it is 0.1 else. The sound is made by the clock (as many pieces of 20 ms as have passed), not one a tick of a timer.
- **The viewer plays by the times of the Objects.** It takes the smallest difference between its clock and a time that came as "no delay", and plays everything `buffer` milliseconds after that: frames wait in a queue and the newest one that is due is drawn at a repaint (`requestAnimationFrame`); a piece of audio is given to the `AudioContext` for its time, or left out when that is past.
- It notes when the square comes on in the frames it draws and when the beep begins in the pieces it schedules, and pairs the two.

## Observed results

Nine seconds each, 286 frames (30 a second) and 474 pieces of audio.

| Buffer | A frame came after it was made | A piece came after | Shown after | Sounded after | Pieces too late | The picture after the sound, at the 5 marks |
|---|---|---|---|---|---|---|
| 150 ms (the default) | median -1 ms, at most 18 | median -8, at most 3 | median 148, at most 157 | 139 | 0 | 4, -7, 25, -9, 8 ms |
| 0 | median 0, at most 20 | median -8, at most 5 | median 9, at most 30 | — | 473 of 475 | (not sounded) |
| 50 ms | median 0, at most 20 | median -8, at most 5 | median 48, at most 58 | 40 | 0 | -25, 5, 7, -10, 5 ms |
| 300 ms | median 0, at most 11 | median -7, at most 5 | median 297, at most 307 | 290 | 0 | 19, 2, 3, -15, 19 ms |

- **Picture and sound came out within 25 ms of each other** at every mark, with a buffer of 50 ms or more. As they were sent, the two marks were -37 to 13 ms apart already: a frame is 33 ms and a piece 20 ms, and the mark comes on with whichever is made next.
- **On loopback a frame is there within 20 ms of the time it carries, and mostly at once.** The decode is in that; the time a frame carries is when it was handed to the encoder.
- **The audio "came" 8 ms before it was made.** The times of the tone are counted, 20 ms a piece from the first, and a piece is handed to the encoder at the start of its 20 ms; with a microphone the time would be the capture's. So the two tracks are about 8 ms apart by construction here, which is in the numbers above.
- Without a buffer nearly every piece of audio is past its time when it is decoded, and is not sounded.

## Limitations and blockers

- **Nobody saw or heard it.** The numbers are the page's own: when it drew, and for when it scheduled. What the screen and the loudspeaker add (`AudioContext.outputLatency`, the display) is not in them.
- One machine, one clock, loopback, headless; 9 seconds a run, one run each.
- The default of 150 ms is a guess, as is everything about the buffer: it does not adapt, a frame that is late is drawn late, a piece that is late is left out.
- The clock offset is the smallest difference seen, and is never raised again: a viewer whose clock drifts, or whose fastest Object was unusually fast, plays later than `buffer` says.
- No camera and no microphone.

## Open questions raised

- How long the buffer is, and whether it adapts.
- What is played when audio is late: left out, as here, or stretched.
- Which clock leads: here both follow the publisher's times; a player usually lets the audio lead.
- Where this belongs: it is in the pages of the demo, and `@xmoq/media` has no buffer.
