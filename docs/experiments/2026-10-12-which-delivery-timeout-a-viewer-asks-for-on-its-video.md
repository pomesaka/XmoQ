# Which delivery timeout a viewer asks for on its video

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link that is narrow or loses packets; the open decision on what a slow subscriber is not sent. Material; nothing is decided here.

## Objective

A viewer that asks for a SUBGROUP_DELIVERY_TIMEOUT of 100 ms on its video, at a relay with no flag, had the best picture on a narrow link with 50 ms each way ([record](2026-10-12-the-relay-follows-the-delivery-timeout-a-viewer-asks-for.md)). A timeout shorter than a round trip also gives up what a packet sent again would have brought. Which number, on links that are nearer and further, narrow or lossy? Can it be said in round trips, and can the page work it out from what it measures?

Expected, written before the runs: on a narrow link without loss the shortest timeout is the best whatever the delay, since nothing is sent again there; with loss the best timeout grows with the round trip (about one to two of them), and a timeout below it shows fewer frames than a longer one.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Viewer | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; `/watch?videoTimeout=N`, nothing asked for the sound; buffer 150 ms |
| Relay | own, Cubic, no flag: the subgroup timer of a subscription that asked, counted from the publisher's FIN |
| Link | `spikes/lossy-link/proxy.mjs` between viewer and relay: 10, 50 and 100 ms each way; `rate` 2,000 kbit/s with `queue` 100 ms and no loss, or 1% and 5% loss each way with no limit |
| Sent | on the narrow link the busy picture at 2.5 Mbit/s, with loss the clock (about 450 kbit/s); 640x360, 30 frames and a Group a second; Opus, ten pieces a Group |

Two runs of 20 s (600 frames, 1,000 pieces of sound) for each timeout; 96 runs. With 1% loss, 10 ms each way was not run.

## Result

Frames shown of 600, the two runs:

| Link | Each way | none | 50 ms | 100 ms | 150 ms | 250 ms | 500 ms |
|---|---|---|---|---|---|---|---|
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 10 ms | 123, 123 | 423, 426 | 426, 423 | 424, 431 | 410, 406 | 357, 357 |
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 50 ms | 123, 115 | 416, 420 | 418, 421 | 413, 417 | 395, 399 | 356, 353 |
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 100 ms | 111, 113 | 379, 378 | 388, 385 | 387, 387 | 378, 375 | 339, 330 |
| 1% loss, room to spare (clock) | 50 ms | 601, 546 | 602, 598 | 593, 596 | 578, 592 | 589, 600 | 604, 605 |
| 1% loss, room to spare (clock) | 100 ms | 508, 302 | 430, 399 | 537, 473 | 489, 434 | 360, 451 | 538, 430 |
| 5% loss, room to spare (clock) | 10 ms | 603, 606 | 604, 605 | 606, 606 | 606, 606 | 605, 605 | 605, 606 |
| 5% loss, room to spare (clock) | 50 ms | 319, 337 | 380, 401 | 386, 320 | 385, 307 | 364, 325 | 353, 294 |
| 5% loss, room to spare (clock) | 100 ms | 62, 68 | 111, 153 | 175, 159 | 111, 150 | 111, 209 | 126, 164 |

How late the picture came at the end of the run, ms (the median of the last 200 frames):

