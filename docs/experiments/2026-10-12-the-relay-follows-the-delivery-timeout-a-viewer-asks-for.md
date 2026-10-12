# The relay follows the delivery timeout a viewer asks for

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a viewer whose link has less room than the video; the open decisions on what the relay gives up for a slow subscriber and on how many pieces of sound share a Group.

**Two of the rules below were changed in review, the same day**: when the timer starts, and whom `--older-groups give-up` holds for. What was first built and measured is left as it was written, marked where it no longer holds; the section "After the review" at the end has what the relay does now and what was measured with it.

## Objective

The relay's experiments held for every track of every viewer, and what suits video broke sound ([record](2026-10-12-a-page-behind-a-link-with-too-little-room-and-a-relay-that-gives-groups-up.md), the last section). Draft 18 lets a subscriber say a delivery timeout for one subscription ([record](2026-10-12-delivery-timeouts-of-a-subscription-what-draft-18-has-and-who-does-what.md)). With the relay acting on it: does a viewer that asks for a timeout on its video alone get the video's gain and keep its sound, in the cases where the sound broke?

Expected, written before the runs: with no flag and `?videoTimeout=500` the page sees what it saw with `--delivery-patience-ms 500`; with `--older-groups give-up` the video is as it was with that flag, and the sound is as with no flag at all, also with no grace and also with a piece of sound a Group at 5% loss.

## What the relay does now

**Provisional**, decided for this change on 2026-10-12; `docs/moqt-implementation-notes.md` has it among the known gaps, with what is left out.

- **It acts on SUBGROUP_DELIVERY_TIMEOUT of a SUBSCRIBE** (§8, §10.2.3): a subgroup stream of that subscription that the subscriber's transport has not acknowledged whole so long after the relay wrote its FIN is reset with DELIVERY_TIMEOUT (0x2). 0 is no timeout. The mechanism is the one `--delivery-patience-ms` had, now with a time for each stream.
- **`--delivery-patience-ms` is what a subscriber gets that says nothing, and the smaller of the two holds where both are there** (§8: "if both the publisher and the subscriber have a non-zero value, the smaller of the two is used"). With neither, nothing is given up, as before.
- (**Withdrawn in review**, below.) **`--older-groups give-up`, with its grace, is applied only to a subscription that set a SUBGROUP_DELIVERY_TIMEOUT.** A subscriber that sets none on a track gets every stream of it whole (§8: "if none of the timeouts are set to a non-zero value, all Objects ... are delivered").
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

(Written before the review. The first point holds; the second is overtaken by the section after this one.)

- **The two decisions no longer hang together.** With the rule applied by subscription, what the relay gives up of the video does nothing to the sound, at either size of a Group of sound and with or without a grace. The question of ten pieces or one can be answered by [its own record](2026-10-12-one-five-or-ten-pieces-of-sound-a-group.md).
- **A default for the relay need not be a flag at all**: with no flag, a viewer that asks for 500 ms on its video has what the flag gave, and one that asks for nothing has every Object. Which of the relay's rules a viewer's timeout switches on is the open part: today a timeout alone gets "so long after a Group's end", and "at the next Group" needs the flag as well, though it was the better of the two in every run here.
- Not known: what a viewer should ask for (500 ms is the number the experiments used, from no measurement of its own); a timeout on the sound; several viewers that ask for different times; BBR; a real network; and nobody watched or listened.

## After the review

A review of the change, against §8 of the draft, found two things, and both were changed.

