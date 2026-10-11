# The demo over a link that loses, delays and cuts

- Date: 2026-10-11
- Commit SHA: the commit that adds `spikes/lossy-link`, together with this record; the runs were made on its working copy
- Roadmap item: Phase 2, "Measure latency, drops, stalls and recovery under loss". A first look; nothing was mended

## Objective

See what a viewer of the demo gets when the link between it and the relay loses packets, delays them, lets them pass each other, or is cut for some seconds; and whether a Group on one stream and an Object on a stream differ.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless, publisher and viewer |
| Relay | own (idle timeout 6 seconds, keep-alive every 3) |
| The link | `spikes/lossy-link/proxy.mjs`, a UDP relay in Node 23.10.0 **between the viewer and the relay only**; the publisher connects straight. Loss is a share of the datagrams, each way, at random; delay is added to each datagram, each way; with jitter each datagram is held a random time more, **so datagrams pass each other** |
| The demo | H.264 640x360 at 30 frames a second, about 1.5 Mbit/s, a key frame every second; Opus, a stream for every 20 ms; the viewer's buffer 150 ms |

## Commands

In `spikes/lossy-link/README.md`. One run of 10 seconds for each condition and mapping (a cut: 4 seconds, the cut, 6 seconds).

## Observed results

"Came" is how long after its time a frame was decoded at the viewer; "shown" the same for when it was drawn. A stall is a picture that stood for more than 100 ms. Of about 300 frames and 500 pieces of audio in 10 seconds.

| The link | Mapping | Frames shown | Came (median, most) | Shown (median, most) | Video dropped | Audio too late to sound | Stalls |
|---|---|---|---|---|---|---|---|
| nothing | a Group a stream | 298 | -1, 20 ms | 147, 156 ms | 0 | 0 of 499 | none |
| | an Object a stream | 298 | 2, 8 | 148, 157 | 0 | 0 of 500 | none |
| loss 1% | Group | 298 | 2, 23 | 146, 156 | 0 | 0 of 499 | none |
| | Object | 298 | 1, 20 | 151, 158 | 0 | 0 of 497 | none |
| loss 5% | Group | 297 | 2, 52 | 148, 157 | 0 | 7 of 493 | none |
| | Object | 267 | 2, 32 | 149, 158 | 0 | 181 of 438 | none |
| loss 10% | Group | 282 | 1, 57 | 148, 156 | 0 | 275 of 450 | none |
| | Object | 298 | 3, 43 | 149, 156 | 0 | 256 of 481 | none |
| delay 50 ms | Group | 270 | 53, 64 | 198, 207 | 0 | 0 of 450 | none |
| | Object | 272 | 53, 58 | 199, 207 | 0 | 0 of 451 | none |
| delay 200 ms | Group | 230 | 202, 223 | 353, 363 | 0 | 4 of 372 | 2, 167 ms the longest |
| | Object | 227 | 202, 209 | 349, 358 | 0 | 6 of 372 | 2, 184 ms |
| **delay 50 ms, jitter 30 ms, loss 1%** | Group | **58** | 684, 2291 | 735, 2274 | `late` 81 | 367 of 403 | 29, 1.2 s the longest, **7.4 s of the 10** |
| | Object | **15** | 178, 3966 | 228, 3927 | `overflow` 90, `gap` 7, `late` 4 | 122 of 132 | 2, 3.2 s the longest |
| cut for 3 s | Group | 288 | 1, 3503 | 149, 3507 | `late` 91 | 300 of 499 | 1 of 3.4 s; **a frame 518 ms after the link came back** |
| | Object | 262 | 0, 3496 | 147, 1287 | `late` 64, `overflow` 23, `gap` 13, `no-key-frame` 13, `decoder-behind` 5 | 292 of 491 | 2, 3.4 s the longest; a frame 518 ms after |
| cut for 8 s | both | 120, all before the cut | | | 0 | | **nothing after the link came back** |

- **Loss alone, up to 10%, did not touch the picture** on a link without delay: QUIC sent again within milliseconds, and the buffer of 150 ms hid it. **The sound suffered first**: with 10% loss more than half of the pieces came after their time and were not sounded, with both mappings. The audio is a stream for every 20 ms.
- **Delay is added as it is**: 50 and 200 ms more to "came" and "shown", and nothing else. With 200 ms the picture began later (fewer frames in the 10 seconds) and stood twice at the start.
- **Datagrams that pass each other broke it**: with 30 ms of jitter on 50 ms of delay and 1% loss, a fifth of the frames or fewer were shown and the picture stood for most of the run. **Not looked into**; what is known of QUIC is that it takes a packet that is passed by three later ones for lost and slows down, and this link reorders nearly every packet. A real link jitters without reordering this much, so the row says more about the link here than about a network.
- **A cut of 3 seconds is survived**: the picture stood for 3.4 seconds and a frame was shown half a second after the link came back. What had been sent meanwhile came all at once and was dropped as too old (`late`) with a Group on a stream; with an Object on a stream the receiver's limits were met as well (`overflow`, `gap`) and the decoder fell behind.
- **A cut of 8 seconds ends the session, and the page does not come back**: longer than the relay's idle timeout of 6 seconds. The watching page neither connects again nor says that it ended: its line of numbers goes on showing the last ones.
- **The two mappings did not differ in a way one run each can show**, but for the kinds of drop after a cut and under reordering.

## Limitations and blockers

- **One run for each row**, 10 seconds, on loopback: the numbers move from run to run, and a difference of a few frames means nothing.
- The link is between the viewer and the relay only; the publisher's link was not touched.
- The model is crude: independent loss, no bursts, no limit on the rate, jitter that reorders.
- Nothing was mended, and nothing here was seen or heard.

## Open questions raised

- Whether the audio is to go on fewer streams, or as datagrams: it is what breaks first under loss.
- What a viewer does when its session ends: connect again, and say so.
- What reordering does to the relay's and the browser's QUIC, measured with a link that reorders a little.
- How long the buffer is under delay and loss, and whether it adapts.
