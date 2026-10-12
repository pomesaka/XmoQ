# A session that announces a namespace again and again

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer (the relay's limits)

## Objective

The known gaps have had, as the fourth of those that can do the most harm, that a session can announce a namespace as often as it likes: each PUBLISH_NAMESPACE has the own relay look at its tracks and send that session a SUBSCRIBE for each track of the namespace that is subscribed to or held. Nobody had run it. This run measures what such a session costs the relay and the sessions beside it.

## What the draft says

§13.1 (Subscription Amplification), whole: "A malicious subscriber could attempt to overwhelm a publisher or relay by requesting subscriptions to many tracks simultaneously. Relays SHOULD implement rate limiting on subscription requests and MAY reject excessive subscriptions with REQUEST_ERROR using the EXCESSIVE_LOAD error code. Publishers SHOULD monitor the number of active subscriptions and enforce limits to prevent resource exhaustion from a single subscriber or session."

**The SHOULD is about subscription requests of a subscriber, and does not name PUBLISH_NAMESPACE.** It says what to limit (the rate of subscription requests) and what a relay may answer beyond it (`EXCESSIVE_LOAD`); it gives no number, and does not say to close the session. The known gap cited §13.1 for announcements, which is this project's extension of it: an announcement makes the relay send SUBSCRIBEs, so it is a way to the same load.

## What was built

`xmoq-quiet-peer --connect <url> --cert-sha256 <hex> --stay-ms <n> --announce-per-second <n>`: it sends its SETUP, and then that many times a second a PUBLISH_NAMESPACE for `(xmoq)`, which it cancels (reset, CANCELLED) as soon as it is answered (§9.5: "When a publisher wants to stop new subscriptions for a published namespace, it cancels the request"). The SUBSCRIBEs the relay sends it are refused with `DOES_NOT_EXIST`. At the end it prints what it counted.

The script that ran the pieces and took the numbers was a throwaway and is not in the repository. It started the relay, a publisher and a subscriber of `xmoq/a` (own Rust clients), and a subscriber of `xmoq/b` held with a RENDEZVOUS_TIMEOUT of 30 s, so that each announcement had two tracks to be asked for. It had the publisher send an Object every 100 ms and took the time from the line written to the publisher's stdin to the `OBJECT` line of the subscriber, and the relay's CPU time and resident memory from `ps`.

## Environment

Apple M3 Pro (11 cores), macOS, loopback. Release builds of `xmoq-relay`, `xmoq-moqt-client` and `xmoq-quiet-peer`. The relay's log went to a file. One run for each line.

## Observed results

| Announcements a second | Answered REQUEST_OK | SUBSCRIBEs the relay sent the session | Relay's CPU | Delay of the Objects beside it (p50 / p95 / max) | Relay's memory, before and at the end | Lines of the relay's log |
|---|---|---|---|---|---|---|
| none (5 s before each run) | | | 0.04 to 0.05 s in 6.6 s | 0.9 to 1.3 / 1.6 to 1.9 / 1.7 to 2.4 ms | 10.0 MB | |
| 10, for 10 s | 100 of 100 | 200 | 0.16 s in 9.5 s | 0.8 / 1.4 / 1.5 ms | 10.1, 11.0 MB | 1,292 |
| 100, for 10 s | 1,000 of 1,000 | 1,998 | 0.63 s in 9.5 s | 0.6 / 1.2 / 2.1 ms | 10.0, 12.0 MB | 9,442 |
| 1,000, for 10 s | 10,000 of 10,000 | 15,229 | 4.41 s in 9.5 s | 0.6 / 1.2 / 1.3 ms | 10.0, 17.6 MB | 76,910 |
| 1,000, for 30 s | 30,000 of 30,000 | 38,325 | 9.37 s in 29.5 s | 0.4 / 1.0 / 8.3 ms | 10.0, 30.9 MB (25.0 MB 5 s after the session went) | 209,256 |
| 1,000, for 10 s, and again from a new session | 10,000 and 10,000 | 15,561 and 16,412 | 3.75 and 3.31 s in 9.5 s | 0.6 / 1.0 / 1.4 and 0.6 / 0.9 / 1.0 ms | 10.0, 17.9, and 19.3 MB after the second | not counted |

- **The sessions beside it were not delayed.** Every Object arrived, and the 95th percentile of its delay stayed between 0.9 and 1.4 ms at every rate, as without the announcements. The one 8.3 ms is a single Object in 276.
- **The relay did what the gap says, at the rate it was asked**: it answered every announcement, and sent the session one SUBSCRIBE for each of the two tracks (fewer at 1,000 a second, where an announcement is cancelled before the relay has asked for both). It refused none, and closed nothing.
- **The cost is CPU on the relay: about a third to a half of one core at 1,000 a second**, and in proportion below that. The relay's log grows by about 7 lines for each announcement.
- **Memory grew with the number of announcements of one session, by about 0.7 kB each**, and a second session after the first added 1.4 MB where the first had added 7.9 MB: what is kept goes with the session and is used again. Which state it is was not looked for.
- The subscriber that was held ended with `TIMEOUT` after its 30 s in the two runs that lasted that long, as it is to.

## With many tracks held under the namespace

The run above had two tracks to be asked for. Here `xmoq-quiet-peer --hold-tracks <n>` has SUBSCRIBEs for n tracks of `(xmoq)` held (16 for a session, with a RENDEZVOUS_TIMEOUT of 30 s), beside the publisher and the subscriber of `xmoq/a`, and the session that announces is run for 10 s at 10 and at 100 a second.

**Expected, written before the run**: the relay sends the announcing session one SUBSCRIBE for each held track at each announcement, up to what it lets itself have open towards one session, so the SUBSCRIBEs sent grow with the tracks and stop growing at that limit. The relay's CPU grows with the tracks. With 100 tracks nothing is seen beside it. With 1,000 tracks and 100 announcements a second the relay uses a whole core, and the Objects beside it are late by more than 10 ms at the 95th percentile, since the relay's routing is one task.

| Held tracks | Announcements a second | SUBSCRIBEs the relay sent the session | Relay's CPU in 9.5 s | Delay of the Objects beside it (p50 / p95 / max) | Relay's memory, before and at the end |
|---|---|---|---|---|---|
| 2 | 10 | 300 | 0.17 s | 0.7 / 1.3 / 1.5 ms | 10.1, 11.1 MB |
| 2 | 100 | 3,000 | 0.82 s | 0.6 / 1.3 / 1.8 ms | 10.0, 12.1 MB |
| 100 | 10 | 6,400 | 1.07 s | 0.7 / 1.2 / 1.3 ms | 14.2, 20.0 MB |
| 100 | 100 | 64,000 | 8.26 s | 0.6 / 1.2 / 1.7 ms | 14.2, 21.6 MB |
| 1,000 | 10 | 6,400 | 1.17 s | 0.6 / 1.3 / 1.5 ms | 45.1, 51.3 MB |
| 1,000 | 100 | 64,000 | 8.49 s | 0.6 / 1.7 / 2.0 ms | 45.1, 51.6 MB |

Every announcement was answered with REQUEST_OK, and every Object beside it arrived (79 of 79 in each run). Without the announcements the delay was 0.5 to 1.2 / 0.8 to 1.9 / 0.9 to 1.9 ms in the 3 s before each run. With 2 held tracks the relay sends 3 SUBSCRIBEs for an announcement: the track `xmoq/a` has a subscriber, and is asked for as well.

- **One announcement makes the relay send 64 SUBSCRIBEs at most**, with 100 held tracks as with 1,000: the relay has 64 requests of its own open towards one session and no more. As expected.
- **The relay's CPU is close to a whole core at 100 announcements a second once there are 64 tracks to ask for** (8.3 to 8.5 s in 9.5 s, 0.87 to 0.89 of a core), and a tenth of it at 10 a second. It goes with the SUBSCRIBEs sent, about 0.13 ms each, and not with the number of tracks the relay looks through: 1,000 tracks cost what 100 do. Expected for the whole core; the part of the expectation that the tracks themselves would be felt was wrong.
- **The Objects beside it were not late, against the expectation**: the 95th percentile stayed at 1.2 to 1.7 ms with the relay at 0.87 of a core. The relay's process has more than one thread and the machine 11 cores; the time went where it did not hold up the task that routes.
- The memory before each run is what the held SUBSCRIBEs themselves take: about 35 kB for a held track with its share of a session.



## With the limit

After the second run the relay was given a limit: a session may send five PUBLISH_NAMESPACE requests at once, and gets one back each second; a request beyond that is refused with `EXCESSIVE_LOAD` (§10.6.2: "The responder is overloaded and cannot process the request at this time"). The same script, the same machine, one run for each line.

| Held tracks | Announcements a second | Accepted, refused | SUBSCRIBEs the relay sent the session | Relay's CPU in 9.5 s (before the limit) | Delay of the Objects beside it (p50 / p95 / max) |
|---|---|---|---|---|---|
| 100 | 100 | 14, 986 | 896 | 0.20 s (8.26 s) | 0.3 / 0.4 / 0.4 ms |
| 1,000 | 100 | 14, 986 | 896 | 0.30 s (8.49 s) | 0.6 / 0.7 / 0.9 ms |
| 1,000 | 1,000 | 14, 9,986 | 708 | 1.32 s (not run) | 0.5 / 0.7 / 0.8 ms |

- 14 announcements are accepted in 10 s (five, and one for each of nine seconds), each with 64 SUBSCRIBEs: 896. The relay's CPU for the session that took 0.87 of a core is 0.02 to 0.03 of one.
- What is left at 1,000 a second (0.14 of a core) is the refusing itself: 9,986 requests read and answered.
- The interop run `relay-many-announcements` has the same in the matrix, at 20 a second for 3 s beside a publisher and a subscriber: 7 accepted, 53 refused with `0x9`, no session closed, and the subscriber got its Object afterwards.

## Not run

- More than 100 announcements a second with 64 tracks or more to ask for, where the relay would need more than one core.
- More than one session that announces. With the limit each session has its own five and one a second, so 256 sessions, which is as many as the relay takes, could have it send 16,384 SUBSCRIBEs a second.
- A relay on a machine with one core, or a link that is not loopback. With 11 cores idle, half a core of the relay takes nothing from the others.
- The browser as the session that announces.

## Next action

No limit was made after the first run, since nothing beside the session was harmed. After the second, in which one session took most of a core, the limit above was made; its numbers are provisional (`docs/open-questions.md`). A limit for the sessions together is not made.
