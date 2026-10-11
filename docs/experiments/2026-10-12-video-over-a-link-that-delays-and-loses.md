# Video over a link that delays and loses: how late the frames come, and the congestion control

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, the relay (latency)

## Objective

What a viewer waits for a frame when the path from the relay to the viewer has a delay and loses packets at once, which the measurements of the day before did not have: their loss was on a path without delay. And what of it is the transport's doing.

## Environment

One machine, macOS, loopback. `xmoq-relay` and `xmoq-moqt-client` of the working copy this record is part of, built with `--release`. Between the relay and the viewers, and nowhere else, `spikes/lossy-link/proxy.mjs` (a UDP relay in Node that holds every datagram for a delay, each way, and drops a share of the ones from the relay to the viewer). The videos are the test patterns of [the record of the fan-out](2026-10-11-one-video-track-to-many-viewers.md), 25 frames a second, a key frame a second, and one more of 160x90 at 78 kbit/s, the least the encoder gave when asked for 25.

## Commands

```sh
[CONGESTION=cubic|newreno|bbr] spikes/latency-loss/run.sh <delay ms, one way> <loss, 0..1> <file.h264> <group|object> 500 4
```

- The publisher is the Rust client with `--video-in`, connected to the relay itself; `--video-streams group` puts a Group on one stream, as before, and `object`, new, every frame on a stream of its own.
- The viewers are 4 sessions (`--viewers 4`) that take 500 Objects each, 20 seconds. Each prints every Object that came with the microseconds from its Timestamp of LOC to the clock (`VIDEO_ARRIVALS`, new).
- `spikes/latency-loss/analyse.py` makes the numbers: **a frame is late by the time from its Timestamp to the moment it and every frame of its Group before it have come**, which is when it can be shown. The delay of the path is in it. An episode is a run of frames more than one frame (40 ms) later than the viewer's median. "In N ms" is the share of the frames no more than N ms later than the fastest frame of the run, which is what a viewer with a buffer of N ms would show in time.
- `--congestion cubic|newreno|bbr` of the relay is new: the congestion control of its connections. Cubic is the default of quinn-proto 0.11.19, and stays the relay's.

## Observed results

One run each.

### Three congestion controls, one way 50 ms

| Video | Loss | Congestion control | Late: median / 95% / 99% / highest, ms | The median of each quarter of the 20 seconds, ms | Congestion window when a stream is opened: median / least, bytes | In 150 ms | Frames that came, of 2,000 |
|---|---|---|---|---|---|---|---|
| 160x90, 78 kbit/s | 1% | cubic | 52 / 54 / 193 / 314 | 52 / 51 / 52 / 52 | 5880 / 2904 | 99.8% | 1939 |
| 160x90, 78 kbit/s | 1% | newreno | 52 / 77 / 193 / 271 | 52 / 52 / 52 / 52 | 6000 / 2904 | 99.9% | 1942 |
| 160x90, 78 kbit/s | 1% | bbr | 52 / 53 / 158 / 201 | 51 / 51 / 52 / 52 | 353067 / 244584 | 99.9% | 1940 |
| 160x90, 78 kbit/s | 5% | cubic | 52 / 195 / 286 / 469 | 52 / 52 / 52 / 52 | 3150 / 2904 | 96.9% | 1951 |
| 160x90, 78 kbit/s | 5% | newreno | 52 / 194 / 244 / 447 | 52 / 52 / 53 / 53 | 3000 / 2904 | 97.5% | 1946 |
| 160x90, 78 kbit/s | 5% | bbr | 52 / 184 / 203 / 354 | 52 / 52 / 52 / 52 | 353550 / 244563 | 98.9% | 1941 |
| 320x180, 513 kbit/s | 1% | cubic | 53 / 143 / 199 / 331 | 53 / 53 / 53 / 52 | 12000 / 6948 | 99.1% | 1940 |
| 320x180, 513 kbit/s | 1% | newreno | 53 / 213 / 373 / 573 | 53 / 53 / 53 / 53 | 12000 / 4834 | 94.5% | 1940 |
| 320x180, 513 kbit/s | 1% | bbr | 53 / 92 / 172 / 203 | 53 / 53 / 52 / 52 | 876858 / 241869 | 99.9% | 1941 |
| 320x180, 513 kbit/s | 5% | cubic | 218 / 919 / 1420 / 1949 | 208 / 339 / 164 / 288 | 8617 / 4400 | 45.8% | 1943 |
| 320x180, 513 kbit/s | 5% | newreno | 429 / 1544 / 2002 / 3028 | 381 / 564 / 245 / 663 | 8071 / 3000 | 23.1% | 1952 |
| 320x180, 513 kbit/s | 5% | bbr | 53 / 195 / 276 / 323 | 53 / 52 / 52 / 53 | 875766 / 244589 | 97.7% | 1946 |
| 1280x720, 2.5 Mbit/s | 1% | cubic | 5269 / 18281 / 22542 / 27229 | 315 / 5304 / 9644 / 7351 | 18538 / 7219 | 11.0% | 1771 |
| 1280x720, 2.5 Mbit/s | 1% | newreno | 6964 / 20778 / 24888 / 27997 | 1874 / 7590 / 11529 / 8259 | 17487 / 3689 | 6.7% | 1651 |
| 1280x720, 2.5 Mbit/s | 1% | bbr | 53 / 159 / 176 / 278 | 54 / 53 / 53 / 53 | 3345986 / 244584 | 99.8% | 1938 |
| 1280x720, 2.5 Mbit/s | 5% | cubic | 11771 / 27976 / 32263 / 35296 | 4904 / 11184 / 13817 / 14555 | 9160 / 4538 | 1.2% | 642 |
| 1280x720, 2.5 Mbit/s | 5% | newreno | 12417 / 28213 / 31485 / 33506 | 8722 / 10989 / 13517 / 14449 | 8391 / 3356 | 0.6% | 521 |
| 1280x720, 2.5 Mbit/s | 5% | bbr | 129 / 233 / 311 / 514 | 131 / 118 / 124 / 136 | 3199121 / 32296 | 94.3% | 1945 |

