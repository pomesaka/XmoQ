# The relay follows the delivery timeout a viewer asks for

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer whose link has less room than the video; the open decisions on what the relay gives up for a slow subscriber and on how many pieces of sound share a Group.

## Objective

The relay's experiments held for every track of every viewer, and what suits video broke sound ([record](2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md), the last section). Draft 18 lets a subscriber say a delivery timeout for one subscription ([record](2026-10-12-delivery-timeouts-of-a-subscription-what-draft-18-has-and-who-does-what.md)). With the relay acting on it: does a viewer that asks for a timeout on its video alone get the video's gain and keep its sound, in the cases where the sound broke?

Expected, written before the runs: with no flag and `?videoTimeout=500` the page sees what it saw with `--delivery-patience-ms 500`; with `--older-groups give-up` the video is as it was with that flag, and the sound is as with no flag at all, also with no grace and also with a piece of sound a Group at 5% loss.

## What the relay does now

**Provisional**, decided for this change on 2026-10-12; `docs/moqt-implementation-notes.md` has it among the known gaps, with what is left out.

- **It acts on SUBGROUP_DELIVERY_TIMEOUT of a SUBSCRIBE** (§8, §10.2.3): a subgroup stream of that subscription that the subscriber's transport has not acknowledged whole so long after the relay wrote its FIN is reset with DELIVERY_TIMEOUT (0x2). 0 is no timeout. The mechanism is the one `--delivery-patience-ms` had, now with a time for each stream.
- **`--delivery-patience-ms` is what a subscriber gets that says nothing, and the smaller of the two holds where both are there** (§8: "if both the publisher and the subscriber have a non-zero value, the smaller of the two is used"). With neither, nothing is given up, as before.
- **`--older-groups give-up`, with its grace, is applied only to a subscription that set a SUBGROUP_DELIVERY_TIMEOUT.** A subscriber that sets none on a track gets every stream of it whole (§8: "if none of the timeouts are set to a non-zero value, all Objects ... are delivered").
- OBJECT_DELIVERY_TIMEOUT is read and not acted on; the Track Property of a publisher and REQUEST_UPDATE are as before (ignored, refused); nothing is sent upstream.

In the code: `xmoq-moqt` has `params::subgroup_delivery_timeout`; `xmoq-relay-core` keeps it with the subscriber and hands it on with every `Action::OpenDataStream`; `OutgoingSubgroup::spawn` of `xmoq-moqt-io` takes the stream's own time and uses the smaller of it and `SendLimits::delivery_patience`; the hub gives older Groups up only where the action has a timeout. Nothing changed in `@xmoq/moqt` or `@xmoq/media`: they are the side that asks.

