# The meeting of a page follows a GOAWAY

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale: the meeting's page (`/meet`) attended the meeting in a loop of its own in `demo/meet.ts`, which connected again to the same relay after a GOAWAY.

## Objective

A full relay with `--when-full` sends each session of a page away with a GOAWAY after SETUP. With `keepAttending` of `@xmoq/meeting`, on `keepMoving` of `@xmoq/moqt`, which hands `connect` the GOAWAY that ended the session before, does a participant of the meeting go to the relay it was sent to?

**Loopback, one machine.**

## Expected

Written before the first run, in the head of `interop/when-full.ts` (case `moves`):

- The second viewer attends the meeting of the room `r` with a connection of its own to the relay above, as the demo's meeting page does (`keepMeeting`). The relay above prints `SESSION_SENT_AWAY` a fourth time, with the same URI; the participant says `moved` with it, and then `attending 2` at that URI.
- Written after a review, before `keepAttending` said `attending` only once the SUBSCRIBE_NAMESPACE was taken: the session sent away is then counted for nothing, and the participant says `moved` and then `attending 1` at that URI, with no `attending` before.

## Environment

macOS 14.4, arm64, Chromium of Playwright; the own relay (debug build) on 31500 and 32500 (32501 for the second case), on `rqtzyqqr` (the change that adds `keepMoving`) with this change. `mise run interop -- --base-port 31500 --only "when full"`.

## Result

**As expected, in the first run, and again after the review.** Both cases passed (12.5 s together). In `moves` (5.1 s) the participant said: connecting 1, attending 1 `https://127.0.0.1:31500`, moved `https://127.0.0.1:32500?sent-away=1`, connecting 2, attending 2 `https://127.0.0.1:32500?sent-away=1`: the session at the full relay was counted, since `attending` was said before the SUBSCRIBE_NAMESPACE was answered. With `attending` said once it was taken, and `keepMoving` waiting for the `run` of a session a GOAWAY ended (`RUN_END_WAIT_MS`), both cases passed again (11.9 s): connecting 1, moved `https://127.0.0.1:32500?sent-away=1`, connecting 2, attending 1 `https://127.0.0.1:32500?sent-away=1`. `--only meeting` passed as well (4 of 4).

With `keepAttending` handing `connect` no GOAWAY (`options.connect(goAway && undefined)`), the case failed as it should: the participant went back to `https://127.0.0.1:31500` (`attending 2 https://127.0.0.1:31500`) and was sent away again.

`packages/meeting/test/keep-attending.test.ts` has the same on sessions of the test: an attending that ends, a GOAWAY, a refusal, a refusal with GOING_AWAY before its GOAWAY, and a caller that stops. `packages/moqt/test/webtransport/keep-moving.test.ts` has a `run` that ends 20 ms after its connection closed, which the next session waits for; without the wait that row fails.

The demo's meeting page, two of them in a headless Chromium (Playwright's 156.0.8078.4, the voice of `fixtures/speech/` as the microphone), on a demo of its own (`bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787`): after 17 s each was seated, publishing and attending, and had the other's picture (256 and 257 frames) and sound (846 and 870 pieces); the screenshot of each shows its own clock and the other's, moving. No GOAWAY was sent there.

## Not looked at

- What the participant receives at the relay it was sent to: nobody else is in the meeting of `when-full.ts`.
- The demo's meeting page at a GOAWAY: it was opened (above), but no relay of that demo sends one. Whether a person hears the sound was not looked at.