| Link | Each way | none | 50 ms | 100 ms | 150 ms | 250 ms | 500 ms |
|---|---|---|---|---|---|---|---|
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 10 ms | 635, 623 | 292, 302 | 312, 322 | 349, 354 | 388, 379 | 495, 498 |
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 50 ms | 709, 720 | 323, 324 | 357, 349 | 386, 360 | 420, 416 | 530, 531 |
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 100 ms | 883, 853 | 352, 354 | 384, 393 | 415, 401 | 434, 446 | 561, 566 |
| 1% loss, room to spare (clock) | 50 ms | 54, 106 | 53, 53 | 61, 52 | 53, 52 | 53, 53 | 52, 53 |
| 1% loss, room to spare (clock) | 100 ms | 104, 258 | 156, 166 | 103, 171 | 105, 204 | 246, 175 | 140, 217 |
| 5% loss, room to spare (clock) | 10 ms | 13, 13 | 13, 13 | 14, 14 | 13, 13 | 14, 13 | 13, 14 |
| 5% loss, room to spare (clock) | 50 ms | 173, 186 | 146, 123 | 157, 158 | 106, 178 | 141, 200 | 148, 207 |
| 5% loss, room to spare (clock) | 100 ms | 1028, 810 | 501, 218 | 263, 267 | 409, 344 | 601, 221 | 435, 367 |

The sound, for which nothing was asked: pieces sounded of 1,000, and in brackets how much later than the fastest piece 95 of 100 pieces came, ms (the two runs):

| Link | Each way | none | 50 ms | 100 ms | 150 ms | 250 ms | 500 ms |
|---|---|---|---|---|---|---|---|
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 10 ms | 994, 984 (88, 96) | 992, 986 (59, 60) | 983, 995 (62, 55) | 991, 991 (54, 58) | 985, 981 (56, 61) | 997, 991 (61, 60) |
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 50 ms | 984, 994 (120, 121) | 989, 996 (90, 90) | 998, 989 (90, 88) | 993, 994 (94, 93) | 999, 998 (96, 87) | 992, 998 (95, 96) |
| 2 Mbit/s, no loss (busy picture, 2.5 Mbit/s) | 100 ms | 946, 931 (128, 133) | 953, 970 (104, 106) | 968, 974 (97, 100) | 977, 981 (96, 103) | 955, 975 (105, 105) | 967, 986 (99, 106) |
| 1% loss, room to spare (clock) | 50 ms | 990, 995 (84, 105) | 998, 995 (64, 70) | 983, 996 (110, 81) | 986, 986 (110, 98) | 994, 988 (86, 97) | 998, 994 (86, 70) |
| 1% loss, room to spare (clock) | 100 ms | 913, 893 (143, 180) | 854, 888 (195, 158) | 953, 937 (130, 134) | 961, 920 (116, 150) | 935, 914 (151, 148) | 963, 974 (119, 131) |
| 5% loss, room to spare (clock) | 10 ms | 998, 996 (35, 31) | 999, 996 (38, 38) | 1000, 998 (43, 43) | 999, 998 (43, 36) | 998, 999 (37, 37) | 999, 998 (40, 40) |
| 5% loss, room to spare (clock) | 50 ms | 924, 938 (170, 156) | 927, 935 (160, 165) | 903, 897 (180, 192) | 909, 893 (165, 197) | 922, 912 (159, 172) | 939, 925 (152, 163) |
| 5% loss, room to spare (clock) | 100 ms | 278, 165 (295, 305) | 591, 556 (231, 260) | 713, 653 (217, 229) | 560, 697 (236, 213) | 566, 662 (222, 223) | 587, 673 (231, 222) |

- **On the narrow link the shorter timeout is the better one, at every delay**, as expected: 50, 100 and 150 ms show the same frames (within 10 of 600) and 250 and 500 ms fewer; the picture is the nearer the shorter the timeout (at 50 ms each way: 323, 353, 373, 418 and 530 ms behind for 50 to 500 ms). The delay of the link hardly matters: 423 to 431 frames at 10 ms each way, 413 to 421 at 50, 378 to 388 at 100. Nothing is sent again on a link that only delays, so nothing is lost by giving up early.
- **With loss, the runs do not show a best timeout, and not that a short one costs frames.** At 10 ms each way and 5% loss every frame comes with any timeout, 50 ms among them (a packet sent again is there within 50 ms). At 50 ms each way, two runs of one timeout differ by more (320 and 386 with 100 ms) than the timeouts do; at 1% all are whole or nearly. At 100 ms each way everything is poor, with or without a timeout: the relay's Cubic lets less through than the video needs (62 and 68 frames with no timeout at 5%), which is the congestion control's matter and not the timeout's. **The expectation that the best timeout grows with the round trip is neither shown nor refuted.**
- **A timeout on the video helps the sound where the link is the bottleneck**: at 100 ms each way and 5% loss, 165 and 278 pieces sounded with no timeout and 556 to 713 with one, though nothing was asked for the sound. The video that is given up no longer stands in its way.
- **On the narrow link every timeout beat none by far** (330 to 431 frames against 111 to 123), and 500 ms was the worst of the five. With loss the cells with and without a timeout overlap, but for 5% at 100 ms each way, where every timeout showed more than none.

