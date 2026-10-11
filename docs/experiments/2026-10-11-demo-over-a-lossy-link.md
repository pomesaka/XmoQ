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
- **A cut of 8 seconds ends the session, and the page did not come back** (it does now: "Afterwards"): longer than the relay's idle timeout of 6 seconds. The watching page neither connects again nor says that it ended: its line of numbers goes on showing the last ones.
- **The two mappings did not differ in a way one run each can show**, but for the kinds of drop after a cut and under reordering.

## Afterwards: the pages connect again, and the sound under loss

### A session that ends

Both pages of the demo now say when their session has ended and connect again every 2 seconds; the watching page asks the room anew where the broadcast is, and also starts over when the video track ends while its session is still there. `spikes/lossy-link/reconnect.ts`, a cut of 8 seconds, one run each:

| Whose link was cut | While it was cut | The picture back | The sound back |
|---|---|---|---|
| The viewer's | "the session ended (error)" after 6 seconds, then "connecting" | 1.8 s after the link came back | 1.3 s |
| The publisher's | The viewer still said "playing" at 8 seconds | 3.4 s after the link came back | 3.4 s |

- **A publisher that comes back announces the same namespace again and is taken**: the relay had let the old session go by its idle timeout. Nothing had to be changed in the relay.
- **A viewer is not told that its publisher is gone for at least 8 seconds.** It learns it some time after the link is back (the run did not note when), and that is where the 3.4 seconds come from: the publisher's 2 seconds, and the viewer's.
- The encoders of the publishing page go on while it has no session; what they make then goes nowhere.

### The sound under loss

`spikes/lossy-link/audio-under-loss.ts`: the viewer's link with loss, 10 seconds, two runs each. Pieces of audio that came after their time, of about 480:

| | Loss 5% | Loss 10% |
|---|---|---|
| As it is: a stream for every chunk of 20 ms, buffer 150 ms | 143, 172 | 292, 320 |
| Buffer 400 ms (picture and sound) | 0, 0 | 105, 98 |
| 5 chunks a Group, 100 ms a stream (`chunksPerGroup`, buffer 150 ms) | 0, 0 | 100, 75 |
| 25 chunks a Group, 500 ms a stream | 0, 0 | **0, 0** |

- **With a stream for every chunk the sound falls behind and stays behind**: a piece came 171 to 397 ms after its time at the median, where the picture came within a few. With 25 chunks a stream it came 12 to 14 ms after, under the same loss. **Why a new stream every 20 ms costs this much under loss was not looked into** (what a stream needs before it can be opened, and what happens when that is lost, would be the place to look).
- A longer buffer hides it at 5% and not at 10%.
- **What several chunks in a Group cost was not measured**: a viewer that comes waits for the next Group's first chunk (up to 500 ms with 25), and a lost packet holds back what follows it on its stream. `chunksPerGroup` is an option for trying; without it the publisher does as before.

## Where the sound waits, and what a viewer is told when its publisher goes

Looked into without changing the relay: the relay run by hand with the clock put before each of its lines, loss for one way alone in the link (`up`, `down`), the own Rust client as the viewer, and the watching page with `?diag=1` (when the first bytes of each stream came) and `?video=no`.

### The sound under loss: what is known