- **Cubic and New Reno fall behind for good where the video is more than the window lets through.** A loss makes them halve the window, or nearly; with a loss every hundred packets or so and a round trip of 100 ms the window stays at a few packets (3,000 to 18,000 bytes), and what that lets through in a round trip is less than 2.5 Mbit/s at 1% of loss, and less than 513 kbit/s at 5%. The lateness then grows through the run and does not come back: 0.3, 5.3, 9.6 seconds in the first three quarters for 2.5 Mbit/s at 1%. Some viewers did not get their 500 frames in the time they had.
- **BBR does not.** Its window stays at hundreds of thousands of bytes whatever is lost, and the median lateness stays the delay of the path; what is left is the wait for a packet that is sent again (the 99th percentile, 160 to 310 ms). At 5% of loss and 2.5 Mbit/s the median is 129 ms, and 94% of the frames are within a buffer of 150 ms.
- At 78 kbit/s none of the three falls behind: a window of two packets is enough for it.
- The viewers are the Rust client, so this is no matter of a browser.

### Delay and loss, with Cubic, a Group on one stream

| Video | Delay, one way | Loss | Late: median / 95% / 99% / highest, ms | Episodes | Frames of an episode: median / longest | In 50 / 150 / 300 ms | Frames that came, of 2,000 |
|---|---|---|---|---|---|---|---|
| 1280x720, 2.5 Mbit/s | 0 ms | 0% | 1 / 3 / 5 / 8 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1905 |
| 1280x720, 2.5 Mbit/s | 0 ms | 1% | 1 / 4 / 29 / 37 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1928 |
| 1280x720, 2.5 Mbit/s | 0 ms | 2% | 1 / 4 / 30 / 42 | 1 | 1 / 1 | 100.0% / 100.0% / 100.0% | 1905 |
| 1280x720, 2.5 Mbit/s | 0 ms | 5% | 1 / 4 / 32 / 43 | 4 | 1 / 1 | 100.0% / 100.0% / 100.0% | 1904 |
| 1280x720, 2.5 Mbit/s | 25 ms | 0% | 27 / 29 / 37 / 48 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1922 |
| 1280x720, 2.5 Mbit/s | 25 ms | 1% | 228 / 1734 / 3035 / 3735 | 53 | 13 / 74 | 31.9% / 47.3% / 56.0% | 1922 |
| 1280x720, 2.5 Mbit/s | 25 ms | 2% | 3432 / 10702 / 13687 / 17438 | 87 | 12 / 20 | 0.9% / 2.4% / 4.4% | 1922 |
| 1280x720, 2.5 Mbit/s | 25 ms | 5% | 8595 / 23361 / 28425 / 31641 | 102 | 6 / 15 | 0.1% / 0.4% / 1.2% | 1368 |
| 1280x720, 2.5 Mbit/s | 50 ms | 0% | 53 / 56 / 74 / 83 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1942 |
| 1280x720, 2.5 Mbit/s | 50 ms | 1% | 5006 / 17526 / 22396 / 25757 | 92 | 9 / 38 | 3.5% / 6.0% / 8.7% | 1792 |
| 1280x720, 2.5 Mbit/s | 50 ms | 2% | 9125 / 24531 / 29018 / 33494 | 96 | 6 / 11 | 1.0% / 2.1% / 2.6% | 1201 |
| 1280x720, 2.5 Mbit/s | 50 ms | 5% | 11254 / 28941 / 33868 / 35817 | 75 | 5 / 10 | 0.3% / 1.5% / 2.3% | 747 |
| 1280x720, 2.5 Mbit/s | 100 ms | 0% | 103 / 120 / 429 / 493 | 4 | 17 / 17 | 96.6% / 97.4% / 98.6% | 1975 |
| 1280x720, 2.5 Mbit/s | 100 ms | 1% | 11107 / 27653 / 31756 / 36118 | 79 | 5 / 10 | 0.2% / 1.5% / 3.0% | 810 |
| 1280x720, 2.5 Mbit/s | 100 ms | 2% | 8671 / 26203 / 32050 / 36167 | 47 | 6 / 33 | 2.3% / 3.2% / 7.7% | 666 |
| 1280x720, 2.5 Mbit/s | 100 ms | 5% | 16157 / 28466 / 30534 / 32589 | 45 | 2 / 4 | 0.6% / 0.6% / 0.6% | 179 |
| 320x180, 513 kbit/s | 0 ms | 0% | 1 / 2 / 3 / 7 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1904 |
| 320x180, 513 kbit/s | 0 ms | 1% | 1 / 3 / 5 / 36 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1904 |
| 320x180, 513 kbit/s | 0 ms | 2% | 1 / 4 / 32 / 43 | 1 | 1 / 1 | 100.0% / 100.0% / 100.0% | 1905 |
| 320x180, 513 kbit/s | 0 ms | 5% | 1 / 29 / 33 / 75 | 3 | 1 / 1 | 99.8% / 100.0% / 100.0% | 1928 |
| 320x180, 513 kbit/s | 25 ms | 0% | 28 / 29 / 30 / 31 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1922 |
| 320x180, 513 kbit/s | 25 ms | 1% | 28 / 34 / 92 / 131 | 33 | 1 / 3 | 97.5% / 100.0% / 100.0% | 1922 |
| 320x180, 513 kbit/s | 25 ms | 2% | 28 / 82 / 122 / 202 | 81 | 1 / 9 | 93.1% / 99.9% / 100.0% | 1923 |
| 320x180, 513 kbit/s | 25 ms | 5% | 28 / 137 / 208 / 357 | 176 | 2 / 21 | 76.5% / 97.7% / 99.9% | 1926 |
| 320x180, 513 kbit/s | 50 ms | 0% | 53 / 54 / 55 / 70 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1940 |
| 320x180, 513 kbit/s | 50 ms | 1% | 53 / 132 / 196 / 292 | 65 | 2 / 16 | 91.3% / 99.2% / 100.0% | 1941 |
| 320x180, 513 kbit/s | 50 ms | 2% | 53 / 315 / 1151 / 1565 | 93 | 2 / 98 | 71.0% / 88.7% / 95.7% | 1940 |
| 320x180, 513 kbit/s | 50 ms | 5% | 383 / 1882 / 2359 / 2966 | 55 | 8 / 171 | 13.6% / 29.8% / 47.8% | 1946 |
| 320x180, 513 kbit/s | 100 ms | 0% | 102 / 104 / 104 / 133 | 0 | 0 / 0 | 100.0% / 100.0% / 100.0% | 1975 |
| 320x180, 513 kbit/s | 100 ms | 1% | 104 / 1200 / 1968 / 2717 | 58 | 5 / 177 | 60.3% / 69.6% / 83.2% | 1960 |
| 320x180, 513 kbit/s | 100 ms | 2% | 508 / 3561 / 5243 / 6029 | 81 | 6 / 99 | 20.8% / 27.5% / 41.9% | 1975 |
| 320x180, 513 kbit/s | 100 ms | 5% | 5588 / 19278 / 23902 / 26978 | 96 | 9 / 20 | 1.0% / 2.6% / 6.2% | 1749 |

