# A judge of the versions that counts what the relay gave up, and tries a higher version before it shows it

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer behind a link that is narrow (`?abr=1`, a trial). [The record of a timeout for each version](2026-10-12-a-delivery-timeout-for-each-version-of-the-video.md) ended with two things the judge could do better.

## Objective

1. **Down by what the relay gave up.** The page goes to the lower version when the picture comes 300 ms late. A stream the relay reset with DELIVERY_TIMEOUT says the same of the link, and also where a short timeout keeps the picture from ever being 300 ms late.
2. **Not up onto a link that is as narrow as before.** At the lower version the picture is in time whatever room is above it, so the page goes up every few seconds, falls behind for 2 to 4 s and comes down again. What was thought of first, how fast the Groups of the low version come, says nothing: a live Group is made in real time and comes as fast as it is made on any link that has room for it. **Tried instead**: when the judge would go up, the page subscribes to the higher version and goes on showing the lower one; the higher one is shown only when its frames have come in time for a while, and is left again unseen when they have not.

## Expected, written before the runs

The link has room for 3 Mbit/s for 10 s, then 2 for 20 s, then 3 for 25 s; the versions are 2.5 and 0.5 Mbit/s, each with a timeout of 150 ms; a Group is a second.

| | Judge | Expected |
|---|---|---|
| A | As it is | As before: down 1.7 to 4.8 s after the link got narrow; in the narrow 20 s up once more and down again (4 changes in all); 0.7 to 1.2 s of standing still there |
| B | A, and down when 2 streams were given up within 3 s | Down 2 to 3 s after the link got narrow in every run: the first Group that is late is given up 150 ms after its end, about 1.2 to 2 s in, the second a second later. **Not sooner than A at its best, and never as late as A at its worst.** The rest as A |
| C | A, and a higher version is tried for 2 s before it is shown | Down as A. In the narrow 20 s the try fails and the low version stays on the screen: **2 changes in all (down, and up at the end) where A has 4**, more frames shown there than A, less standing still. Up 2 to 3 s later than A when the link has room, which is what the try costs. The low version comes late while a try runs, since both come over the one link |
| D | B and C | Both |
| E | D with a timeout of 50 ms on the high version | Down in every run, where A with 50 ms once never went down |

## What was built for the runs

**None of it is in `main`**: it was in the working copy for the runs and taken out again, as neither rule was plainly better.

- `@xmoq/moqt` did not tell a subscriber of a subgroup stream its publisher reset: the code went no further than the session. For the runs an event of the session (`subgroup-reset`, with the Group and the code) and a count on the subscription (`streamsReset(code)`) were added, and `streamsGivenUp` in what `playVideoTrack` counts. **A reset is seen only for a stream the viewer did not have whole**: a stream it had read to its end is gone when the reset comes.
- `createQualitySwitch` took two options. `downGivenUp: 2`: down also when 2 streams of the version were given up within 3 s. `tryMs: 2000`: a higher version is received next to the one that is shown, its frames decoded and closed unseen; it is shown when 2 s have passed since its first frame, and left when its picture or that of the shown version is more than 300 ms late, or one of its streams is given up. A try that fails doubles the wait for the next, like a change that fails.
- `/watch?judge=given,try` and `?versionTimeouts=A,B`.

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; the own relay with no flag, Cubic; `spikes/lossy-link/proxy.mjs` between viewer and relay, 50 ms each way, queue 100 ms. `/publish?picture=busy&bitrate=2500000&ladder=1`, `/watch?abr=1&buffer=150`.

## Result

"Narrow" is the 20 s at 2 Mbit/s (600 frames), "wide" the 25 s after (750 frames). A has the three runs of the record before and one more.

| | Down after, s | Narrow: frames of 600 | Narrow: stood still, ms | Changes while narrow | Up after the link has room, s | Wide: frames of 750 | Plays at the end |
|---|---|---|---|---|---|---|---|
| A | 1.7, 2.9, 4.8, 3.2 | 551, 547, 538, 532 | 982, 1,215, 717, 817 | 3 in each | 1.7, 2.6, 4.5, 4.7 | 748, 744, 744, 737 | high (4 of 4) |
| B (given up) | 1.3, 2.3, 2.3 | 568, 557, 566 | 519, 567, 232 | 3 in each | 1.0, 1.8, 1.8 | 741, 746, 739 | low (2 of 3) |
| C (a try first) | 3.3, already low, 2.8 | 575, 599, 571 | 0, 332, 699 | 1, 0, 1 | never, 14.6, never | 758, 755, 757 | low (2 of 3) |
| D (both), one run | already low | 584 | 900 | 0 | 16.6 | 748 | low |

With the link at 4 Mbit/s before and after (two runs each):

| | Down after, s | Narrow: frames of 600 | Changes while narrow | Up after the link has room, s |
|---|---|---|---|---|
| A | 2.5, 3.4 | 558, 560 | 3, 3 | 2.0, 3.1 |
| C | 2.1, 3.0 | 571, 573 | 1, 1 | 24.6, never (within 25 s) |

E (50 ms on the high version) was not run.

Against what was expected:

- **B goes down sooner and more evenly** (1.3 to 2.3 s), better than expected. **"The rest as A" was wrong**: back on the high version with the link at 3 Mbit/s it went down again in all three runs, 7 to 12 s after going up, with the picture in time. The relay gives up Groups of a 2.5 Mbit/s picture on a 3 Mbit/s link now and then (a key frame fills the queue, and with 50 ms each way 150 ms leaves little), and two of them within 3 s read as a link that is too narrow. In the one run of D the same took the page down 5 s before the link got narrow at all.
- **C keeps the low version on the screen while the link is narrow**, as expected: one change there where A has three, 20 to 60 more frames, and the tries fail unseen.
- **C comes back far later than the 2 to 3 s more that were expected**: 14.6 s, or not within the 25 s, and so with room for 4 Mbit/s too. Two things: a try that began as the link got wider failed in four of the five runs (at 20.1 to 21.1 s, the link changing at 20.0; the fifth had none then), and each try that fails doubles the wait, so the tries during the narrow 20 s had put the next one 20 s off. At 3 Mbit/s there is the room for the two versions together as well: 2.5 and 0.5 and the sound are more than the link, so a try can fail on a link that the high version alone fits (the try at 42.1 s of the third run).

With the code, the checks of TypeScript (`mise run check:ts`) passed; `mise run check` passed without it. `mise run interop` was not run: nothing of `packages/` is changed in the end.

## What this says, and what it does not

- **Streams given up are a sign that comes sooner than lateness, and one that is wrong on a link with little room to spare.** Two in 3 s is not the rule. What was not tried: more of them (3 in 4 s would be as late as lateness is), or counting them only together with a picture that is somewhat late.
- **A try before going up does what it was made for on a narrow link and costs the way back.** As built it is not better than the judge that is there. What it would need: a wait that does not double for a try nobody saw (a failed try costs the viewer little, so it can come again sooner), and a rule for the try that allows for both versions being on the link at once.
- The one thing that held without a cost is the count itself: a subscriber can know which Groups its sender gave up. Nothing in `main` reads it, so it was not put in either.
- Not known: other links, other pictures, a real network; whether the breaks in the sound differ (0 to 5 in every run here).
