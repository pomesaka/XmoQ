# The chat of a page follows a GOAWAY

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale: the known gap of the [record of a full relay](2026-10-25-a-full-relay-sends-the-next-session-away.md), that `@xmoq/chat` did not follow GOAWAY.

## Objective

A page has two sessions, one for its media and one for the chat. A full relay with `--when-full` sends each one away with a GOAWAY after SETUP. The media went on at the relay named. The chat's session was refused there (GOING_AWAY) and connected again to the same full relay. With `keepInTheRoom` of `@xmoq/chat`, which hands `connect` the GOAWAY that ended the session before, does the chat join the room at the relay it was sent to?

**Loopback, one machine.**

## Expected

Written before the first run, in the head of `interop/when-full.ts` (case `moves`):

- The second viewer keeps a participant of the chat in the room `r` with a connection of its own to the relay above (`keepChat`, the URI of the last GOAWAY and the run's certificate hash). The relay above prints `SESSION_SENT_AWAY` a second time, with the same URI. The chat says `moved` with it, and then `joined 1` at that URI, its first session joined.

## Environment

macOS 14.4, arm64, Chromium of Playwright; the own relay (debug build) on 31500 and 32500 (32501 for the second case), on `main` at `rottxsyp` with this change. `mise run interop -- --only "when full:" --base-port 31500`.

## Result

**As expected, in the first run.** Both cases passed (12.3 s together):

| Case | Took | What the second viewer's chat said |
|---|---|---|
| moves | 4.8 s | connecting 1, moved `https://127.0.0.1:32500?sent-away=1`, connecting 2, joined 1 `https://127.0.0.1:32500?sent-away=1` |
| both-full | 7.5 s | (no chat in this case; the video as before) |

The whole matrix then ended 241 of 241 as expected (541.2 s, `--base-port 31500`), both cases among them.

With `keepInTheRoom` handing `connect` no GOAWAY (`options.connect(movedBy && undefined)`), the case `moves` failed as it should: `timed out waiting for the second viewer's chat to join`.

`packages/chat/test/keep-in-the-room.test.ts` has the same on sessions of the test: a GOAWAY before the PUBLISH_NAMESPACE is answered and one after it joined both move the participant, and the next `connect` is handed that GOAWAY, also after a session at the new place ended.

## Not looked at

- The demo's pages: `demo/room-chat.ts` keeps the chat with `keepInTheRoom` now, and `demo/shared.ts` connects to the URI of a GOAWAY; not run in a browser by hand.
- The chat at the relay it was sent to, with a gatherer: no gatherer runs in `when-full.ts`, so the chat joined (the namespace was announced) and no line went through it.
- The captions of the watching page (`demo/captions.ts`), a session of their own, do not follow a GOAWAY.
- The pause: the next session after one a GOAWAY ended is made at once, once in a row, as for `@xmoq/media`; the test gives an `againMs` of 1 and does not tell the two apart.