- **Without delay, loss costs little**: the 99th percentile is one frame more, at 5% as well. A lost packet is sent again within a round trip, which is under a millisecond here, and a small window is enough.
- **With delay and no loss, the lateness is the delay.**
- **With both, it depends on the bit rate against the window**, as above: 2.5 Mbit/s falls behind at 25 ms and 1%; 513 kbit/s holds at 25 ms up to 5% (a buffer of 150 ms has 98% of the frames), and falls behind at 50 ms with 5% and at 100 ms with 2%.
- An episode, where the video holds, is one to three frames at 25 ms and up to a few at 50 ms: the frames behind a lost packet on the stream of their Group wait for it.

### A frame on a stream of its own, against a Group on one stream

Added afterwards (`--video-streams object`). With Cubic, the same delays and losses; "late" is, as above, until the frame can be shown, which needs the frames of its Group before it.

| Video | Delay, one way | Loss | A Group on one stream: median / 99% / highest, ms | In 150 ms | A frame on its own stream: median / 99% / highest, ms | In 150 ms | Frames that came, of 2,000 | Frames that could not be shown for one before them that did not come |
|---|---|---|---|---|---|---|---|---|
| 1280x720, 2.5 Mbit/s | 0 ms | 0% | 1 / 5 / 8 | 100.0% | 1 / 5 / 12 | 100.0% | 1905 | 0 |
| 1280x720, 2.5 Mbit/s | 0 ms | 1% | 1 / 29 / 37 | 100.0% | 1 / 7 / 32 | 100.0% | 1928 | 0 |
| 1280x720, 2.5 Mbit/s | 0 ms | 2% | 1 / 30 / 42 | 100.0% | 1 / 30 / 42 | 100.0% | 1905 | 0 |
| 1280x720, 2.5 Mbit/s | 0 ms | 5% | 1 / 32 / 43 | 100.0% | 1 / 33 / 44 | 100.0% | 1952 | 0 |
| 1280x720, 2.5 Mbit/s | 25 ms | 0% | 27 / 37 / 48 | 100.0% | 28 / 38 / 52 | 100.0% | 1921 | 0 |
| 1280x720, 2.5 Mbit/s | 25 ms | 1% | 228 / 3035 / 3735 | 47.3% | 654 / 6113 / 6633 | 31.2% | 1781 | 0 |
| 1280x720, 2.5 Mbit/s | 25 ms | 2% | 3432 / 13687 / 17438 | 2.4% | 9648 / 19909 / 21500 | 1.0% | 1107 | 432 |
| 1280x720, 2.5 Mbit/s | 25 ms | 5% | 8595 / 28425 / 31641 | 0.4% | 15865 / 18443 / 18563 | 1.2% | 336 | 341 |
| 1280x720, 2.5 Mbit/s | 50 ms | 0% | 53 / 74 / 83 | 100.0% | 53 / 79 / 105 | 100.0% | 1939 | 0 |
| 1280x720, 2.5 Mbit/s | 50 ms | 1% | 5006 / 22396 / 25757 | 6.0% | 12139 / 22519 / 22759 | 20.1% | 661 | 545 |
| 1280x720, 2.5 Mbit/s | 50 ms | 2% | 9125 / 29018 / 33494 | 2.1% | 16183 / 18783 / 18863 | 16.0% | 257 | 40 |
| 1280x720, 2.5 Mbit/s | 50 ms | 5% | 11254 / 33868 / 35817 | 1.5% | 3567 / 11873 / 11873 | 24.0% | 25 | 0 |
| 1280x720, 2.5 Mbit/s | 100 ms | 0% | 103 / 429 / 493 | 97.4% | 103 / 624 / 823 | 97.1% | 1975 | 0 |
| 1280x720, 2.5 Mbit/s | 100 ms | 1% | 11107 / 31756 / 36118 | 1.5% | 18599 / 19766 / 19791 | 2.4% | 164 | 32 |
| 1280x720, 2.5 Mbit/s | 100 ms | 2% | 8671 / 32050 / 36167 | 3.2% | 1507 / 5562 / 5562 | 9.3% | 75 | 0 |
| 1280x720, 2.5 Mbit/s | 100 ms | 5% | 16157 / 30534 / 32589 | 0.6% | no frame | - | 0 | - |
| 320x180, 513 kbit/s | 0 ms | 0% | 1 / 3 / 7 | 100.0% | 1 / 4 / 6 | 100.0% | 1904 | 0 |
| 320x180, 513 kbit/s | 0 ms | 1% | 1 / 5 / 36 | 100.0% | 2 / 30 / 37 | 100.0% | 1905 | 0 |
| 320x180, 513 kbit/s | 0 ms | 2% | 1 / 32 / 43 | 100.0% | 2 / 34 / 39 | 100.0% | 1904 | 0 |
| 320x180, 513 kbit/s | 0 ms | 5% | 1 / 33 / 75 | 100.0% | 2 / 38 / 85 | 100.0% | 1928 | 0 |
| 320x180, 513 kbit/s | 25 ms | 0% | 28 / 30 / 31 | 100.0% | 28 / 30 / 33 | 100.0% | 1922 | 0 |
| 320x180, 513 kbit/s | 25 ms | 1% | 28 / 92 / 131 | 100.0% | 28 / 95 / 134 | 100.0% | 1922 | 0 |
| 320x180, 513 kbit/s | 25 ms | 2% | 28 / 122 / 202 | 99.9% | 28 / 160 / 454 | 99.1% | 1922 | 0 |
| 320x180, 513 kbit/s | 25 ms | 5% | 28 / 208 / 357 | 97.7% | 28 / 465 / 696 | 95.5% | 1925 | 2 |
| 320x180, 513 kbit/s | 50 ms | 0% | 53 / 55 / 70 | 100.0% | 53 / 55 / 71 | 100.0% | 1940 | 0 |
| 320x180, 513 kbit/s | 50 ms | 1% | 53 / 196 / 292 | 99.2% | 53 / 312 / 452 | 97.6% | 1939 | 0 |
| 320x180, 513 kbit/s | 50 ms | 2% | 53 / 1151 / 1565 | 88.7% | 53 / 1205 / 1765 | 88.2% | 1934 | 4 |
| 320x180, 513 kbit/s | 50 ms | 5% | 383 / 2359 / 2966 | 29.8% | 795 / 8522 / 8922 | 27.4% | 1839 | 18 |
| 320x180, 513 kbit/s | 100 ms | 0% | 102 / 104 / 133 | 100.0% | 103 / 105 / 139 | 100.0% | 1975 | 0 |
| 320x180, 513 kbit/s | 100 ms | 1% | 104 / 1968 / 2717 | 69.6% | 548 / 12012 / 12692 | 37.2% | 1748 | 20 |
| 320x180, 513 kbit/s | 100 ms | 2% | 508 / 5243 / 6029 | 27.5% | 5646 / 10631 / 11151 | 12.0% | 1581 | 18 |
| 320x180, 513 kbit/s | 100 ms | 5% | 5588 / 23902 / 26978 | 2.6% | 13339 / 23779 / 24099 | 1.2% | 864 | 740 |

