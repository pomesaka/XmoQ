# A page fetches the lines it missed

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat.

## Objective

A participant of `@xmoq/chat` asked for the newest 16 lines with every Joining FETCH, so a page whose subscription to `lines` was made again lost the lines between what it had and the newest 16. Decided on 2026-10-24: when it subscribes again, it fetches from the line after the newest it had to the new LARGEST_OBJECT, 200 lines at the most by default (`catchUpLines`, 1 to 1,000); where it missed more, it fetches the newest 200 and tells of a gap before them (`onGap`). The rule is `historyToFetch` of `packages/chat/src/core/history.ts`. A participant of a new connection is given the newest line the page has (`lastHad`), so that it catches up the same way. Does it ask for those ranges, and do the relays and the gatherer answer them?

## Expected

Written before the runs.

- `interop/chat-pages.ts`, after what it checked before (page b has left, a's line 21 was numbered):
  - Page c leaves, a says 22 to 26, and c joins again on the same session with `lastHad` 21. Its Absolute Joining FETCH starts at Group 22 and brings 22 to 26 as history; no gap is told. Through the own relay the gatherer prints `FETCH_ANSWERED room="r" start=22,0 end=26,1 lines=5 unknown_through=none`.
  - c leaves again, a says 27 to 31, and c joins again with `lastHad` 26 and `catchUpLines` 3. It is told of a gap after 26 and before 29, has 29 to 31 as history, and never 27 or 28. The gatherer prints `start=29,0 end=31,1 lines=3 unknown_through=none`. a's next line, 32, reaches c live.
  - Through the own relay alone (`--restart yes`): the gatherer is stopped and started again on the same store. The subscriptions of a and c end and are made again. Nothing was said meanwhile, so neither FETCHes (the new gatherer prints no `FETCH_ANSWERED`); before this change each would have asked for the newest 16. a's line 33 reaches both live, and no line comes twice.
- The same through `moqtail`'s relay 0.15.0, without the restart (`chat.ts` has why). Not known before the run: whether that relay answers the catching-up FETCHes from what it holds, as it answered the FETCH for the newest 16, so the gatherer's answers are not checked there.
- The unit tests of `historyToFetch`: nothing had; had up to the largest; a few missed; exactly the limit missed (no gap); one more than the limit (a gap); a largest below what was had (numbered anew by a gatherer with another store: forgotten, the newest 16).

## Environment

macOS 14.4, arm64, loopback; this working copy with the change; Bun 1.3.14; Chromium of playwright-core 1.64.0, headless; the own relay and `xmoq-chat` of this tree (debug builds, `cargo build --workspace`); `moqtail`'s relay 0.15.0. `bun interop/matrix.ts --only "chat:" --base-port 45300`, 4 times.

## Result

**As expected, through both relays, in each of 4 runs** (`chat:`, 4 of 4 pairings ended as expected each time; the own pages' pairing took 11.5 and 10.6 s through the own relay and 10.6 and 10.8 s through `moqtail`'s in the two runs whose times were kept):

- Page c, joining again with `lastHad` 21 after a said 22 to 26, had `4-19 history, 20-21 live, 22-26 history`, and no gap. Joining again with `lastHad` 26 and `catchUpLines` 3 after 27 to 31, it printed `CHAT_GAP after=26 before=29` and had 29 to 31 as history and 32 live; 27 and 28 never came.
- Through the own relay the gatherer answered `start=22,0 end=26,1 lines=5 unknown_through=none` and `start=29,0 end=31,1 lines=3 unknown_through=none`. Through `moqtail`'s relay 0.15.0 the gatherer answered neither: that relay answered both catching-up FETCHes from what it held, as it does the FETCH for the newest 16.
- The restart (own relay): the subscriptions of a and c ended with `PUBLISH_DONE 0x2` and were made again; the gatherer started again printed no `FETCH_ANSWERED`, and line 33 came live to both, each line once (`0-33 live` for a).
- `mise run check` passed after the change; the unit tests of `historyToFetch` and `catchUpLines` are in `packages/chat/test/history.test.ts`.

## What was not run

- The demo's pages (`demo/room-chat.ts`, `?chatCatchUpLines=N`, the dimmed line for a gap) were not opened in a browser.
- A subscription made again within one participant while lines are said by others: the gatherer is away while its subscription is down, so no line is numbered then, and the window after it comes back was not made to happen. The rule is the same function as for a participant that joins again; its unit tests have the cases.
- A gatherer started again on another store (the numbers go back): the unit tests alone.

After that, `mise run check` passed and `mise run interop -- --base-port 45300` ended 200 of 200 as expected.
