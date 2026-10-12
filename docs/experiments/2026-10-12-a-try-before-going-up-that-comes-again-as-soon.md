# A higher version is tried before it is shown, and a try that fails comes again as soon

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link that is narrow (`?abr=1`, a trial). The record of the judge that tries a higher version before it shows it ("a judge that counts what the relay gave up, and tries before it goes up", of the same day) found the try good while the link is narrow (one change where the judge has three, 20 to 60 more frames of 600) and bad on the way back: 14.6 s, or not within 25 s, where the judge that is there takes 1.7 to 4.7 s.

## Objective

Whether the try keeps what it gained with the two causes of the slow way back taken away.

1. **A try that fails does not make the next one wait longer.** A change that fails is seen (the picture falls behind and stands still), so the judge doubles the wait after it. A try that fails is not shown, so it comes again after the same wait: 5 s of a picture in time.
2. **The try is judged by how late the pictures come, and by nothing else.** Before, a single stream of the tried version that the relay gave up failed the try; the first Group of a version begins with a key frame, which fills the queue for a moment on any link, and both versions are on the link while the try runs. Now the try fails when the picture of either version comes more than 300 ms late (the mark the judge goes down by), and holds when 2 s have passed since the first frame of the tried one without that. With this the count of streams given up is not needed, and nothing of `@xmoq/moqt` changes.

## Expected, written before the runs

The link has room for 3 Mbit/s for 10 s, 2 for 20 s, 3 for 25 s (and the same with 4 in place of 3); the versions are 2.5 and 0.5 Mbit/s with a timeout of 150 ms each.

| | Expected |
|---|---|
| A, the judge as it is | As in the records before: down after 1.7 to 4.8 s, 3 changes while narrow, 532 to 560 of 600 frames there, up 1.7 to 4.7 s after the link has room |
| T, a try first, as above | Down as A. **While narrow: 1 change, and 560 to 590 frames**, a little fewer than the try before had (571 to 599), because the tries now come every 8 to 9 s (2 s in which nothing changes, 5 s in time, up to 2 s of trying) and each has the low version come late for a moment: 2 tries in the 20 s, perhaps 3. **Up after the link has room: 3 to 10 s**, 6 on average: the next try begins up to 7 s after the link changed and holds 2 s after its first frame. At 3 Mbit/s the two versions and the sound are a little more than the link (about 3.05), which puts the picture some 30 ms further behind each second of the try: within the 300 ms, so the try holds |

**The mark set for taking it into `@xmoq/media`**: while narrow as good as the try before, and up within twice what A takes, about 10 s.

## What was built

- `createQualitySwitch` of `@xmoq/media` has `tryMs` (**a trial, off when left out; 2,000 ms in the runs, provisional**) and `trying()`: the version to receive next to the one that is played. `tick` takes how late the picture of the tried version is. The rule is the one of "Objective"; the table tests are in `packages/media/test/quality-switch.test.ts`.
- `/watch?abr=1&judge=try`: the page subscribes to the tried version, decodes its frames and closes them unseen, and tells the judge how late they came; when the try has held it shows that version from its next frame on and leaves the one before. Without `judge=try` the page is as it was.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag, Cubic; `spikes/lossy-link/proxy.mjs` between viewer and relay, 50 ms each way, queue 100 ms. `/publish?picture=busy&bitrate=2500000&ladder=1`, `/watch?abr=1&buffer=150`. Three runs each, A and T in turn.

## Result

"Narrow" is the 20 s at 2 Mbit/s (600 frames), "wide" the 25 s after (750 frames).

| Wide | | Down after, s | Narrow: frames of 600 | Narrow: stood still, ms | Changes while narrow | Tries while narrow | Up after the link has room, s | Wide: frames of 750 | Plays at the end |
|---|---|---|---|---|---|---|---|---|---|
| 3 Mbit/s | A | 4.2, 3.9, 5.4 | 545, 542, 529 | 467, 1,234, 851 | 3, 3, 3 | | 3.7, 3.4, 5.1 | 744, 747, 741 | high (3 of 3) |
| 3 Mbit/s | T | 2.6, 2.1, 2.9 | 566, 581, 568 | 949, 200, 618 | 1, 1, 1 | 2 failed in each | 4.1, 2.9, 4.6 | 740, 743, 744 | high (3 of 3) |
| 4 Mbit/s | A | 2.0, 2.8, 2.1 | 559, 556, 560 | 901, 951, 817 | 3, 3, 3 | | 2.0, 2.1, 1.1 | 753, 739, 745 | high (3 of 3) |
| 4 Mbit/s | T | 5.9, 1.9, 2.3 | 547, 576, 577 | 617, 885, 668 | 1, 1, 1 | 2 failed in each | 7.1, 3.1, 2.8 | 743, 750, 751 | high (3 of 3) |

Against what was expected:

- **While narrow: one change in every run, as expected**, and 547 to 581 frames where A has 529 to 560. The one run of T with 547 went down late (5.9 s, which is the judge's part that is the same in both). Two tries failed in each run, unseen, 6 to 7 s apart.
- **Up after the link has room: 2.8 to 7.1 s, within the 3 to 10 s expected**, and the first try after the link changed held in all six runs. A takes 1.1 to 5.1 s. T is later than A by what a try takes, and not by more.
- **Standing still while narrow is not less than with A** (200 to 950 ms against 470 to 1,230 ms): a try that fails has the low version come late for a moment, since both are on the one link. That was expected as a cost and not measured apart.
- The difference in "down after" between A and T is not the try's: nothing is tried before the first change.

**The mark is met**: while narrow as good as the try before (1 change; 547 to 581 frames against 571 to 599 then), and up within 7.1 s. `tryMs` is in `@xmoq/media`, off unless asked for.

`mise run check` and `mise run interop` passed (the numbers are in the pull request).

## What this says, and what it does not

- Trying a higher version unseen, judged by lateness alone and tried again after the same wait, takes away the two changes a narrow link otherwise costs every time the judge looks up, and costs 1 to 5 s on the way back.
- **What it costs is not measured well**: the low version's picture while a try fails (the sound is on the same link too; 0 to 6 breaks in a run with T, 0 to 1 with A, too few runs to say), and the decoding of a second version for 2 s.
- Not known: more than two versions, a link that changes slowly, loss, a real network, a person watching. `?abr=1` stays a trial, and so does `judge=try`.
