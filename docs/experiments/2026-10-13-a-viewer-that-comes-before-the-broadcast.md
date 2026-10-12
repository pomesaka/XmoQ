# A viewer that comes before the broadcast

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer (the relay's routing); Phase 3, the terminal

## Objective

A SUBSCRIBE for a track nobody publishes was refused at once by the own relay, so a viewer had to come after the publisher, and the demo has the room tell its pages when a broadcast is there. Draft 18 has a parameter for a viewer that comes first, RENDEZVOUS_TIMEOUT. Whether the relay can hold such a SUBSCRIBE, and what the own clients and the peers do with the parameter.

## What the draft says

§10.2.6 has it, and §9.5 refers to it once without its name: a relay that gets a PUBLISH "if the relay is holding a downstream SUBSCRIBE awaiting a publisher for this Track (see Section 10.2.6), it MUST proceed with the SUBSCRIBE and MUST NOT also forward the PUBLISH to that subscriber". The sections on SUBSCRIBE and on PUBLISH_NAMESPACE do not speak of it. (The first version of this record said that no section on relays does; a search for the name does not find §9.5.)

- Who sends it: the subscriber, in a SUBSCRIBE. "The RENDEZVOUS_TIMEOUT parameter (Parameter Type 0x04) MAY appear in a SUBSCRIBE message." And: "It is the duration in milliseconds the subscriber is willing to wait for a publisher to become available."
- When it applies: "This applies when a relay receives a SUBSCRIBE for a Track that has no current publisher."
- What the relay does: "If the RENDEZVOUS_TIMEOUT is present, the relay SHOULD hold the subscription and wait for a publisher to appear, up to the specified duration. The relay does not send SUBSCRIBE_OK until a publisher becomes available. If a publisher becomes available within this time, the relay proceeds with the subscription normally. If the timeout expires without a publisher, the relay SHOULD respond with REQUEST_ERROR with error code TIMEOUT."
- Who sets the limit: the relay. "The relay MAY use a shorter timeout than requested by the subscriber. For example, a relay might limit the maximum rendezvous timeout to protect its resources."
- The default: "A value of 0 indicates the subscriber does not want to wait and expects an immediate response. The relay MUST immediately return REQUEST_ERROR with error code DOES_NOT_EXIST if no publisher is available", and "If RENDEZVOUS_TIMEOUT is absent, the default is 0."
- **What it does not say**: how the value is encoded (a varint in both own implementations and in the two peers' libraries); whether a relay that uses a shorter time tells the subscriber; what "a publisher" is where a session has announced the namespace and will not serve the track; what a publisher that is not a relay does with the parameter; whether a relay passes it on in the SUBSCRIBE it sends upstream.

## What the peers do

- **`moqtail` relay 0.15.0, run**: it does not hold. An own-rust SUBSCRIBE with a RENDEZVOUS_TIMEOUT of 1,500 ms for a track nobody publishes was answered at once with `REQUEST_ERROR 0x10` and the reason "Unknown track namespace".
- `moqtail` library 0.15.0, read (`src/model/parameter/message_parameter.rs`): the parameter is encoded as a varint and allowed in SUBSCRIBE alone. Its clients were not looked at for whether they send it.
- `moqtransport` v0.14.0 under moqlivemock, read (`remote_track.go`, `subscribe.go`): a subscriber can send it (`WithRendezvousTimeout`), a publisher can read it from a SUBSCRIBE (`RendezvousTimeout()`). Whether `mlmsub` sends it was not looked at; `mlmpub` is no relay.
- The interop runner of the working group lists a test `rendezvous-timeout` and one `subscribe-before-announce` (read on 2026-10-09, `spikes/msf-loc/implementations-sources.md`); neither was run here.

## What was built

- The own relay holds a SUBSCRIBE with RENDEZVOUS_TIMEOUT for a track nobody publishes: for as long as it asks, 30 seconds at most. A PUBLISH for the track, or a session that announces the namespace and accepts the relay's SUBSCRIBE, has it answered with SUBSCRIBE_OK. When the time is over and nobody has the track: REQUEST_ERROR with TIMEOUT (0x2).
- A session has 16 SUBSCRIBEs held at most, and the relay 1,024 tracks without a source that one is held for; a SUBSCRIBE beyond either is not held.
- The clients: `@xmoq/moqt` had the parameter (`parameters.rendezvousTimeout`); the Rust client and the terminal viewer have a flag, `--rendezvous-timeout-ms`.
- The demo is as it was: its pages connect when the room has a broadcast.

## Environment

One machine, macOS, loopback; Chromium 156.0.8078.4 of Playwright, headless. The own relay.

## Commands

```sh
cd interop
bun rendezvous.ts --case publisher-comes|nobody-comes|not-asked --subscriber own-browser|own-rust
bun rendezvous.ts --case viewer-before-the-broadcast
```

## Observed results

| Case | Subscriber | Outcome |
|---|---|---|
| The publisher comes 2 s after the SUBSCRIBE (20 s asked) | own-browser, own-rust | `HOLD_REQUEST session=0 stream=1 SUBSCRIBE`, nothing sent for 2 s, then SUBSCRIBE_OK when own-rust sent its PUBLISH; the subscriber got the Object |
| Nobody comes (1.5 s asked) | own-browser, own-rust | REQUEST_ERROR 0x2 (TIMEOUT), 1,709 and 2,020 ms after the subscriber began; no session closed |
| The parameter is left out | own-browser, own-rust | REQUEST_ERROR 0x10 (DOES_NOT_EXIST) at once, nothing held |
| The publisher ends the track with PUBLISH_DONE (Stream Count unknown, so the relay waits 2 s for late streams), the subscriber asks 11 and 168 ms later (20 s asked), and the publisher sends PUBLISH anew 4 s after the PUBLISH_DONE | own-rust, own-browser | `DRAIN_TIMER session=0 track_alias=1`, then `HOLD_REQUEST session=1 stream=1 SUBSCRIBE`; nothing sent for 4 s, through the wait and after the track was over; SUBSCRIBE_OK at the new PUBLISH, and the subscriber got the Object. Added after the first version, which refused such a SUBSCRIBE with DOES_NOT_EXIST at once (seen in the relay's tests, not in a run) |
| The terminal viewer comes 2 s before a page's broadcast (20 s asked) | xmoq-watch | held on the SUBSCRIBE for the catalog; when the page published, it found both tracks and played: 354 frames drawn, 587 packets of sound |

**The first run of the first case failed, and showed what the tests had not**: the own-rust publisher announces its namespace before it sends PUBLISH. The relay asked it for the held track at once, it refused (`NOT_SUPPORTED`, as that mode does), and the relay refused the held subscriber with it (`0x10`), a moment before the PUBLISH came. A held SUBSCRIBE now waits on when a session that was asked will not serve the track, until its own time is over: a session that announced and refuses is no publisher of the track.

## Whether the demo could use it in place of the room's word

Nothing was built or run for this section: it is read from the code as it is (`demo/watch.ts`, `demo/shared.ts`, `packages/media/src/core/keep-session.ts`, `packages/msf/src/moqt/follow.ts`, the players of `@xmoq/media`).

**What the room tells a watching page is two things, and RENDEZVOUS_TIMEOUT can stand in for one.** The room says *where* the broadcast is (the relay, the hash of its certificate, the namespace) and gives a token for it; and by saying it only once there is a broadcast, it says *when*. The parameter is about when alone: a page has to be connected to the right relay, and name the right namespace, before it can be held. Where the place is known beforehand, the wait for "when" can be the relay's.

| What a page would do | Today | With the parameter |
|---|---|---|
| Wait for a broadcast to begin, the place known | It waits for the room's word and connects then. Without a room: SUBSCRIBE, `DOES_NOT_EXIST`, close the session, 2 seconds, connect and ask again (`keepPlaying`) | One session; the SUBSCRIBE for the catalog is held up to 30 seconds, and asked again when it is refused with `TIMEOUT` |
| Go on when the publisher comes back after it had gone | The tracks end (PUBLISH_DONE), the page closes its session and asks again every 2 seconds, with a new connection each time | The same session; the SUBSCRIBE that follows the end is held until the publisher is back |
| Begin to play once the publisher is there | Up to 2 seconds and a handshake after it came, or at once with the room's word | At once: the relay answers the held SUBSCRIBE when the PUBLISH or the announcement comes |

**Is 30 seconds, and asking again when it is over, enough?** For a wait of minutes it is: the page asks again every 30 seconds on a session it keeps, where it now makes a connection every 2. A publisher that comes in the moment between a `TIMEOUT` and the next SUBSCRIBE is found by that SUBSCRIBE, which is then not held at all. The session is not closed as one that holds nothing while a SUBSCRIBE is held, and has 30 seconds after a refusal to ask again. What 30 seconds do not give is a wait without any traffic: the page has to ask again, and a relay with many waiting viewers gets one SUBSCRIBE from each every 30 seconds.

**What is missing for the demo to do it:**

- **A way to send it.** `followCatalog` of `@xmoq/msf` sends its SUBSCRIBE with no parameters and has no option for any; `playVideoTrack` and `playAudioTrack` put only the delivery timeouts on theirs. Each needs to pass the parameter on. The catalog's SUBSCRIBE is the one that matters: once it is answered, the broadcast is there, and the tracks are subscribed to without a wait. `followCatalog` sends its Joining FETCH after the SUBSCRIBE was answered, so the FETCH is not held with it.
- The demo's own wait for a catalog (5 seconds, `CATALOG_TIMEOUT_MS`) needs no change: it begins when `followCatalog` has returned, which is when the SUBSCRIBE was answered, as the terminal viewer's does. The wait for that answer is `@xmoq/moqt`'s, and is longer by the time that is asked for.
- **`keepPlaying` closes the session when playing fails**, and connects again after 2 seconds. A `TIMEOUT` that only means "not yet" would then cost a connection every 32 seconds, which is still far fewer than today; asking again on the same session needs a loop inside the playing, or a way for `keepPlaying` to tell "not yet" from a failure.
- **A track that is still draining refuses a SUBSCRIBE at once, held or not** (the row "A SUBSCRIBE for a track whose sources have all said PUBLISH_DONE"; up to 2 seconds after the publisher's PUBLISH_DONE). A viewer that subscribes again right after the end gets `DOES_NOT_EXIST` once before it can be held. The relay would have to hold such a SUBSCRIBE too, or the page ask twice.
- **The token.** With `--auth` the room gives a token when the page asks, for a namespace. A page that waits in a held SUBSCRIBE has sent its token already; whether a token can run out while the SUBSCRIBE is held, and what the relay does then, was not looked at.
- **A publisher that comes back on another relay, or under another namespace**, is not found by a held SUBSCRIBE at all. Only the room, or something else outside the relay, can say that.

**Which is the better way for a viewer that waits after the publisher has gone?** Where the place stays the same, the held SUBSCRIBE: it is one session in place of a connection every 2 seconds for each waiting viewer, the picture begins as soon as the publisher is back, and the relay, which is the one that knows when a publisher appears, is the one that says so. `keepPlaying` stays what it is for: a session that ended, a relay that went and came back. The two fit together: the room (or a fixed address) for the place and the token, the parameter for the moment, and connecting again for a session that is lost. The demo as it is needs neither more nor less than the room for what it shows; the parameter would matter for a viewer without a room, which is what the terminal viewer was before this, and for many viewers waiting at once.

## With everything of the night together

The relay as of the commit that gives its refusals a Retry Interval: it holds a SUBSCRIBE under RENDEZVOUS_TIMEOUT, also while a track drains, limits how often a session announces, and says with a refusal when to ask again. The watching page asks again after the time a refusal names (`@xmoq/media`). No code was changed for these runs; what does not work is reported and not mended here.

**Expected, written before the runs:**

| Scene | Expected |
|---|---|
| 1a. `/watch?wait=1` comes 10 s before the publishing page | As in the record of the page that waits: the picture about 0.5 s after the publishing page was asked for, 1 connection, told "not yet" 0 times |
| 1b. It comes 40 s before | The relay's `TIMEOUT` once, at 30 s, with a Retry Interval of 1; the page asks again 100 ms later, where it waited a second before; the picture about 0.5 s after the publishing page was asked for; 1 connection; "not yet" 1 time; no `DOES_NOT_EXIST` |
| 2. A publisher ends its track with PUBLISH_DONE after Groups 0 to 2, a subscriber asks with a RENDEZVOUS_TIMEOUT while the relay waits for late streams, and another publisher, which numbers its Groups from 0 again, sends PUBLISH within the 2 s | The subscriber is held and gets SUBSCRIBE_OK at the new PUBLISH, with the Largest Object of the first publisher (Group 2). It gets nothing of the new publisher's Groups 0, 1 and 2, and gets Group 3. With a publisher that numbers its Groups from the time, as the demo's page does, the new Groups are larger and all arrive (read in the code, `demo/publish.ts`; not run) |
| 3. The relay with a key; a subscriber whose token runs out 5 s later asks with a RENDEZVOUS_TIMEOUT of 20 s, and a publisher comes after 8 s | The SUBSCRIBE is held, and accepted when the publisher comes although the token has run out: the relay looks at a token when the request comes, and not again |

**Result.** macOS, arm64, loopback, debug builds. Scenes 1a and 1b: `bun demo/serve.ts` on ports of its own with its rooms, headless Chromium, `/watch?room=R&wait=1` and `/publish?room=R`; the time to the picture is from when the publishing page was asked for to the first frame the watching page counts, read every 50 ms. Scenes 2 and 3: the own relay and the own Rust clients, started by a throwaway script that is not in the repository.

| Scene | Runs | Picture after the publisher's page was asked for, ms | Connections of the viewer | Told "not yet" |
|---|---|---|---|---|
| 1a. 10 s before | 3 | 478, 487, 526 | 1 | 0 |
| 1b. 40 s before | 2 | 428, 479 | 1 | 1, 1 |

- **1a and 1b: as expected**, and as in the record of the page that waits (493 to 499 ms there). In 1b the relay sent the page `TIMEOUT` once in each run and `DOES_NOT_EXIST` never. **How long the page waited before it asked again was not measured**: the relay's log has no times, and the page reports no time for it. That the pause is 100 ms after a first `TIMEOUT` is in the tests of `@xmoq/media`.
- **2: as expected, and it is a way to lose the beginning of a broadcast.** The first publisher sent Groups 0, 1 and 2 and PUBLISH_DONE; the subscriber's SUBSCRIBE came 26 ms later and was held (`DRAIN_TIMER`, then `HOLD_REQUEST`); the second publisher, another process that numbers its Groups from 0, sent PUBLISH 0.7 s after the PUBLISH_DONE, and the subscriber got SUBSCRIBE_OK at once. Of the six Groups the second publisher sent (0 to 5), **the subscriber got 3, 4 and 5, and nothing of 0, 1 and 2**, which were not larger than what the first publisher had sent. With the second publisher numbering from 1,000 (`--first-group 1000`) it got all six. What a viewer sees with a publisher that numbers from 0 again: no picture until the new publisher has sent as many Groups as the old one had, which for a broadcast that ran an hour is an hour. The demo's publishing page numbers its Groups from the time of day in milliseconds (`demo/publish.ts`), so it is larger each time; that was read and not run. **Not run with the watching page**: nothing that is there publishes a broadcast the page can play and ends it with PUBLISH_DONE. The publishing page of the demo ends with its session, the Rust client publishes one track of bytes, and the interop page's broadcast has nothing to end it with.
- **3: as expected.** The relay with a key (`--auth-key-hex`); a subscriber with a token that ran out 5 s later asked with a RENDEZVOUS_TIMEOUT of 20 s and was held; the publisher came after 8 s, 3 s after the token had run out, and the subscriber got SUBSCRIBE_OK and the Object. A subscriber that came with a token that had run out was closed in its SETUP (`CLOSE_SESSION ... code=0x18 reason="authorization"`). So a token bounds when a viewer may begin to wait, and not for how long: up to the 30 s the relay holds a SUBSCRIBE, and after that for as long as the subscription lasts, which the relay's code already has as a FIXME. The demo's rooms give tokens for 600 s (`spikes/durable-objects-room/worker.js`), which nothing sets from outside, so this was not run with the pages; the tokens here were made by the script with the relay's key.



## Afterwards: what the runs above left open

- **The Groups of a publisher that comes back.** The relay stays as it is; the publisher begins above what it used before (transport §2.4, in the implementation notes). The own Rust client has `--first-group now` for that. Interop run `another-publisher-comes`: the first own-rust process sends Groups 0 to 2 and PUBLISH_DONE, the subscriber is held, another own-rust process with `--first-group now` sends PUBLISH 42 ms (own-rust subscriber) and 215 ms (own page) after the PUBLISH_DONE, and the subscriber gets SUBSCRIBE_OK and that publisher's first Object.
- **The line `HOLD_REQUEST` for a session that was closed.** Nothing was held: the relay had dropped the session when it closed it, and takes nothing of a request that is read afterwards. The line was printed for every SUBSCRIBE whose step neither answered it nor closed its session, and such a step is one of them. Reproduced in the relay's tests (a request of the wrong parity, and a SUBSCRIBE in order behind it: `CLOSE_SESSION`, then `HOLD_REQUEST`, with 0 pending tracks and 0 timers), and the line is now printed for a session that is there only.
- **A page that ends its broadcast with PUBLISH_DONE.** The senders of `@xmoq/media` and `@xmoq/msf` had `end()`; the interop page has `endBroadcast()` for it now, and `keepBroadcastPlayed`, which plays as the demo's `/watch?wait=1` does. Interop run `page-publisher-returns`, once by itself: the viewer drew 30 frames, the publishing page ended its three tracks and stayed connected, the viewer was told that what it played had ended, asked again on its session a second later and was held; another page published the broadcast 3 s after the end, and the viewer drew 32 frames more. One session, never told "not yet", and no `DOES_NOT_EXIST` from the relay between the end and the new broadcast. (The first form of the run looked for no `DOES_NOT_EXIST` at all and failed once in the whole matrix: while the first broadcast played, the relay had passed the publishing page's `NOT_SUPPORTED` for a FETCH of its video track on to the viewer's Joining FETCH as `DOES_NOT_EXIST`, which is what it does and is not about waiting.) **The relay did not wait for late streams there**: a page that ends its tracks closes its streams first and counts them, so the tracks were over at once, and the SUBSCRIBE was held as one for a track nobody publishes. A page's viewer held while a track drains is still not run; with the Rust clients it is.

## Not run

- The 30 seconds themselves, and the limits of 16 and 1,024, against real connections: each is in the relay's tests, with connections in memory and a clock moved by hand. The 1,024 are there by both ways a held SUBSCRIBE comes to its track: nobody asked, and a session that announced asked and refusing. (The first version of the change had no test of the 1,024, and the second way went round the limit.)
- A held SUBSCRIBE through the `moqtail` relay with a publisher that comes later: that relay refuses at once, so there is nothing to wait for.
- The parameter passed on upstream: the relay sends its own SUBSCRIBE without it.
- The demo with the parameter in place of the room's word.

## Next action

Whether the demo's pages use it is to be decided.
