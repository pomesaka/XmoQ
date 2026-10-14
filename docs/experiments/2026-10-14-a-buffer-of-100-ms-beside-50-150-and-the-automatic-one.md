# A buffer of 100 ms beside 50, 150 and the automatic one

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2; the open decision on the default of the playout buffer ([open decisions](../open-questions.md)). Measurements for that decision. No code is changed.

## Objective

[The record of the default buffer](2026-10-13-the-default-playout-buffer.md) compared 50 ms, 150 ms and the automatic buffer with ten pieces of sound a Group. The default is one a Group now, and [measured again](2026-10-14-a-piece-of-sound-a-stream-measured-again.md) it has more breaks at 1% loss with 50 ms. Here 100 ms is added, and the four are run with one a Group: pieces too late, breaks, how long after it was made a frame is shown, and where the automatic buffer stands.

## Expected, written before the runs

- **Clean link (50 ms each way).** None too late and no breaks with any of the four. A frame is shown the buffer and about 57 ms after it was made: about 107, 157 and 207 ms; the automatic one stands at 30 to 40 ms and shows a frame about 90 ms after.
- **1% loss.** A piece behind a lost packet comes a round trip later or more, 100 ms and up. 50 ms: 50 to 90 of 1,000 too late and 55 to 60 breaks (as this morning). 100 ms: between, 10 to 40 too late and 10 to 30 breaks. 150 ms: 0 to 10, 0 to 7. Automatic: under 10 too late and about 5 breaks, standing at 110 to 160 ms.
- **Narrow link (2,000 kbit/s, queue 100 ms, the busy picture at 2.5 Mbit/s).** 50 ms: about 70 of 100 too late. 150 ms and automatic: none after the first seconds, the automatic one at its limit of 400 ms. 100 ms: some, under a fifth. A frame is shown about 380 ms after it was made with the fixed ones (the link's queue), about 450 with the automatic one.

## Environment

As in [the record of this morning](2026-10-14-a-piece-of-sound-a-stream-measured-again.md): macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the demo from `main`, ports 45381, 45382, 45384; the own relay with no flag, Cubic; `spikes/lossy-link/proxy.mjs` between relay and viewer, 50 ms each way; one piece of sound a Group (the default). The viewer asks its default timeout for the video and none for the sound. 20 s counted after 5.5 s of playing, the buffer read every second; three runs of each, the four buffers in turn.

## Result

Of 1,000 pieces of sound and 600 frames in 20 s; in each cell the three runs in order. "Buffer stood" is the least and the most of the 20 readings.

| Link | Buffer | Too late | Breaks | Frames shown | Frame shown after it was made, ms | Buffer stood, ms |
|---|---|---|---|---|---|---|
| clean | 50 | 0, 0, 0 | 0, 0, 0 | 606, 606, 606 | 103, 109, 105 | 50 |
| clean | 100 | 0, 0, 0 | 0, 0, 0 | 606, 607, 606 | 156, 158, 159 | 100 |
| clean | 150 | 0, 0, 0 | 0, 0, 0 | 606, 606, 606 | 208, 209, 209 | 150 |
| clean | automatic | 0, 0, 0 | 0, 0, 0 | 606, 607, 606 | 97, 97, 97 | 40 |
| 1% loss | 50 | 18, 66, 31 | 24, 39, 39 | 576, 550, 561 | 108 to 110 | 50 |
| 1% loss | 100 | 15, 16, 15 | 16, 14, 26 | 553, 584, 585 | 158 to 159 | 100 |
| 1% loss | 150 | 0, 0, 1 | 2, 1, 2 | 605, 604, 600 | 207 to 209 | 150 |
| 1% loss | automatic | 1, 4, 2 | 7, 1, 4 | 601, 563, 566 | 224, 458, 258 | 150 to 190, 90 to 400, 100 to 370 |
| narrow | 50 | 757, 747, 764 | 93, 98, 83 | 417, 418, 412 | 381 to 391 | 50 |
| narrow | 100 | 6, 2, 12 | 8, 2, 18 | 418, 417, 415 | 384 to 388 | 100 |
| narrow | 150 | 0, 0, 0 | 0, 0, 0 | 423, 422, 417 | 370 to 384 | 150 |
| narrow | automatic | 0, 0, 0 | 0, 0, 0 | 435, 434, 436 | 474 to 478 | 400 |

- **100 ms is between, nearer 150 ms on the narrow link and nearer 50 ms at 1% loss**: 15 or 16 too late and 14 to 26 breaks in 20 s at 1% loss (about one a second), where 150 ms has 0 or 1 and 1 or 2. On the narrow link 2 to 12 too late and 2 to 18 breaks, where 150 ms has none. On the clean link a frame is shown 50 ms sooner than with 150 ms (about 157 ms against 208).
- As expected for 100 ms at 1% loss (10 to 40, 10 to 30) and on the narrow link (under a fifth); fewer there than expected.
- **50 ms at 1% loss did better than this morning**: 18 to 66 too late and 24 to 39 breaks, where the record of this morning had 49 to 91 and 54 to 62 with the same code and link. What three runs say of 50 ms at 1% loss is therefore only "tens too late and tens of breaks in 20 s"; the difference between one and ten a stream there is within it.
- **The automatic one went to 400 ms on a link that lost 1% in one run of three** (from 90 ms), and 370 ms in another, where 110 to 160 ms were expected: a frame was shown 458 ms after it was made. On the clean link it stood at 40 ms, sooner than 50 ms. Behind the narrow link it was at its limit, 400 ms, the whole time, and whole.
- The frames shown are alike for the fixed ones on the narrow link (412 to 423 of 600); the automatic one showed a few more (434 to 436).

**What the measurer recommends as the default: 150 ms.** It was the only fixed one with no more than 1 piece too late and 2 breaks on every link here; 100 ms saves 50 ms of delay and breaks the sound about once a second at 1% loss, and the automatic one went to 400 ms at 1% loss in one run of three and has still not been listened to. Also the more breaks of one a stream at 1% loss with 50 ms, of this morning, are within what three runs of it scatter here; they are no reason for or against one a stream. The cost of 150 ms is about 100 ms more delay than 50 ms on a clean link (208 against 105 ms). The decision is the project owner's.
