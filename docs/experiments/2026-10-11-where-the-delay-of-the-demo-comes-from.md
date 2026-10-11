# Where the delay of the demo comes from, and a buffer that follows the link

- Date: 2026-10-11
- Commit SHA: the commit that changes `demo/watch.ts`, together with this record
- Roadmap item: Phase 1, the playout buffer (its size is undecided; 150 ms in the demo is provisional)

## Objective

A person who watched the demo expected it to feel more live, and asked where the delay comes from. Show the parts on the page, let the buffer be moved while watching, and try one buffer that sets itself: is the delay smaller on a good link, and what happens on a bad one?

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback; publisher, relay and viewers on the one machine |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Link | `spikes/lossy-link/proxy.mjs` between the viewers and the relay: delay and loss each way |
| Picture and sound | the clock of the demo, 30 frames a second; Opus, 10 pieces of 20 ms in a Group |

## Commands

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
# scripts of this run: the buttons of the page pressed in turn, 8 s each; and for each link, viewers with
# ?buffer=150, ?buffer=auto, ?buffer=auto&autoShare=0.95, ?buffer=auto&autoShare=0.9&autoMax=250
# side by side through the link, 12 s to settle and 10 s measured
bun demo/check.ts http://localhost:45381
mise run check
```

## Observed results

**The parts, on one machine with no link between** (medians of 200 frames, milliseconds; buttons of the page):

| Buffer | To get here (encoder, link, relay, decoder) | Waiting in the buffer | For the next repaint | In all | Frames after their turn, of about 210 | Pieces of sound too late, of about 350 |
|---|---|---|---|---|---|---|
| 150 ms | 1 | 131 | 9 | 141 | 0 | 0 |
| 50 ms | 1 | 31 | 7 | 39 | 0 | 0 |
| 0 | 1 | 0 | 7 | 9 | 212 | **343: no sound** |
| auto (settled at 40 ms) | 1 | 21 | 9 | 31 | 2 | 0 |

- **Nearly all of the delay is the buffer.** The buffer waits about 20 ms less than its number, because it counts from the fastest arrival.
- With no buffer the picture is drawn as it comes; the sound is silent, because a piece is given to the loudspeaker for a time that is over when it comes.
- 99 of 100 frames and pieces came within 17 to 24 ms of the fastest.

**The automatic buffer** (a trial): what 99 of 100 of the last 10 seconds came within, and 10 ms more; it grows at once and shrinks 10 ms twice a second. Four viewers at once through the link, 10 seconds each (about 300 frames, 500 pieces of sound):

| Link (each way) | Buffer | Settled at | Delay of the picture | Frames shown | Frames after their turn | Pictures that stood still over 100 ms | Sound: pieces sounded |
|---|---|---|---|---|---|---|---|
| 50 ms | fixed 150 | 150 | 192 | 303 | 0 | 0 | 500 |
| 50 ms | auto | 40 | **81** | 303 | 0 | 0 | 500 |
| 50 ms, 1 % loss | fixed 150 | 150 | 199 | 295 | 5 | 2 | **0** |
| 50 ms, 1 % loss | auto, 400 at most | 400 | 449 | 303 | 0 | 0 | **0** |
| 50 ms, 1 % loss | auto, 95 of 100 | 240 | 282 | 292 | 4 | 1 | 322 |
| 50 ms, 5 % loss | fixed 150 | 150 | 215 | 191 | 70 | 24 | 0 |
| 50 ms, 5 % loss | auto, 400 at most | 400 | 447 | 255 | 14 | 6 | 0 |
| 50 ms, 1 % loss, 10 ms jitter | fixed 150 | 150 | 201 | 241 | 47 | 12 | 0 |
| 50 ms, 1 % loss, 10 ms jitter | auto, 400 at most | 400 | 449 | 226 | 44 | 17 | 0 |
| 50 ms, 1 % loss, 10 ms jitter | auto, 95 of 100 | 400 | 447 | 302 | 0 | 0 | 0 |

- **On a link without loss the automatic buffer more than halves the delay** (192 to 81 ms at 50 ms each way; 147 to 27 ms with no link), with nothing late.
- **With loss it does not hold.** What 99 of 100 came within was 0.8 to 1 second at 1 % and 5 to 6 seconds at 5 %: a first run without a limit set the buffer to 810 ms, 1,260 ms and 5,350 ms. With the limit of 400 ms it sits at the limit, and the picture is better at 5 % (255 frames against 191) and no better with jitter.
- **With loss there was no sound at nearly any buffer up to 400 ms**: every piece came after its time. This is the lateness of the sound under loss that was found in Chromium before; 10 pieces in a Group did not keep it away here, with 50 ms each way. Not looked into further here.
- The rows are one run each, with four viewers sharing the link and the machine: the differences between the automatic kinds are within what one run says.

**The camera**: `VideoFrame.metadata().captureTime` would say when the camera took a frame. With Chromium's fake camera it is not there, and the publishing page says "the browser does not say". A real camera was not tried.

## Limitations and blockers

- One machine, one run for each row, headless. The clocks of publisher and viewer are the same clock here; between two machines "to get here" holds the difference of the clocks, and only the buffer and the repaint are right.
- From the repaint to the screen is not measured, nor from the camera to the encoder.
- The automatic buffer is a trial: its window, margin, share and limit are not chosen by measurement, and each change of the buffer of more than 8 ms is a break in the sound.
- The buffer of 150 ms stays the demo's default; which default is right is the person's to say.
