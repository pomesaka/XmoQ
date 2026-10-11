# The sound falls behind on a link with delay and loss, and does not catch up

- Date: 2026-10-11
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, validation over a link that loses packets. **Open: the cause is not found.**

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

## Limitations and blockers

- One or two runs each, 16 seconds, one machine. The two runs of B differ much (522 ms and 102 ms behind): how fast it falls behind follows which packets were lost.
- The contents of the datagrams are not seen (they are encrypted); the link counts bytes.
- A real background tab could not be made for `demo/check.ts`: a headed Chromium under Playwright keeps a page "visible" with another tab in front, with its window minimized, and with `Emulation.setFocusEmulationEnabled` off, and its timers run at full rate.
