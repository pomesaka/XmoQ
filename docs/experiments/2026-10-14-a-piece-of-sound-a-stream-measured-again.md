# A piece of sound a stream, measured beside ten

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2; the demo's numbers after the default became one chunk of sound a Group and a stream (decided on 2026-10-13). No code is changed.

## Objective

The numbers of [the demo's summary](../av-demo.md) were taken with ten chunks of sound a Group. Here the same page and viewer run with one (`/publish?audioGroup=1`, the default now) and with ten (`audioGroup=10`), in turn, on three links: delay, pieces too late, breaks in the sound, frames shown, and the relay's CPU.

## Expected, written before the runs

- **Clean link (50 ms each way).** The sound is the same with both: no piece too late, 0 to 2 breaks in 20 s, with the default buffer of 50 ms. The relay opens 50 streams a second for the sound instead of 5; its CPU goes up, by a few points of one core at most (one viewer: 50 streams a second is little beside 30 frames).
- **1% loss.** One a stream is no worse and likely better: a lost packet holds back the chunks after it on the same stream, which is up to nine more with ten a stream and none with one. With 50 ms, about 100 of 1,000 too late were seen with ten (92 to 137); with one, fewer. With 150 ms, none with either.
- **Narrow link (2,000 kbit/s, queue 100 ms, the busy picture at 2.5 Mbit/s), the viewer asking a timeout for the video alone.** The sound is whole with both: 960 to 985 of 1,000 sounded, as in the record of 2026-10-12 (962 to 983 for both). The picture as before.
- **What would be worse with one**: nothing in these three, by the reasoning above; where one a stream lost more (204 to 375 of 1,000 sounding) the relay gave older Groups up for every track with no grace, which the relay no longer does.

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the demo (`demo/serve.ts`) from the working copy on `main` and the change to one a stream, on ports 45381, 45382, 45384; the own relay with no flag, Cubic; `spikes/lossy-link/proxy.mjs` between relay and viewer, 50 ms each way. The viewer asks for its default timeout for the video and none for the sound. 20 s counted after 5.5 s of playing, 1,000 pieces of sound and 600 frames; the relay's CPU from `ps -o time` at the start and the end of those 20 s. Three runs of each, one and ten in turn.

## Result

Three runs each; in each cell, the runs in order. Of 1,000 pieces of sound and 600 frames in 20 s. The relay's CPU is of one core.

| Link | Buffer | A Group | Sounded | Too late | Breaks | Frames shown | Frame shown after it was made, ms | Relay CPU, % |
|---|---|---|---|---|---|---|---|---|
| clean | 50 | 1 | 1025, 1000, 1000 | 0, 0, 0 | 0, 0, 0 | 621, 606, 606 | 104 to 108 | 4.9, 7.6, 7.4 |
| clean | 50 | 10 | 1000, 1000, 1000 | 0, 0, 0 | 0, 0, 0 | 606, 606, 606 | 107 to 108 | 4.4, 4.5, 4.5 |
| clean | 150 | 1 | 999, 1000, 1000 | 0, 0, 0 | 0, 0, 0 | 606, 606, 606 | 207 to 208 | 6.3, 6.7, 7.0 |
| clean | 150 | 10 | 1000, 999, 1000 | 0, 0, 0 | 0, 0, 0 | 606, 606, 606 | 207 | 4.6, 4.3, 4.4 |
| 1% loss | 50 | 1 | 915, 899, 941 | 69, 91, 49 | **62, 61, 54** | 522, 518, 562 | 107 to 108 | 6.8, 7.6, 7.1 |
| 1% loss | 50 | 10 | 914, 902, 934 | 73, 98, 48 | **41, 46, 30** | 563, 559, 573 | 107 to 108 | 4.6, 4.5, 4.5 |
| 1% loss | 150 | 1 | 1000, 995, 998 | 0, 9, 0 | 0, 7, 2 | 602, 583, 606 | 208 to 210 | 6.6, 7.6, 7.7 |
| 1% loss | 150 | 10 | 987, 996, 1001 | 8, 7, 0 | 3, 3, 0 | 580, 576, 606 | 206 to 209 | 4.5, 4.6, 5.0 |
| narrow | 50 | 1 | 312, 270, 346 | 689, 730, 654 | 104, 105, 143 | 413, 416, 422 | 380 to 386 | 6.4, 6.4, 6.4 |
| narrow | 50 | 10 | 251, 233, 217 | 751, 765, 785 | 96, 107, 93 | 418, 419, 412 | 385 to 394 | 4.1, 4.0, 4.1 |
| narrow | 150 | 1 | 1001, 1000, 1003 | 0, 0, 0 | 0, 0, 0 | 414, 416, 417 | 384 to 386 | 6.7, 6.5, 6.3 |
| narrow | 150 | 10 | 1003, 1002, 1002 | 0, 0, 0 | 0, 0, 0 | 412, 414, 414 | 377 to 389 | 4.0, 4.1, 4.1 |

- **Worse with one a stream: the breaks at 1% loss with 50 ms**, 54 to 62 in 20 s against 30 to 46, with as many pieces too late (49 to 91 against 48 to 98). Why the same number of late pieces makes more breaks was not looked into; one reading, not checked, is that with ten a stream the late pieces come in runs, which is one break for several. With 150 ms the two are alike (0 to 7 breaks).
- **The relay's CPU is about half again as much**: 6.3 to 7.7% of a core against 4.0 to 5.0%, with one viewer, on every link. As expected in size (a few points).
- **On the narrow link with 50 ms one a stream lost fewer** (270 to 346 sounded, against 217 to 251); both lose most of the sound there, and with 150 ms both are whole.
- The picture is the same with both, but at 1% loss with 50 ms, where fewer frames were shown with one a stream (518 to 562 against 559 to 573). Not looked into.
- **The expectation was wrong for the narrow link with 50 ms**: "whole with both" was taken from the record of 2026-10-12, whose viewer had a buffer of 150 ms; with 50 ms three of four pieces are too late, as [the record of the default buffer](2026-10-13-the-default-playout-buffer.md) had. It was also wrong in "fewer too late with one at 1% loss": as many.

**Later the same day**, three more runs of one a stream at 1% loss with 50 ms, on the same code and link, had 18 to 66 too late and 24 to 39 breaks ([record](2026-10-14-a-buffer-of-100-ms-beside-50-150-and-the-automatic-one.md)), as few as ten a stream had here. The more breaks of one a stream above are therefore within what runs of it scatter, and not shown to come from one a stream.