| What was done | Seen |
|---|---|
| The relay's lines, loss 10% both ways on the viewer's link | **The relay orders a stream for every chunk on time**, 20 ms apart (`OPEN_DATA`, `FINISH_DATA`), while the page gets the sound 591 ms late (median). The publisher's side is not it |
| Loss towards the relay alone, 10% and 30% (the viewer's acknowledgements and its credit for streams) | **No delay**: 0 pieces too late, the sound 8 ms early as without loss. 3 runs |
| Loss from the relay alone, 5% and 10% (the data) | **The delay**: 174 ms at 5%, 273 and 332 ms at 10% (median), 191 to 318 pieces too late. 3 runs |
| **The own Rust client (quinn) as the viewer**, the audio track, loss 10% from the relay | **No delay**: an Object was there 1 ms after the relay ordered its stream (median), 54 and 61 ms at most, 450 Objects. 2 runs; 20 ms at most without loss |
| The page, loss 10% from the relay, `?diag=1` | **The delay only grows**, in steps of about 20 ms: the last 40 pieces came 210 to 273 ms late, in rising order. The first bytes of one audio stream and the next are 16 to 30 ms apart for most (245 of 353), 0 ms apart for 20: **what is late is not made up for**. 354 streams in 8 seconds, where there are 406 without loss |
| The same with the sound alone (`?video=no`) | The same: 412 to 475, and 431 to 572 ms late at the end of 8 seconds. The video is not part of it |

- So the sound waits **between the relay's order to open a stream and the page having its bytes, and only where Chromium receives**: with quinn at both ends of the same link it does not. And it waits as if one stream could pass every 20 ms and no faster, so that every loss puts everything after it one step back for good.
- **Not found here: which of the two it is** (it is neither: the next section). (a) The relay waits to open the stream, for the credit Chromium gives for streams: credit that comes back only as streams are done would let one stream through for each one that ends, which is the rate they are made at and no more. (b) The relay has the stream and its QUIC holds the bytes back after a loss (the congestion window), with Chromium acknowledging late when little is sent. Loss towards the relay did nothing, which speaks against credit that is lost, and not against credit that comes late.
- **What would tell**: the relay's time between ordering a stream and having it open (the hub has that event, `OutStreamOpened`, and prints no line for it). With it, (a) shows as a wait there and (b) as none.

### With the relay's line for a stream that opened: neither of the two

The relay now prints `DATA_OPENED … after_ms=… opening=… cwnd=… lost_packets=… congestion_events=…` (run with `--slow-open-ms 0`, for every stream). The sound alone (`?video=no`), loss 10% from the relay, 8 seconds:

| | The Chromium page as the viewer | The own Rust client as the viewer (2 runs) |
|---|---|---|
| The sound at the viewer | 532 ms late at the median, 721 at most, rising | 1 ms after the relay's order, 51 and 58 at most |
| `after_ms`, of about 400 streams | **0 for every one**; `opening` 0 | 0 for every one; `opening` 0 |
| `cwnd`, least and median | 2,904 bytes | 2,904 bytes |
| `lost_packets`, `congestion_events` at the end | 90, 84 | 99 and 112, 94 and 100 |

- **Not (a)**: the relay has every stream open at once. It does not wait for credit.
- **Not (b) either**: the relay's congestion window is at its least with both viewers, and only one of them gets the sound late. And the link itself was counted (it now keeps every datagram's time and length, `/log`): by the second, the streams the relay had ordered and the datagrams of an audio stream's length that had reached the viewer were 2 to 5 apart with Chromium (51/47, 100/98, 150/148, 201/197, 250/248, 300/298, 350/345, 400/395) and 0 to 3 with the Rust client. **The bytes reach the viewer's machine on time.** Half a second of lag would be 25 streams apart.
- **So the sound waits inside the browser.** With one line put into `@xmoq/moqt` for the measurement and taken out again (the time at which the browser hands an incoming stream to the page): from there to the stream's first bytes is 0 ms (40 at most), and the streams are handed over 16 to 30 ms apart, 611 ms late at the end of 8 seconds. **The page's library is not it**: with 4, 64 or 4,096 streams read at once the lag is the same (475, 472, 354 ms at the median).
- **Firefox as the viewer does not have it**: loss 10% from the relay, the sound 6 ms before its time at the median and 24 ms after at most, none too late (one run; Playwright's Firefox 157.0).
- What Chromium does between a datagram and handing the stream to the page was not looked into. It looks as if it gave the page one new stream for each that comes in, and a stream that comes late is never made up for.

**What could be done, none of it tried beyond what is above**: fewer streams (25 chunks of audio on one: no lag, measured above); the audio as datagrams, which neither implementation has; one long stream for the audio of a viewer. Nothing in the relay: it sends on time.

### A publisher that goes

The publisher's link cut and left cut, the viewer's untouched; the relay's lines with the clock, and what the page said:

| After the cut | |
|---|---|
| 124 ms | The last frame the viewer showed |
| 9,032 ms | The relay: `CONNECTION_ENDED … Lost("connection error: timed out")` for the publisher, and at once `PUBLISH_DONE status_code=0x2` to the viewer for each of its two subscriptions |
| 9,542 ms | The page: "the video track ended (subscription-ended)" (it looks every 500 ms) |
| 11,526 ms and every 2 s | The page connects again and is refused, `REQUEST_ERROR 0x10`: nobody publishes |

- **The relay tells the viewer, and at once; what takes 9 seconds is the relay noticing**: its idle timeout of 6 seconds, from the last packet it got, with the keep-alive every 3 ([open questions](../open-questions.md), "Idle timeout of the relay"). The page showed nothing wrong.
- In the run with a cut of 8 seconds above, the link was back before the relay had noticed.
- Found on the way and mended in the demo: a session of the watching page in which nothing could be played was left open.

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
