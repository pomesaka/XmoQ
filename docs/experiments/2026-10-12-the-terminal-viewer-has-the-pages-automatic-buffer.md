# The terminal viewer has the page's automatic buffer

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal; Phase 2, quality under a narrow link

## Objective

Behind a narrow link the terminal viewer's fixed buffer of 50 ms lost a third to a half of the sound ([record](2026-10-12-the-terminal-viewer-asks-for-a-delivery-timeout.md)). Whether the buffer that follows how late Objects come, which the watching page has as `?buffer=auto`, can be the same rule in `xmoq-watch`, and what it does on that link.

## What was built

`xmoq-watch --buffer-ms auto` (a trial; the default stays 50 ms). The rule and every value are those of `@xmoq/media` (`packages/media/src/core/playout-clock.ts`, `audio-seam.ts`): what 99 of 100 Objects of the last 10 seconds came within, 10 ms more, steps of 10 ms, 400 ms at most, growing at once and shrinking a step twice a second; a packet of sound follows the one before it to the sample, and is played up to a twentieth slower or faster where its time has moved by more than 8 ms. Where the two differ is in a table in [the viewer's README](../../crates/watch/README.md).

**The seam rule holds with a buffer that is set as well**, so the sound of every run below is placed by it; before, a packet began at its own time or was cut. The sound now waits 30 ms at least, also where `--buffer-ms` is less.

Six rows for the buffer and nine for the seam are shared (`fixtures/playout/`): the tests of `crates/watch` and of `@xmoq/media` read the same files.

## Environment

As in the record above: one machine, loopback, the demo's relay with no flag, the publishing page in headless Chromium at 2.5 Mbit/s, `spikes/lossy-link/proxy.mjs` with 2,000 kbit/s, a queue of 100 ms and 50 ms each way between the relay and the viewer. `--sound off`, the picture to `/dev/null`.

## Commands

```sh
crates/watch/scripts/try-narrow-link.sh 25 --buffer-ms auto|50|300 [--video-timeout-ms 100]
```

## Observed results

Two runs of each, about 24 seconds; about 535 frames and 1,200 packets of sound came in each.

| Buffer | Timeout of the video | Buffer at the end, ms | Frames drawn | Sound played, s | Underruns | Packets over when they came | Packets played slower or faster |
|---|---|---|---|---|---|---|---|
| auto | none | 400, 400 | 150, 150 | 23.5, 23.1 | 42, 56 | 12, 28 | 330, 198 |
| auto | 100 ms | 400, 400 | 531, 527 | 23.3, 23.4 | 45, 25 | 24, 13 | 27, 57 |
| 50 ms | none | 50 | 147, 150 | 13.1, 8.3 | 1,014, 1,552 | 535, 767 | 219, 261 |
| 50 ms | 100 ms | 50 | 532, 538 | 17.2, 17.0 | 667, 681 | 333, 343 | 130, 65 |
| 300 ms | none | 300 | 148, 149 | 23.7, 23.7 | 0, 0 | 0, 0 | 196, 179 |
| 300 ms | 100 ms | 300 | 531, 533 | 23.7, 23.7 | 0, 0 | 0, 0 | 74, 63 |

- **The automatic buffer keeps the sound where 50 ms does not**: 23.1 to 23.5 seconds of about 24 are played, against 8 to 17. Its underruns (25 to 56) and the packets that were over are of the beginning, before 50 Objects had come and the buffer had grown; when in the run they fell was not written down, and that is read from the counts alone.
- **It went to its limit, 400 ms, in every run**, with the timeout as without. The frames count as the packets do, and on this link the frames come 260 to 300 ms late at the median with the timeout and seconds late without it; the sound alone came 46 to 64 ms late. A fixed 300 ms played all of the sound with less delay. The page's rule looks at both tracks together, and it was taken as it is.
- The picture is what the timeout decides, as before: about 530 frames drawn with it, about 150 without, whatever the buffer.
- **Packets are played slower or faster with a fixed buffer too** (63 to 261 a run). Not looked into. The fastest way an Object took is found as the run goes on, and each time a faster one comes every packet's time moves; whether that is all of it was not measured.

## Not measured

- **Nobody listened**: whether a twentieth slower or faster is heard, and whether the seams are clean, is not known for the viewer. The page's seams were listened to.
- A link without congestion, loss, other limits than 400 ms, the buffer shrinking again after the link gets better.
- Picture and sound against each other (the marks) with a buffer that moves.

## Next action

None from this record. Whether the automatic buffer should go by the sound alone, or by each track for itself, is a question for both players at once.
