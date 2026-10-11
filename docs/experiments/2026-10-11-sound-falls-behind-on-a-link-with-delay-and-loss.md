# The sound falls behind on a link with delay and loss, and does not catch up

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, validation over a link that loses packets. **The cause was found afterwards, and it is not the link: see "The cause" at the end.**

## Objective

With 50 ms each way and 1 % loss, a Chromium viewer sounded none of the pieces of audio at any buffer up to 400 ms (docs/experiments/2026-10-11-where-the-delay-of-the-demo-comes-from.md). Does the relay keep up with what is published? Does the sound alone play, or with a smaller picture: is it the two tracks competing for the link?

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Viewer | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless, the watching page of the demo with a buffer of 150 ms |
| Relay | own (`quinn`'s default congestion control), `--slow-open-ms 0` |
| Link | `spikes/lossy-link/proxy.mjs` between the viewer and the relay; the publisher goes to the relay directly |
| Sound | Opus, 10 pieces of 20 ms in a Group, about 67 kbit/s through the link |

## Commands

```sh
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 --relay-log relay.log
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
# a script of this run: one viewer through the link for 16 s; every second the kbit that passed the link
# towards the viewer (/log), how long after it was made the sound came (median of the last 25 pieces),
# and the pieces sounded and decoded
```

## Observed results

How long after it was made the sound came, in milliseconds, at seconds 1, 4, 8, 12 and 16; one run each:

| Run | Link | What the viewer takes | Through the link | 1 | 4 | 8 | 12 | 16 | Sounded |
|---|---|---|---|---|---|---|---|---|---|
| A | 50 ms, 1 % | picture (1.5 Mbit/s asked) and sound | about 500 kbit/s every second | 41 | 82 | 195 | 442 | 543 | none after second 8 |
| A again | the same | the same | about 500 | 40 | 161 | 600 | 601 | 802 | none after second 4 |
| B | 50 ms, 1 % | **the sound alone** | about 69 kbit/s | 41 | 241 | 461 | 522 | 522 | none after second 3 |
| B again | the same | the same | about 69 | 40 | 61 | 80 | 81 | 102 | all (still within the buffer) |
| C | 50 ms, 1 % | picture at 200 kbit/s and sound | about 260 | 39 | 60 | 80 | 362 | 859 | none after second 10 |
| D | **1 %, no delay** | picture and sound | about 500 | -8 | -7 | -8 | -8 | -7 | all |
| E | **50 ms, no loss** | picture and sound | about 500 | 41 | 41 | 41 | 41 | 41 | all |

- **The sound comes later in steps and never comes back**: 20, 60 or 200 ms at a time. No piece is lost: about 50 a second are decoded throughout. It needs both the delay and the loss.
- **It is not the two tracks competing for the link.** The sound alone, at 69 kbit/s, falls behind in the same way, and so does the sound beside a small picture.
- **What passes the link keeps up with what is published**: the same kbit every second, from the first to the last.
- **The relay opens every stream at once** (`after_ms=0` on every `DATA_OPENED`, all runs).
- The relay's congestion window, as its lines say: with the sound alone it went 12,000, 5,880, 4,116, 2,904 bytes in 16 seconds (7 packets lost) and did not grow again; with the picture as well it stayed near 10,000 (13 lost); with loss and no delay it went to 5,723 (20 lost), and nothing was late. Round trip 103 ms. A window of 2,904 bytes in 103 ms is about 225 kbit/s, above the 69 the sound needs.
- **Not found: where the pieces wait.** They are sent at the rate they are made, and come at that rate, a constant time behind. Whether they wait in the relay's sender or in the browser is not told apart by these numbers; a viewer that is not Chromium through the same link would tell.

**The steps and the lost packets, from the lines that were kept** (no new run). The second run of B, the sound alone: the relay's count of lost packets went up at the streams 7, 12, 32, 55 (by two), 63 and 68, each stream 200 ms of sound: about 1.4, 2.4, 6.4, 11, 12.6 and 13.6 seconds after the SUBSCRIBE. The sound came 20 ms later at about 3.8, 7.8 and 13.8 seconds.

- **Not one step for each lost packet**: 7 packets lost towards the viewer, 3 steps. Each step comes within about 1.5 seconds after a loss; 4 losses have no step.
- **A step is not the round trip** (103 ms) nor a wait for a packet to be sent again: here each was 20 ms, the length of one piece; in the other runs 20, 60 and 200 ms were seen, all of them whole pieces.
- How coarse this is: the lateness is the median of 25 pieces, read once a second, and the packets lost towards the relay (the viewer's acknowledgements) are not counted in the relay's number. A reading for each piece, and the time of every packet the link dropped, were not kept.

## The cause: the decoder's times, not the link

**The sound was never late. The times that Chromium's `AudioDecoder` gives its pieces were early.**

With a probe in the page (taken out again), one run of 20 seconds with the sound alone, 50 ms and 1 %:

| Where | How long after it was made |
|---|---|
| The Object is given to the player (`playAudioTrack`), 971 Objects | 24 ms at least, median 30, at most 155: no step in 20 seconds |
| The piece comes out of the decoder, by its `timestamp` | 26 ms at first, 370 ms at the end |

- 17 of the 971 Objects were dropped by the player as late (an Object of a Group that came after the next Group had begun), and 17 pieces of 20 ms are 340 ms: the difference.
- **Chromium 156 numbers the decoder's output from the time of the first chunk, by the samples it has given**, and does not look at the times of the chunks after it. Each chunk that does not go into the decoder leaves everything after it 20 ms early, for good. The demo plays by the time, took the pieces for late, and sounded none.
- That is why a step is a whole number of pieces, why not every lost packet makes one, and why it needs delay and loss together: only then does the rest of a Group come after the next Group has begun.
- **Changed in `@xmoq/media`**: `playAudioTrack` keeps the times of the chunks it gives the decoder, and gives each piece the time of its chunk (a copy with that time, where the decoder's differs by a millisecond or more).

After the change, the same link, 16 seconds each, one run:

| Run | Link | Sound came, ms after it was made, at seconds 1, 8, 16 | Sounded | Dropped as late |
|---|---|---|---|---|
| A | 50 ms, 1 %; picture and sound | 33, 33, 34 | all | not read |
| B | 50 ms, 1 %; the sound alone | 43, 41, 40 | all | not read |
| F | 50 ms, 5 %; picture and sound | 42, 87, 58 | all but 6 | **190 of about 780** |
| G | 5 %, no delay; the sound alone, one piece a stream | -8, -7, -8 | all | 30 |
| G, the change taken out for the run | the same | 88, 391, 770 | none after second 2 | 39 (39 pieces are 780 ms) |
| H | 50 ms, 1 %; the sound alone, one piece a stream | 42, 43, 42 | all | 11 |

- **This takes back a finding of the day** (docs/experiments/2026-10-11-demo-over-a-lossy-link.md and what followed it): "with loss the sound comes late, and it is inside Chromium that streams are handed to the page late". Run G is that case, and it is the same thing: the pieces were on time, their times were not. That Firefox and the Rust viewer showed no lateness fits: neither goes by Chromium's decoder. "10 pieces in a Group keep it away" held because fewer Objects are dropped as late that way, not because streams came sooner.
- **What is left, and real**: an Object that comes after a later one is dropped, and with loss that is many (190 of about 780 at 50 ms and 5 %): gaps in the sound. Putting them back in order within the buffer is not done.
- No test holds the change: there is no `AudioDecoder` outside a browser. Firefox was not run again.

## Afterwards: Objects that come late are put in order

`playAudioTrack` got `holdForOrderMs`: an Object after a gap waits that long for the ones before it, by the times, and is then decoded without them (`packages/media/src/core/in-order.ts`). The demo gives it its buffer less 20 ms (provisional); `?order=no` is as before. The sound alone through the link, about 17 seconds (about 840 pieces made), one run each:

| Link | Buffer | Objects | Decoded | Dropped as late | Too late to sound | Breaks in what leaves |
|---|---|---|---|---|---|---|
| 50 ms, 5 % | 150 ms | put in order | 822 | **0** | 10 | 5 |
| 50 ms, 5 % | 150 ms | as they come | 750 | 46 | 2 | 30 |
| 50 ms, 5 % | 300 ms | put in order | 797 | 0 | 5 | 3 |
| 50 ms, 5 % | 300 ms | as they come | 724 | 57 | 0 | 23 |
| 50 ms, 1 % | 150 ms | put in order | 796 | 0 | 0 | 1 |
| 50 ms, 1 % | 150 ms | as they come | 772 | 13 | 0 | 13 |

- Nothing is dropped as late any more, and the breaks go from 30 to 5 at 5 %. A few pieces now come out of the wait too late to sound (10 of 822): they waited the whole hold.
- The pieces that are missing from "decoded" in every row did not come within the run (the link held them), or the page began later; not looked at one by one.
- **The decoder is not told of a gap**: WebCodecs has no way to say that a chunk is missing. What Opus makes of the chunk after a gap was not listened to.
- With the demo's default buffer of 50 ms the hold is 30 ms, less than it takes to send a lost packet again over 50 ms each way: there it helps little. Not measured.

## Limitations and blockers

- One or two runs each, 16 seconds, one machine. The two runs of B differ much (522 ms and 102 ms behind): how fast it falls behind follows which packets were lost.
- The contents of the datagrams are not seen (they are encrypted); the link counts bytes.
- A real background tab could not be made for `demo/check.ts`: a headed Chromium under Playwright keeps a page "visible" with another tab in front, with its window minimized, and with `Emulation.setFocusEmulationEnabled` off, and its timers run at full rate.