Tests, all on paused time: the smaller of the two in the writer (six rows); the timeout of a SUBSCRIBE in the action (five rows, 0 and the Object's timeout among them); and the relay end to end on an in-memory connection that delivers a finished stream when the test says so (`memory::pair_that_delivers_when_told`, new): reset after the smaller of flag and request (seven rows), and older Groups given up only for a subscription that asked (four rows). The SUBSCRIBE that moqtail npm 0.13.0 serializes with both timeouts is now a shared file, `fixtures/moqt-18-from-libraries/`, which both implementations decode and encode.

## Environment

As in the record of the narrow link: macOS 14.4, arm64, loopback; Chromium 156.0.8078.4, headless, buffer 150 ms; the own relay, Cubic; `spikes/lossy-link/proxy.mjs`, 50 ms each way. The viewer is `/watch?videoTimeout=500`: SUBGROUP_DELIVERY_TIMEOUT 500 on the video's SUBSCRIBE, nothing on the sound's. Two runs each, 20 s.

## Result

### The narrow link (the busy picture at 2.5 Mbit/s)

Frames shown of 600; the picture's lateness at 20 s; pieces of sound that sounded of 1,000, and breaks.

| Relay | Link | Sound | Frames | Picture came after, ms | Sound: sounded, breaks | The same relay with no timeout asked (the records before) |
|---|---|---|---|---|---|---|
| no flag | 2.5 Mbit/s | 10 a Group | 444, 394 | 420, 428 | 972, 980; 2, 0 | 328, 338 frames, 570 ms and growing |
| no flag | 2.5 Mbit/s | 1 a Group | 441, 438 | 434, 428 | 981, 981; 0, 1 | |
| no flag | 2 Mbit/s | 10 a Group | 356, 355 | 528, 525 | 962, 969; 2, 2 | 126, 129 frames, 730 ms and growing |
| no flag | 2 Mbit/s | 1 a Group | 350, 345 | 533, 530 | 979, 983; 1, 0 | |
| give up, no grace | 2.5 Mbit/s | 10 a Group | 520, 529 | 200, 193 | **974, 980; 1, 0** | sound: 870, 864 sounded; 50, 47 breaks |
| give up, no grace | 2.5 Mbit/s | 1 a Group | 521, 510 | 212, 187 | **983, 980; 0, 1** | sound: 375, 225 sounded; 202, 163 breaks |
| give up, no grace | 2 Mbit/s | 10 a Group | 418, 408 | 296, 298 | **972, 965; 0, 4** | sound: 898, 855; 39, 45 |
| give up, no grace | 2 Mbit/s | 1 a Group | 403, 405 | 296, 294 | **975, 969; 0, 2** | sound: 204, 302; 143, 184 |
| give up, grace 100 ms | 2.5 Mbit/s | 10 a Group | 517, 524 | 233, 252 | 971, 974; 3, 0 | |
| give up, grace 100 ms | 2.5 Mbit/s | 1 a Group | 524, 511 | 243, 251 | 978, 978; 1, 1 | |
| give up, grace 100 ms | 2 Mbit/s | 10 a Group | 419, 385 | 347, 352 | 961, 981; 4, 0 | |
| give up, grace 100 ms | 2 Mbit/s | 1 a Group | 409, 407 | 354, 350 | 975, 974; 2, 3 | |

- **With no flag at the relay, a viewer that asks gets what the flag gave**: 394 to 444 frames and about 425 ms behind at 2.5 Mbit/s (444 and 437 with `--delivery-patience-ms 500`), where one that does not ask has 328 and 338 and falls further behind. The relay logged `DELIVERY_TIMED_OUT` 149 times in the eight runs.
- **The sound is kept in every row**, 961 to 983 pieces sounded and 0 to 4 breaks: with no grace as well, where it had 39 to 202 breaks while the rule held for every track, and with a piece a Group as with ten.
- The video with giving up is as it was: 510 to 529 frames at 2.5 Mbit/s and about 200 ms behind with no grace, about 245 ms with 100 ms of grace.

### A link with loss and room to spare (the clock, 640x360)

| Relay | Loss | Sound | Frames | Picture came after, ms | Sound: sounded (received), breaks | With the rule on every track (the record before) |
|---|---|---|---|---|---|---|
| give up, grace 100 ms | 1% | 10 | 591, 582 | 54, 55 | 981 (985), 961 (966); 2, 3 | |
| give up, grace 100 ms | 1% | 1 | 584, 576 | 53, 57 | 972 (976), 978 (979); 3, 2 | |
| give up, grace 100 ms | 5% | 10 | 401, 368 | 127, 150 | 907 (985), 923 (966); 20, 8 | sound: 864 (927), 866 (947); 23, 31 |
| give up, grace 100 ms | 5% | 1 | 366, 469 | 171, 120 | **902 (953), 939 (967); 18, 15** | sound: 728 (898), 774 (913); 48, 44 |
| give up, no grace | 1% | 10 | 590, 594 | 53, 54 | 977 (986), 980 (982); 1, 0 | |
| give up, no grace | 1% | 1 | 592, 585 | 54, 52 | 974 (987), 977 (988); 1, 9 | |
| give up, no grace | 5% | 10 | 409, 393 | 109, 131 | 911 (972), 890 (974); 12, 21 | |
| give up, no grace | 5% | 1 | 332, 432 | 96, 137 | 881 (919), 943 (987); 17, 13 | |

- **The case that broke is whole again**: at 5% loss with a piece of sound a Group, 902 and 939 pieces sounded with 15 and 18 breaks, where 728 and 774 sounded with 44 and 48 while the rule held for the sound too; without any flag it was 883 and 926 with 10 and 19.
- The video under loss is as with the flag before, more frames than with no flag at 5% (332 to 469 against 225 to 303) and the same at 1%.
- In some runs the viewer received fewer pieces of sound than were sent in the 20 s (919 and 953 of about 985 at 5%, 966 at both), though nothing gives sound up now. Not looked into; pieces behind a lost packet at the end of the run are one way.

`mise run check` and `mise run interop` (125 of 125) passed.

## What this says for the decisions, and what it does not

- **The two decisions no longer hang together.** With the rule applied by subscription, what the relay gives up of the video does nothing to the sound, at either size of a Group of sound and with or without a grace. The question of ten pieces or one can be answered by [its own record](2026-10-12-one-five-or-ten-pieces-of-sound-a-group.md).
- **A default for the relay need not be a flag at all**: with no flag, a viewer that asks for 500 ms on its video has what the flag gave, and one that asks for nothing has every Object. Which of the relay's rules a viewer's timeout switches on is the open part: today a timeout alone gets "so long after a Group's end", and "at the next Group" needs the flag as well, though it was the better of the two in every run here.
- Not known: what a viewer should ask for (500 ms is the number the experiments used, from no measurement of its own); a timeout on the sound; several viewers that ask for different times; BBR; a real network; and nobody watched or listened.
