# The demo's pages chat over the tracks

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat over MoQ tracks (decided on 2026-10-24): the participants' side in the pages, beside the room's WebSocket until the move is complete.

## Objective

The gatherer (`xmoq-chat`) numbers, keeps and publishes the lines of a room ([record](2026-10-24-chat-gatherer.md)), and so far only own-rust clients spoke to it. The demo's pages are to be participants: announce `xmoq/chat/<room>/<name>`, say each line as a Group of `said`, and read the room's `lines` with SUBSCRIBE and a Joining FETCH for the history. `joinChat` of the new package `@xmoq/chat` does that, and the pages use it beside the room's WebSocket. Do pages chat through the gatherer, does a page that comes later get the newest lines, does each page show the way each line came, and do the TypeScript reader and the Rust one read the payloads alike?

## Expected

**Written before the first run** (it is the head of `interop/chat-pages.ts`):

- Pages a and b join room r; the gatherer follows each (`SUBSCRIBED`), and each page has the gatherer's subscription to its `said` and its own subscription to the room's `lines`.
- a, b, a, b say a line each, one after the other: the gatherer numbers them 0 to 3, and both pages get all four, live, with those numbers and names.
- a says a line of 513 bytes: the page keeps it back (`too-long`) and the gatherer never sees it.
- b says 6 lines at once, 1.5 s after its last: the page sends 5 (4 to 8) and keeps the sixth back (`too-fast`); the gatherer drops none.
- a says 11 more, 250 ms apart (9 to 19).
- Page c comes then: its Joining FETCH brings the newest 16 lines, 4 to 19, as history, and nothing older.
- c says a line: numbered 20, and all three pages get it live.
- Every page has each line once, and says by which way it came: a and b had 0 to 20 live, c 4 to 19 as history and 20 live.

Added after the first run through the own relay, which had it: the gatherer answers the FETCH itself, `FETCH_ANSWERED room="r" start=4,0 end=19,1 lines=16`, as the own relay holds the latest Group alone and passes a longer range to the publisher.

**For the payloads**, written before the TypeScript reader: the rows added to `fixtures/chat-tracks/` where `JSON.parse` and `serde_json` read the same bytes differently (`-0`, `1.0`, `1e400` in a member more, an escaped lone surrogate, nesting of 127 and 128, a member twice, spaces around, a comma after the last member, something after the object) give in `xmoq-chat-tracks` what the rows say; the TypeScript reader then gives the same for every row.

**For the demo** (by hand, in headless Chromium): a line typed on the publishing page shows on both pages twice, once `[track]` and once `[ws]`; a page with `?chatTextBytes=10` keeps a longer line back and says so; a page that comes later shows the track's lines as history.

## Environment

macOS 14.4, arm64, loopback; the working copy on `main` at `43a083b8` with this change; Bun 1.3.14; Chromium 156.0.8078.4 of playwright-core 1.64.0, headless; the own relay and `xmoq-chat` of this tree (debug builds); `moqtail`'s relay 0.15.0.

## Result

**The payloads, as expected.** With the 14 rows added (11 to `said.tsv`, 3 to `lines.tsv`), `cargo test -p xmoq-chat-tracks` passed before the TypeScript reader existed: `serde_json` reads 127 levels and not 128, takes neither `-0` nor `1.0` as a time, refuses the whole payload for `1e400` and for `"\ud800"`, and keeps the last of a member given twice. The Rust test of bytes a text table cannot hold got a byte order mark and a tab as it is in a string, both no JSON. `bun test` of `@xmoq/chat` then read all 27 and 14 rows, and the same four cases, with the same outcome.

**Through the own relay, as expected**: the first run (`bun interop/chat-pages.ts --port 45390`) and 11 runs of `bun interop/matrix.ts --only "<the pairing>" --base-port 45300` (5 of the own-relay pairing alone, then 6 with the FETCH check added, the last with the code as it is now), 12 of 12:

| Step | The gatherer | The pages |
|---|---|---|
| a and b join | `PARTICIPANT`, `ROOM_PUBLISHED room="r" forward=true largest=none`, `SUBSCRIBED` for each | a's SUBSCRIBE for `lines`, held by the relay under RENDEZVOUS_TIMEOUT, was answered once the room was published |
| a, b, a, b | `LINE` 0 to 3 | both: 0 to 3, live |
| 513 bytes | nothing | `kept-back too-long` |
| 6 at once | `LINE` 4 to 8, no `LINE_DROPPED` | `sent` five times, `kept-back too-fast` |
| 11 more | `LINE` 9 to 19 | |
| c comes | `FETCH_ANSWERED room="r" start=4,0 end=19,1 lines=16 unknown_through=none` | c: 4 to 19, history |
| c says one | `LINE` 20 | a and b: 0 to 20 live; c: 4 to 19 history, 20 live; each once |

A run of the own-relay pairing alone took 5.6 s; the two pairings together, which the matrix runs side by side, 13.3 to 14.0 s.

**Through `moqtail`'s relay 0.15.0, as expected as well**, in 6 of 6 runs of the driver with `--relay moqtail` (`--port 45391`), then in 6 runs of the matrix with the pairing `moqtail relay: chat: own pages ...`. Two things differ from the own relay. Page a's first SUBSCRIBE for the room was refused at once with `REQUEST_ERROR 0x10` ("Unknown track namespace"), since that relay does not hold a SUBSCRIBE until a publisher comes, and the page asked again 2 s later and was answered. **The gatherer was asked for no FETCH**: no `FETCH_ANSWERED` in any of the 12 runs (the last 6 check it), and page c had 4 to 19 all the same, so that relay answered the Joining FETCH from what it had kept of the track itself.

**The demo**, by hand, on other ports (`bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 --chat-store <a file in a scratch directory>`), a script of headless Chromium opening `/publish?room=R`, `/watch?room=R&chatTextBytes=10`, and a second `/watch?room=R` later:

- "hello from the publisher" showed on both pages as `[track] 0 publisher` and `[ws] 1 publisher`, each with the time it was taken; "hi back" of the viewer as `[track] 1` and `[ws] 2`.
- On the viewer with `chatTextBytes=10`, "this is longer than ten bytes" was not sent on either way, and the line under the form said `not sent: longer than 10 bytes`.
- Six lines of the publisher, 400 ms apart: `[track]` 2 to 7 and `[ws]` 3 to 8 on both pages; a `[ws]` line came before the `[track]` line of the same text as often as after.
- The page that came later showed the room's history on the WebSocket (1 to 8) and the track's lines 0 to 7, marked as history.

The serve process was stopped with SIGINT, and the relay, the rooms and the gatherer with it.

## What was not run

- A page in a real browser window, and a person reading the chat. The demo was driven by a script only.
- More than 16 lines before a page comes, in the demo (the interop run has 20).
- The gatherer started again under running pages: the pages subscribe again 2 s after their subscription ends, and the FETCH then brings the newest 16 again. Not run.
- Two pages with the same name (two publishing pages are both `publisher`).
- `--auth`: the gatherer has no token, so the demo does not start it there and the chat goes over the WebSocket alone; not run.
- The terminal viewer: it still chats over the room's WebSocket.

After that, `mise run interop -- --base-port 45300` ended 194 of 194 as expected (358 s). Then the Joining FETCH of the pages was changed from Relative (Joining Start 15) to Absolute, from the largest Group less 15, as the terminal viewer asks: moqtail's relay refuses a Relative Joining Start larger than the track's largest Group. After that change, `mise run check` passed and `bun interop/matrix.ts --only chat: --base-port 45300` ended 4 of 4 as expected five times running, the pages' two pairings among them; the whole matrix was not run again.