With BBR, one way 50 ms; "came" is how late the frame itself came, whatever came before it:

| Video | Loss | Streams | Late (can be shown): median / 95% / 99% / highest, ms | Came: median / 99%, ms | In 150 ms |
|---|---|---|---|---|---|
| 320x180, 513 kbit/s | 1% | group | 52 / 92 / 194 / 325 | 52 / 194 | 99.6% |
| 320x180, 513 kbit/s | 1% | object | 53 / 119 / 194 / 202 | 53 / 194 | 99.9% |
| 320x180, 513 kbit/s | 5% | group | 53 / 196 / 246 / 327 | 53 / 245 | 98.5% |
| 320x180, 513 kbit/s | 5% | object | 53 / 193 / 201 / 328 | 53 / 199 | 98.9% |
| 1280x720, 2.5 Mbit/s | 1% | group | 53 / 158 / 195 / 290 | 53 / 195 | 99.7% |
| 1280x720, 2.5 Mbit/s | 1% | object | 53 / 157 / 196 / 279 | 52 / 194 | 99.7% |
| 1280x720, 2.5 Mbit/s | 5% | group | 130 / 235 / 278 / 411 | 130 / 278 | 93.5% |
| 1280x720, 2.5 Mbit/s | 5% | object | 119 / 198 / 276 / 411 | 53 / 276 | 95.2% |

