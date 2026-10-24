# The chat with tokens through the gatherer

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat.

## Objective

`interop/chat-tokens.ts` checked the pages' tokens through the own relay with `--auth-key-hex`, but a page published the room's `lines` in the gatherer's place, as the gatherer sent no token then. The gatherer now makes tokens of its own with the relay's key (`--auth-key-hex`, as in the demo with `--auth`). Do lines that pages say with the room's two tokens reach the other pages through the real gatherer, end to end?

## Expected

Written before the first run with the gatherer, at the top of `interop/chat-tokens.ts`:

- The gatherer with `--auth-key-hex` is told of `xmoq/chat` (`WATCHING`): the relay took its SETUP and its SUBSCRIBE_NAMESPACE.
- Pages a and e connect with `sub` for `xmoq/chat/r` and join as `a` and `e` with `pub` for their own name: their PUBLISH_NAMESPACE is accepted, the gatherer subscribes to each `said`, and each follows the room's `lines`.
- a says a line, then e: numbered 0 and 1, and both pages have both, live.
- Page f comes then with the same two tokens: its Joining FETCH, which the relay passes to the gatherer, brings 0 and 1 as history.
- b (`pub` for another name), c (no token of its own) and d (`pub` alone, on the connection and the request) are refused as before: b's and c's PUBLISH_NAMESPACE with 0x1, d's SUBSCRIBE for `lines` with 0x1.
- Nothing of b, c or d reaches the gatherer as a line.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.14; Chromium 156.0.8078.4 of playwright-core, headless; the own relay and `xmoq-chat` of this tree (debug builds), on `main` at `2e9a25f6` with this change. `bun interop/chat-tokens.ts --port 49150` once, then `mise run interop -- --base-port 49100` with the machine's lock for full runs, from 00:50:00.

## Result

**As expected, in both runs.**

- The gatherer printed `SETUP_OK` and `WATCHING prefix=xmoq/chat`, then `SUBSCRIBED` for a and e, and published the room (`ROOM_PUBLISHED room="r" forward=true largest=none`).
- a's line was numbered 0 and e's 1; both pages had `0 a live, 1 e live`.
- Page f had `0 a history, 1 e history`; the gatherer printed `FETCH_ANSWERED room="r" start=0,0 end=1,1 lines=2 unknown_through=none`.
- b and c were refused their PUBLISH_NAMESPACE with 0x1. d announced, and the gatherer followed it (`SUBSCRIBED room="r" name="d"`: d's `pub` allows its own namespace); d's SUBSCRIBE for `lines` was refused with 0x1. The gatherer took lines of a and e alone.

The full run ended 208 of 208 as expected (428.8 s; load average 13 and 17.4 GB of 18 GB of swap in use when it began).

## What was not run

- A token that expires during a session: the pages' tokens and the gatherer's are good for 600 s, longer than the run.
- The demo with `--auth`, in a browser, by a person.
