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

## Not run

- The 30 seconds themselves, and the limits of 16 and 1,024, against real connections: each is in the relay's tests, with connections in memory and a clock moved by hand. The 1,024 are there by both ways a held SUBSCRIBE comes to its track: nobody asked, and a session that announced asked and refusing. (The first version of the change had no test of the 1,024, and the second way went round the limit.)
- A held SUBSCRIBE through the `moqtail` relay with a publisher that comes later: that relay refuses at once, so there is nothing to wait for.
- The parameter passed on upstream: the relay sends its own SUBSCRIBE without it.
- The demo with the parameter in place of the room's word.

## Next action

Whether the demo's pages use it is to be decided.