- **A stream for every frame does not make the video come sooner.** Where the congestion window is what holds it back (Cubic with delay and loss) it is no better and mostly worse: fewer frames came in the 20 seconds, and frames were left without the ones before them. Where the window is no limit (BBR) the two are the same within what one run shows.
- **The frame itself does come without waiting**: at 5% of loss and 2.5 Mbit/s with BBR, half of the frames came within 53 ms on streams of their own, and within 130 ms on the stream of their Group, where a frame waits behind a lost packet of the one before it. It can be shown no sooner for that (119 against 130 ms), since it needs the frames before it. What a stream for every frame would buy is for frames that do not depend on each other, or for a viewer that drops what is late.
- Seven of the runs with Cubic (the rows of 1280x720 from 0 ms and 2% to 50 ms and 2%) ran while `mise run check` and the matrix of `interop/` ran on the same machine.

### The shape of a track of audio

Added afterwards, for a question from the browser's side: a page fell behind with sound alone, 69 kbit/s, on such a link. The same shape from the Rust publisher (`--video-frame-ms 20` and a file of Groups of 10 Objects of 160 bytes: 50 Objects a second, a Group on one stream every 200 ms, 64 kbit/s), 4 Rust viewers, 1,000 Objects each, one way 50 ms:

| Loss | Congestion control | Late: median / 95% / 99% / highest, ms | The median of each quarter, ms | Congestion window: median / least, bytes | In 150 ms |
|---|---|---|---|---|---|
| 0% | Cubic | 52 / 53 / 54 / 58 | 52 / 52 / 52 / 52 | 12000 / 12000 | 100.0% |
| 1% | Cubic | 52 / 75 / 174 / 304 | 52 / 52 / 52 / 52 | 2904 / 2904 | 99.8% |
| 5% | Cubic | 52 / 172 / 225 / 317 | 52 / 52 / 52 / 52 | 2904 / 2904 | 98.8% |
| 0% | BBR | 52 / 53 / 54 / 58 | 52 / 52 / 52 / 52 | 350726 / 244584 | 100.0% |
| 1% | BBR | 52 / 75 / 172 / 184 | 52 / 52 / 52 / 52 | 349273 / 244584 | 100.0% |
| 5% | BBR | 52 / 173 / 213 / 425 | 52 / 52 / 52 / 52 | 349882 / 244584 | 98.9% |

- **A Rust viewer does not fall behind with this shape**, with either control, at 5% as well: the median stays the delay of the path through the 20 seconds, and an Object behind a lost packet is 120 to 170 ms late, once. A window of two packets is enough for 64 kbit/s.
- So a page that falls behind with sound alone does so for another reason than the relay's congestion control.
- The Objects are no Opus: bytes of the size of a piece of it, with the Timestamp of LOC.

## Limitations and blockers

- **The loss is random, and the path is not full.** A path that loses because it is full is another matter: there BBR, which does not back off for loss, may fill what there is and push others out. Not measured.
- **quinn-proto 0.11.19 has of its BBR: "Experimental! Use at your own risk."**
- One machine and loopback; the link is a program that holds datagrams with timers of Node, good to about a millisecond.
- One run each. The loss is drawn anew in every run, so two runs differ.
- 78 kbit/s is video, not sound: a frame every 40 ms on the stream of its Group.
- `CubicConfig::initial_window` and the like were not tried: a window that is cut at every loss does not stay where it began.
- Whether the relay cut a viewer off, or held a stream back for the viewer's limit of streams, in the runs with a frame on a stream of its own that lost frames, was not looked at.

## Decision or next action

Which congestion control the relay uses is with the project's owner. The flag is there to try them.
