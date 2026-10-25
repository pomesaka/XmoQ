# The captions of a page follow a GOAWAY

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, relay scale: the captions of the watching page (`demo/captions.ts`), a session of their own beside the media's and the chat's, did not follow GOAWAY.

## Objective

A full relay with `--when-full` sends each session of a page away with a GOAWAY after SETUP. The captions connected again to the same full relay. With `keepFollowingTranscript` of `@xmoq/transcript`, which hands `connect` the GOAWAY that ended the session before, does the reader of the transcript go to the relay it was sent to?

**Loopback, one machine.**

## Expected

Written before the first run, in the head of `interop/when-full.ts` (case `moves`):

- The second viewer follows the transcript of the bars with a connection of its own to the relay above, as the demo's captions do (`keepTranscript`). The relay above prints `SESSION_SENT_AWAY` once more, with the same URI; the reader says `moved` with it, and then `reading 2` at that URI.

## Environment

macOS 14.4, arm64, Chromium of Playwright; the own relay (debug build) on 31500 and 32500 (32501 for the second case), on `main` at `ruxkxssx` with this change. `mise run interop -- --only "when full:" --base-port 31500`.

## Result

**As expected, in the first run.** Both cases passed (14.9 s together). In `moves` (6.8 s) the reader said: connecting 1, reading 1 `https://127.0.0.1:31500`, moved `https://127.0.0.1:32500?sent-away=1`, connecting 2, reading 2 `https://127.0.0.1:32500?sent-away=1`. The reader is begun on a session before its GOAWAY is looked at, so the session at the full relay is counted as `reading 1`.

The whole matrix then ended 241 of 241 as expected (543.6 s, `--base-port 31500`), both cases among them, before this change was put on the chat's (`keepInTheRoom`). Put on it, `moves` keeps the chat first and the captions after (the relay above sends three sessions away), and both cases passed again (11.9 s; `moves` 4.6 s, the captions as above).

With `keepFollowingTranscript` handing `connect` no GOAWAY (`options.connect(movedBy && undefined)`), the case failed as it should: the reader went back to `https://127.0.0.1:31500` and was sent away again.

`packages/transcript/test/keep-following.test.ts` has the same on sessions of the test: a session that ends, a GOAWAY, the same GOAWAY after a session at the new place ended, and a second GOAWAY.

## Not looked at

- What the reader reads at the relay it was sent to: no transcriber runs in `when-full.ts`.
- The demo's watching page: `demo/captions.ts` uses `keepFollowingTranscript` now, and `demo/watch.ts` connects to the URI of a GOAWAY; type-checked and bundled, not opened in a browser.
- The pause: the next session after one a GOAWAY ended is made at once, once in a row; the test gives an `againMs` of 1 and does not tell the two apart.
