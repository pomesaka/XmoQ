# A page that waits for the broadcast at the relay

- Date: 2026-10-13
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2 and the rooms: a viewer that is there before the publisher, and one that stays when the publisher goes and comes back. The relay holds a SUBSCRIBE that carries a RENDEZVOUS_TIMEOUT for a track nobody publishes (transport §10.2.6; the record "a viewer that comes before the broadcast" of the same day has the relay's side and the terminal viewer).

## Objective

That a page can use it: come before the broadcast and begin to play when it begins, and go on when the publisher has gone and come back, on the session it has, where it now closes its session and connects again every 2 seconds.

## What was built

- `followCatalog` of `@xmoq/msf` takes `rendezvousTimeoutMs`, the RENDEZVOUS_TIMEOUT of its SUBSCRIBE for the catalog track. On that SUBSCRIBE alone: once the catalog is answered the broadcast is there. `@xmoq/moqt` had the parameter and waits for the answer that much longer (the rule of the wait for an answer); `RequestErrorCode.TIMEOUT` (0x2) is new there, as a name for what the relay answers.
- `stayOnSession` (`@xmoq/media`, `src/core`, table tests) and `stay` on `keepPlaying`: where the caller says that a failure to play only means "not yet", and where what was played has ended, the playing is asked for again on the same session after a pause (1 s, provisional), and the session is not closed. Left out, `keepPlaying` is as it was.
- `/watch?wait=1`: the SUBSCRIBE for the catalog asks for 30 s (what the relay holds at most); "not yet" is the relay's `TIMEOUT`, and `DOES_NOT_EXIST` as well, which a track that has just ended is refused with for up to 2 s, and which a relay that holds nothing answers; where the room names no broadcast the place is the demo's own. Without the flag the page is as it was.

## Expected, written before the runs

The demo with rooms (`mise run demo`'s parts on other ports); the publishing page and the watching page in headless Chromium.

| Scene | Without `?wait=1` | With it |
|---|---|---|
| The viewer comes 10 s before the publisher | It waits for the room's word and connects then: playing 1 to 3 s after the publisher began (the room's word, a connection, the catalog), 1 connection | 1 connection, made at once; the SUBSCRIBE is held; playing within 1 s of the publisher's announcement, sooner than without by the connection's handshake |
| The viewer comes 40 s before the publisher | As above | The relay answers `TIMEOUT` after 30 s once; the page asks again on the session a second later; playing as above; 1 connection, "not yet" 1 time |
| The publisher's page is closed and opened again after 10 s | The tracks end, the page closes its session and asks the room; a connection when the room names the broadcast again: 2 connections in all, playing again 1 to 3 s after the publisher is back | 1 connection in all. The tracks end; a second later the page subscribes again and is refused with `DOES_NOT_EXIST` while the track drains (up to 2 s), once or twice; then it is held; playing again within 1 s of the publisher's announcement |

What is counted: the time from the publishing page's "publishing" to the watching page's first frame, the connections the watching page made, how many times it was told "not yet".

## Environment

macOS 14.4, arm64, loopback; Chromium 156.0.8078.4 (playwright-core 1.64.0), headless; `bun demo/serve.ts` with its rooms (Miniflare) and the own relay as of the commit that holds a SUBSCRIBE for its RENDEZVOUS_TIMEOUT, no flag. `/publish?room=R` and `/watch?room=R`, with and without `&wait=1`. The time to the picture is from when the publishing page was asked for to the first frame the watching page showed, read every 50 ms.

## Result

| Scene | | Picture after the publisher's page was asked for, ms | Connections of the viewer | Told "not yet" |
|---|---|---|---|---|
| The viewer comes 10 s before | as it is | 2,492, 2,474, 2,481 | 1 | |
| | `wait=1` | 493, 499, 496 | 1 | 0 |
| The viewer comes 40 s before | as it is | 2,500, 2,492 | 1 | |
| | `wait=1` | 496, 497 | 1 | 1, 1 |
| The publisher's page is closed and opened again 10 s later | as it is | 2,425, 2,451, 2,467 | 2 | |
| | `wait=1` | 438, 491, 493 | 1 | 0 |

Against what was expected:

- **The viewer that waits at the relay has its picture half a second after the publisher's page was asked for, in every run, where the page as it is has it after 2.5 s.** Sooner than expected by more than a handshake: as it is, the page learns of the broadcast by asking again every 2 seconds whether the room names one, and that wait is most of the 2.5 s.
- **40 s before**: the relay answered `TIMEOUT` once, after its 30 s, the page asked again on its session a second later, and was held again. One connection. As expected.
- **The publisher that comes back**: one connection in place of two, and the picture 2 s sooner. **Not as expected: the page was never told "not yet" there.** The publishing page was closed and said no PUBLISH_DONE, so the relay ended the track with the session and had nothing to wait out; the SUBSCRIBE a second later was held at once. The 2 s in which a track that ended with PUBLISH_DONE is refused with `DOES_NOT_EXIST` were not met in these runs. A change to the relay that holds a SUBSCRIBE through those 2 s as well is under review as this is written; with it the page's reading of `DOES_NOT_EXIST` as "not yet" is for other relays alone.
- **No SUBSCRIBE was refused as a second one for its track** (`DUPLICATE_SUBSCRIPTION`): the page lets go of what it subscribed to when what it played has ended, and asks again a second later, by which time the relay's PUBLISH_DONE has come. A page that asked again before the relay's PUBLISH_DONE would be refused so, for up to 2 s; this one does not.
- Without the flag the page is as it was: its rows are what the page did before.

**In the interop run**: an own page that reads the catalog of a broadcast that has not begun (`followCatalog` with `rendezvousTimeoutMs`) is held by the own relay for 2 s and more, and reads the catalog once another page publishes it (`catalog.ts --catalog reader-first`).

`mise run check` and `mise run interop` passed (the numbers are in the pull request).

## What this says, and what it does not

- For a viewer that knows where the broadcast will be, the relay is the better one to say when: the picture is there 2 s sooner, on one connection, also after the publisher has gone and come back.
- **The room is still what says where.** `?wait=1` takes the demo's own place where the room names none; a publisher that comes back at another relay or under another namespace is not found by a held SUBSCRIBE.
- Not run: a publisher that ends its tracks with PUBLISH_DONE and comes back within 2 s; a relay with tokens, and a token that runs out while a SUBSCRIBE is held; a wait of minutes (one SUBSCRIBE every 31 s); many viewers waiting at once; a relay that is not ours (`moqtail`'s refuses at once, so the page would ask every second).
- **The time of 2.5 s without the flag is the demo's, not the room's**: a page that took the room's word as it comes, and not every 2 s, would be nearer. That was not built or measured.
