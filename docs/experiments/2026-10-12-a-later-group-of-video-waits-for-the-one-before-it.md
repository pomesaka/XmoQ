# A later Group of video waits for the end of the one before it: tried, and not put in

- Date: 2026-10-12
- Commit SHA: the commit that adds this record; the change that was measured is not in it
- Roadmap item: Phase 1, validation over a link that loses packets

## Objective

The receiver of video begins a later Group as soon as its Object 0 is there, and drops what comes of the Group before it afterwards (`late`), though it was not lost. Sound waits for what is missing as long as the buffer leaves (`holdForOrderMs`, `in-order.ts`). Does the same for video show more frames on a link with delay and loss?

Expected, written before the runs: fewer Objects dropped as `late`, more frames shown, the picture no later; little to see at 1% of loss.

## What was tried

`createVideoReceiver` and `playVideoTrack` with `holdForOrderMs`: a later Group whose Object 0 is there waits that long, from when the Object 0 came, for the rest of the Group that is played; a timer of the player lets it go. So that a Group that is whole is not waited for, a later Group did not wait where its Object 0 followed in time: within one and a half times the smallest distance seen between two frames handed on, the rule of `in-order.ts`. The watching page set it to its buffer less 20 ms. Seven table tests of the rule passed. Nothing waited when the setting was left out.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback; publisher, relay and viewer on the one machine |
| Viewer | Chromium 156 of playwright-core 1.64.0, headless; the watching page, buffer 150 ms and 300 ms, the sound put in order in every run |
| Relay | own, `--congestion bbr` |
| Link | `spikes/lossy-link/proxy.mjs` between the viewer and the relay, 50 ms each way, loss 1% and 5% each way |
| Picture | "clock": the demo's, about 450 kbit/s. "busy": `?picture=busy&bitrate=2500000`. 30 frames a second, a Group a second on one stream |

Each run: one viewer for 20 s (600 frames). "before" is the receiver as it is in `main`.

## Result

Frames shown of 600, and in brackets the Objects dropped as `late` / frames shown after their time. One run each.

| Buffer | Picture | Loss | before | after |
|---|---|---|---|---|
| 150 ms | clock | 1% | 595 (5/0), 603 (0/0), 598 (5/1) | 602 (0/2), 596 (0/4), 602 (0/7) |
| 150 ms | busy | 1% | 591 (9/0), 594 (7/1), 598 (4/1) | 599 (3/8), 599 (0/5), 592 (0/8) |
| 150 ms | clock | 5% | 592 (11/0), 589 (2/1), 575 (16/2) | 557 (8/9), 582 (0/8), 566 (19/9) |
| 150 ms | busy | 5% | 554 (14/12), 566 (10/12), 569 (13/6) | 564 (0/17), 534 (32/19), 533 (19/20) |
| 300 ms | clock | 1% | 595 (2/0), 598 (0/0) | 598 (0/7), 598 (0/5) |
| 300 ms | busy | 1% | 591 (7/0), 594 (4/0) | 598 (0/7), 592 (0/8) |
| 300 ms | clock | 5% | 574 (19/2), 589 (8/0) | 575 (18/4), 588 (0/5) |
| 300 ms | busy | 5% | 567 (18/0), 589 (9/0) | 465 (54/5), 540 (38/8) |

- **At 1% the Objects dropped as `late` went from 0 to 9 a run to 0 to 3, and the frames shown did not change** (596 before, 598 after, the mean of ten runs each). In their place 2 to 8 frames a run were shown after their time, where it was 0 or 1 before.
- **At 5% no more frames were shown, and with the busy picture fewer** (569 before and 527 after, the mean of five runs each). The Group that had waited came out at once when the wait was over, the decoder had more than 8 chunks waiting (`decoder-behind`, up to 4 times a run with that picture, never before), and the rest of that Group was given up.
- The time a frame took to the page did not change (53 to 57 ms, and 121 to 130 ms with the busy picture at 5%).
- A larger buffer did not help: 300 ms is as 150 ms.

So the expectation held for the drops at 1% alone, and not for the frames shown.

## Not looked into

- Why frames were shown after their time at 1% in every run after the change, also with 300 ms. Suspected and not checked: a Group that was whole was waited for, where a frame was left out before the Group's end and the next Object 0 was more than one and a half steps away.
- Reading where a Group ends from the stream (the END_OF_GROUP bit, or the FIN), which would tell a whole Group without the times.
- Letting the decoder take the Group that waited without counting it as behind.

## Decision

Not put in: the receiver is as before. What a later try needs is above.