### What the page can measure

The page does not know the round trip. It does see how much later than the fastest one each piece of sound comes, and the sound is small and not given up, so its lateness is the path's: a piece behind a lost packet comes when the packet was sent again.

| Link | Each way | 95 of 100 pieces of sound came within, ms after the fastest (all runs of the row) | 99 of 100 |
|---|---|---|---|
| 2 Mbit/s, no loss | 10 ms | 54 to 96 | 64 to 173 |
| 2 Mbit/s, no loss | 50 ms | 87 to 121 | 99 to 245 |
| 2 Mbit/s, no loss | 100 ms | 96 to 133 | 141 to 299 |
| 1% loss | 50 ms | 64 to 110 | 112 to 182 |
| 1% loss | 100 ms | 116 to 195 | 176 to 239 |
| 5% loss | 10 ms | 31 to 43 | 51 to 77 |
| 5% loss | 50 ms | 152 to 197 | 223 to 371 |
| 5% loss | 100 ms | 213 to 305 | 237 to 403 |

- With loss it follows the round trip: about 35 ms at a round trip of 20 ms, 150 to 200 at 100 ms, 210 to 300 at 200 ms. On the narrow link it is the queue's wait, 55 to 130 ms.
- **A rule that could be tried, and was not**: ask for what 95 of 100 pieces of sound came within over the last seconds, not below 50 ms and not above 500 ms (the automatic buffer of `createPlayout` keeps the same kind of number for both tracks, `clock.lateness()`; with the video in it, it also counts what the timeout is to remove, 340 to 750 ms in these runs, so it is the sound's alone that would be used). On these links that asks for 50 to 130 ms where the link is narrow and 150 to 300 ms where it loses packets at a distance: the first is where the short timeouts were best, and the second is where the runs cannot tell.

### Changing the timeout while watching

A rule like the one above changes its number as the link does.

- **REQUEST_UPDATE** is the draft's way (§10.9; the parameter "MAY appear in a ... REQUEST_UPDATE message"). It would need: the relay to accept a REQUEST_UPDATE for a subscription and answer REQUEST_OK (today it is refused for a subscription, and answered for a FETCH); a decision on whether the new time holds for streams that are open (the draft does not say); and a way to send one in `@xmoq/moqt`, which encodes and decodes the message and has no call that sends it for a subscription.
- **Without it**: subscribe again with the new number and leave the subscription before at the first frame of the new one, as the change of versions does (`?abr=1`). That works today, and costs what a change of versions costs: with other viewers on the track, up to a Group's wait.
- A fixed number needs neither. On what was run, 100 to 150 ms is as good as any other from 50 to 150 on the narrow links and not worse than longer ones under loss.

## Not done

- More than two runs a cell: with loss the cells differ less than two runs of one cell.
- BBR, which holds its window under loss: with Cubic at 100 ms each way and 5% loss the link carries almost nothing, and no timeout can be judged there.
- Loss together with a narrow link; loss in bursts; a real network.
- The rule from the sound's lateness; a timeout on the sound.
- Anybody watching: a picture that stands still at a Group's end against one that is whole and late.