- **The timer started too late.** §8: the sender "MUST start a timer of SUBGROUP_DELIVERY_TIMEOUT duration once it becomes aware that all of the objects on the subgroup have been published (either by receiving a FIN from the upstream subscription, ...)", and Table 4 has the timeout start "When the FIN for the subgroup is received". The writer started it when it had written the last Object and the FIN, so a stream towards a subscriber that takes nothing lived for as long as its writes waited, and then for the timeout. **Now the time runs from when the relay knows the Subgroup is whole** (the publisher's FIN), and a stream that is not delivered when it is over is reset whether it is written out or not ("If the timer expires before the underlying transport stream reaches "all data committed" state ..., the implementation MUST reset the stream"). `--delivery-patience-ms` is the same timer and starts there too.
- **Giving older Groups up only for those who set a timeout turned the meaning of a timeout round**: a subscriber that asked for ten minutes was cut at the next Group, and one that asked for nothing got everything. A timeout has to be the more patient the larger it is. **`--older-groups give-up` is again what it was before this change**: a flag of the relay for every subscriber and every track, off unless asked for. What a subscriber's timeout moves is the subgroup timer alone.

Also from the review: the ground for the relay's own flag against a subscriber's timeout is §8's "Publishers can, at their discretion, discontinue forwarding Objects before either of the timeouts occurs", not the sentence about the smaller of a publisher's and a subscriber's value, and a relay with `--delivery-patience-ms` leaves §8's "if none of the timeouts are set to a non-zero value, all Objects ... are delivered" for the subscribers that set none (`docs/moqt-implementation-notes.md` has the draft's part and the relay's own apart). The in-memory connection that delivers when told now ends a finished stream that the peer stopped, or whose connection is gone, as a real one does; `contract_tests!` is not run on it, because one case of the contract (a finished stream is delivered without anybody saying so) is what it holds back.

Tests added or changed, on paused time: a stream that cannot be written out is reset when its time after `finish` is over, in the writer and in the relay (a subscriber that takes nothing, 300 ms after the publisher's FIN; never without a timeout); a stream that is delivered within its time is not reset (three rows, told after 100 ms); the rows without a timeout end with the stream delivered and no writer left; an older Group is given up when a newer begins for a subscriber that set nothing (grace 0 and 100 ms). The rows that held the withdrawn rule are gone.

### What that does to the tables above

- The rows "give up, no grace" and "give up, grace 100 ms" with `?videoTimeout=500` were measured with the withdrawn rule: the sound was kept because the rule passed it over. **With the flag for every track again, those rows are as in the record of the narrow link**, where no grace breaks the sound.
- The rows with no flag stand: the timer's start moved by the time a Group's last Objects waited to be written, which on these links is within what the runs differ by (below, 500 ms asked: 438 to 446 frames at 2.5 Mbit/s, where the first runs had 394 to 444).

### A timeout on the video alone, at a relay with no flag

The question the withdrawn rule leaves: can a viewer get what "give up at the next Group, with 100 ms of grace" gave (about 520 of 600 frames at 2.5 Mbit/s, about 250 ms behind) by a timeout alone? Expected, written before the runs: yes with 100 ms. A Group of video is one stream, its FIN comes a frame before the next Group begins, so a timeout of 100 ms from the FIN is nearly a grace of 100 ms from the next Group.

As above: `/watch?videoTimeout=N`, nothing on the sound, the relay with no flag at all, buffer 150 ms, two runs each of ten pieces of sound a Group and of one (four runs a row; the two do not differ).

The narrow link (the busy picture at 2.5 Mbit/s):

| Timeout asked | Link | Frames shown of 600 | Picture came after, ms | Dropped by the page as `late` | Sound: sounded of 1,000, breaks |
|---|---|---|---|---|---|
| 100 ms | 2.5 Mbit/s | 518 to 526 | 236 to 244 | 0 | 975 to 982; 0 to 1 |
| 150 ms | 2.5 Mbit/s | 511 to 521 | 259 to 269 | 3 to 8 | 966 to 982; 1 to 2 |
| 250 ms | 2.5 Mbit/s | 487 to 504 | 310 to 326 | 27 to 30 | 972 to 981; 0 to 1 |
| 500 ms | 2.5 Mbit/s | 438 to 446 | 405 to 437 | 82 to 89 | 973 to 981; 1 to 2 |
| none (the records before) | 2.5 Mbit/s | 328, 338 | 570 and growing | 213 to 219 | 975, 981; 0, 1 |
| 100 ms | 2 Mbit/s | 407 to 412 | 336 to 347 | 0 | 970 to 982; 0 to 3 |
| 150 ms | 2 Mbit/s | 408 to 415 | 366 to 380 | 0 to 1 | 967 to 980; 0 to 3 |
| 250 ms | 2 Mbit/s | 390 to 400 | 410 to 429 | 18 to 22 | 966 to 979; 1 to 4 |
| 500 ms | 2 Mbit/s | 345 to 351 | 516 to 527 | 68 to 73 | 962 to 979; 0 to 4 |
| none (the records before) | 2 Mbit/s | 126, 129 | 730 and growing | 306 to 308 | 954, 964; 2, 3 |

5% loss, with room to spare (the clock, 640x360):

| Timeout asked | Frames shown of 600 | Picture came after, ms | Sound, 10 a Group: sounded, breaks | Sound, 1 a Group: sounded, breaks |
|---|---|---|---|---|
| 100 ms | 342 to 421 | 124 to 156 | 862, 855; 26, 15 | 918, 881; 38, 20 |
| 150 ms | 323 to 433 | 107 to 186 | 886, 891; 19, 27 | 936, 948; 12, 10 |
| 250 ms | 336 to 377 | 125 to 153 | 888, 882; 21, 18 | 915, 936; 13, 12 |
| 500 ms | 280 to 377 | 142 to 187 | 912, 922; 28, 19 | 927, 906; 16, 19 |
| none (the records before) | 225 to 303 | 173 to 346 | 877, 836; 21, 31 | 883, 926; 19, 10 |

- **As expected: a timeout of 100 ms on the video does what "give up at the next Group with 100 ms of grace" did**, at a relay with no flag: 518 to 526 frames and about 240 ms behind at 2.5 Mbit/s (517 to 524 and 233 to 252 with the flag), 407 to 412 and about 340 ms at 2 Mbit/s (385 to 419 and about 350). The page drops nothing as `late`: what it would throw away is not sent.
- **The shorter the timeout, the more frames and the nearer the picture**, at both rates: from 500 ms to 100 ms, 442 to 521 frames and 420 to 240 ms behind at 2.5 Mbit/s.
- **The sound is whole in every row of the narrow link** (962 to 982 sounded, 0 to 4 breaks), with ten pieces a Group and with one: nothing was asked for it, and nothing is given up of it.
- **Under 5% loss the video gains with any of the four** (280 to 433 frames against 225 to 303 with no timeout) and the four cannot be told apart in two runs each; the sound is as without a timeout. In some runs the viewer received fewer pieces of sound than were sent in the 20 s (909 and 934 of about 985), as noted above, with nothing giving sound up.

`mise run check` and `mise run interop` (125 of 125) passed after the changes.

### What this says now

- **The flag `--older-groups give-up` is not needed for what it was measured to be good at**: a viewer's timeout of 100 ms on its video gives the same picture, per viewer and per track, in the draft's own terms, and leaves the sound alone. Whether the flag is kept as an experiment is open.
- **The size of a Group of sound and the relay's rules are separate questions again**, for another reason than the withdrawn one: nothing is given up of a track whose subscriber asked for nothing.
- **What a viewer should ask for is the open number.** 100 ms was the best here, on a link that is only narrow, with 50 ms each way; a timeout below the round trip gives up what a packet sent again would have brought, and at 5% loss the runs do not show which of the four is best. Not known either: a relay whose default `--delivery-patience-ms` meets viewers that ask for something else, several viewers, BBR, a real network; and nobody watched or listened.
