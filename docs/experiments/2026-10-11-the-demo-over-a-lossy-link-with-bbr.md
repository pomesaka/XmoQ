# The demo over a link with delay and loss: the relay with Cubic and with BBR

- Date: 2026-10-11
- Commit SHA: the commit that adds this record (the relay as of #132, which has `--congestion`)
- Roadmap item: Phase 1, validation over a link that loses packets

## Objective

A Rust viewer showed that the relay with `quinn`'s default congestion control (Cubic) falls behind for good on a link with delay and loss, and with BBR does not. Is that so for the demo in a browser, for the picture and for the sound?

Expected, written before the runs: with Cubic the delay of the picture grows through the run where the bitrate is high and packets are lost; with BBR it does not; the sound plays with both.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback; publisher, relay and viewer on the one machine |
| Viewer | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the watching page, buffer 150 ms, the sound put in order |
| Relay | own, `--congestion cubic` and `--congestion bbr` |
| Link | `spikes/lossy-link/proxy.mjs` between the viewer and the relay, 50 ms each way, with loss each way; the publisher goes to the relay directly |
| Picture | "clock": the demo's, about 450 kbit/s. "busy": `?picture=busy&bitrate=2500000`, about 2.65 Mbit/s. 640x360, 30 frames a second. Sound: Opus, about 67 kbit/s |

## Commands

```sh
node spikes/lossy-link/proxy.mjs --listen 45386 --to 45382 --control 45387
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 --congestion cubic   # and bbr
# a script of this run: for each link and picture one viewer through the link for 20 s (600 frames, about 1,000 pieces of sound)
```

## Observed results

One run of 20 seconds each. "Came after": from when a frame was made to when it was decoded in the viewer, the median of the last 200 frames, read at 5 and at 20 seconds (ms). "Shown after": the same to the repaint.

| Relay | Picture | Loss | Through the link, kbit/s | Picture came after, at 5 s and 20 s | Shown after, median and most | Frames shown of 600 | Sound came after, at 5 s and 20 s | Sound: sounded of decoded | Breaks |
|---|---|---|---|---|---|---|---|---|---|
| Cubic | clock | 0 | 531 | 51, 51 | 184, 192 | 606 | 30, 29 | 1,007 of 1,008 | 4 |
| BBR | clock | 0 | 523 | 52, 53 | 196, 206 | 616 | 42, 42 | 1,008 of 1,022 | 4 |
| Cubic | clock | 1 % | 521 | 52, 53 | 193, 253 | 591 | 46, 46 | 982 of 995 | 5 |
| BBR | clock | 1 % | 518 | 52, 52 | 187, 195 | 602 | 33, 33 | 983 of 997 | 3 |
| Cubic | clock | 5 % | 494 | **94, 204** | 211, 1,252 | **307** | 66, 101 | 902 of 978 | 21 |
| BBR | clock | 5 % | 507 | 53, 53 | 190, 203 | 573 | 36, 36 | 965 of 970 | 6 |
| Cubic | busy | 0 | 2,712 | 55, 55 | 190, 198 | 603 | 40, 42 | 984 of 997 | 2 |
| BBR | busy | 0 | 2,719 | 54, 55 | 189, 197 | 604 | 39, 39 | 983 of 996 | 0 |
| Cubic | busy | 1 % | **1,105** | **655, 1,145** | 1,150, 2,987 | **43** | 72, 91 | 793 of 870 | 35 |
| BBR | busy | 1 % | 2,719 | 55, 55 | 189, 198 | 595 | 40, 41 | 983 of 996 | 3 |
| Cubic | busy | 5 % | **555** | **654, 2,092** | 2,092, 4,483 | **25** | 186, 935 | **341 of 631** | 89 |
| BBR | busy | 5 % | 2,719 | 128, 129 | 196, 269 | 520 | 44, 48 | 966 of 992 | 3 |

With the automatic buffer (a trial), busy, 1 %: Cubic went to its limit of 400 ms and showed 37 frames of 600, the picture 1,338 ms behind at the end; BBR settled at 140 ms and showed 587, shown 188 ms after.

- **As expected for the picture**: with Cubic and loss the relay sends less than is published (1,105 of 2,700 kbit/s at 1 %, 555 at 5 %), the picture falls behind through the run, and nearly no frame is shown. With BBR what is published passes the link at every loss, and the delay does not grow.
- **More than expected**: Cubic does not hold the small picture either at 5 % (about 450 kbit/s: 307 frames of 600, the delay growing).
- **Not as expected for the sound**: with Cubic, the busy picture and 5 %, a third of the sound did not come within the run and half of what came was sounded. The sound shares the connection with the picture, and waits behind it.
- With BBR at 5 % and the busy picture, 520 of 600 frames were shown: 44 were dropped as late (a Group that came after the next had begun) and the frames came 128 ms after they were made, not 55.

## Limitations and blockers

- One run each, 20 seconds, one machine, loss that is random and the same each way; no limit on the rate of the link. What BBR does to other traffic on a shared link, and on a link with a narrow rate, was not looked at.
- The viewer's numbers are read from the page; "came after" compares the publisher's clock and the viewer's, which are one clock here.
- The buffer was 150 ms throughout, but for the one run each with the automatic one.
