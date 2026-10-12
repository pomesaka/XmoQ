# A delivery timeout for each version of the video, with a viewer that changes between them

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link that is narrow. [On a link narrower than the video the picture comes about 375 ms late whatever is given up of it](2026-10-12-a-timeout-and-a-buffer-that-follow-the-sound.md); what brings it near is the lower version (`?abr=1`), which the page goes to 2 to 4 s after the link got narrow when it asks for the default timeout of 150 ms.

## Objective

Whether the timeout should differ before and after the change of versions: on the high version, which does not fit the link, and on the low one, which does.

**Each version is a track and a subscription of its own**, so a timeout for each needs no REQUEST_UPDATE: it is the timeout of that version's SUBSCRIBE.

## Expected, written before the runs

The link has room for 3 Mbit/s, then for 2, then for 3 again; the high version is 2.5 Mbit/s and the low one 0.5.

| | High version | Low version | Expected |
|---|---|---|---|
| a | 150 ms | 150 ms | What was seen before: down 2 to 4 s after the link got narrow |
| b | 150 ms | none | **As a, in every number.** The low version fits the link, so no Group of it is 150 ms late and the timeout has nothing to give up |
| c | 50 ms | 150 ms | **Down later than a, or not at all, and not sooner.** The page goes down when the picture comes 300 ms late; a shorter timeout gives up more and keeps the picture nearer, which is the opposite of what would make it go down. More frames are lost while it stays |
| d | none | 150 ms | Down soonest (1 to 2 s, as without a timeout before), with the picture furthest behind until then |

Going up again when the link has room: the same in all four, since the judge wants the picture within 100 ms for 5 s and the low version is that in each.

If this holds, a timeout for each version gains nothing and what would is a judge that sees what the relay gave up; nothing is built into `@xmoq/media` then.

## What was run

**The code of this trial is not in `main`**: `/watch?versionTimeouts=A,B`, which gave each version of the catalog its own timeout in the SUBSCRIBE the page sends for it, was in the working copy for the runs and taken out again, as nothing came of it.

`/publish?picture=busy&bitrate=2500000&ladder=1` and `/watch?abr=1&buffer=150&versionTimeouts=…`. The link: 3,000 kbit/s for 10 s, 2,000 for 20 s ("narrow", 600 frames), 3,000 for 25 s ("wide", 750 frames); 50 ms each way, queue 100 ms. Three runs each. The judge is `createQualitySwitch` with its defaults: down at 300 ms late, up after 5 s within 100 ms, twice as long after each fall back.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag, Cubic; `spikes/lossy-link/proxy.mjs` between viewer and relay.

## Result

| | High, low | Down after, s | Narrow: frames of 600 | Narrow: stood still, ms | Narrow: streams the relay reset | Wide: frames of 750 | Wide: stood still, ms | Plays at the end | Changes |
|---|---|---|---|---|---|---|---|---|---|
| a | 150, 150 | 1.7, 2.9, 4.8 | 551, 547, 538 | 982, 1,215, 717 | 6, 7, 8 | 748, 744, 744 | 0, 0, 233 | high (3 of 3) | 4, 4, 4 |
| b | 150, none | 3.4, 3.4, 2.1 | 555, 549, 537 | 301, 586, 399 | 7, 7, 9 | 753, 745, 723 | 0, 0, 101 | high (3 of 3) | 4, 4, 4 |
| c | 50, 150 | 2.1, 2.6, never | 529, 538, 446 | 1,349, 1,200, 1,099 | 9, 7, 19 | 720, 739, 726 | 351, 116, 518 | high (3 of 3) | 4, 4, 0 |
| d | none, 150 | 1.9, 1.9, 1.9 | 547, 556, 554 | 1,132, 683, 919 | 0 | 741, 740, 742 | 4,510, 0, 0 | low (2 of 3) | 5, 4, 5 |

Against what was expected:

- **b is as a**, as expected: the frames are the same. It stood still for less in the narrow phase (300 to 590 ms against 720 to 1,220), in three runs each, which is too few to say it does.
- **c is not sooner, and once it never went down**, as expected: in that run the page stayed on the high version for all 20 s, the relay reset 19 streams, and 446 of 600 frames were shown. With 50 ms the picture is kept from being 300 ms late, which is the one thing the page goes down by.
- **d goes down soonest and most evenly** (1.9 s in all three), as expected, and shows as many frames in the narrow phase. **What was not expected is what it costs later**: back on the high version with no timeout, the first Group that is late is not given up, and in two of three runs the page fell back to the low version within 2 to 4 s of going up; in one the picture stood still for 4.5 s.
- **Going up is not the same in all four**, against what was expected, and not by the timeout: it is 0.1 to 4.5 s after the link got room, because it comes when the judge's wait is over, wherever in that wait the link changed.
- One run of b had 544 breaks in the sound, where the others have 0 to 3. Not looked into; nothing else of that run differs.

**What the runs show that was not asked**: in the narrow phase every run that went down went up again about 6 s later, with the link as narrow as before, and down again 2 to 4 s after that (four changes where two would do). The low version comes in time, which is all the judge sees; it has no word of whether the link has room for the high one. Each such try is 2 to 4 s of a picture that falls behind and a change back. The judge waits twice as long before the next (11 s), so a longer narrow phase has fewer of them.

`mise run check` passed. Nothing of `packages/`, `crates/` or `interop/` was changed.

## What this says, and what it does not

- **A timeout for each version gains nothing here.** The default of 150 ms on both is as good as any of the four, and the two that differ on the high version are worse: 50 ms can keep the page from going down at all, none makes the way back up fail.
- **Changing a timeout while a version plays (REQUEST_UPDATE) was not needed for this**: the versions are subscriptions of their own. Where it could matter is a viewer with one version.
- What would gain something is the judge, not the timeout: going down by what the relay gave up (a stream reset with DELIVERY_TIMEOUT says the link is too narrow sooner than 300 ms of lateness does, and also where a short timeout hides the lateness), and not going up on a link that is as narrow as before (which needs a measure of the link's room, such as how fast the Groups of the low version come, or a try that is cheaper than a change). Neither was built.
- Not known: other links and pictures, a real network, more than two versions, more than three runs.
