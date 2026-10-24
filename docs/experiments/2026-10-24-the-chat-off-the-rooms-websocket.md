# The chat off the room's WebSocket

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, chat (Q49, decided on 2026-10-24: the chat goes over the MoQ tracks alone, and the room's WebSocket carries none).

## Objective

Until now the demo's pages sent each line on the chat's tracks and on the room's WebSocket, and showed both, marked `[track]` and `[ws]` ([record](2026-10-24-demo-chat-over-tracks.md)). The room (`spikes/durable-objects-room`) numbered the WebSocket's lines and kept the last 50. After the change the room keeps the count, where the broadcast is, the tokens and the reactions, and the pages chat over the tracks alone. Does the room still do what it keeps, does it carry no line, and do the pages chat with nothing of the WebSocket?

## Expected

Written before the runs.

- `node check.mjs` of the room, without a key and with one: the broadcast registers and goes with its publisher's socket; a token has the form `v1|xmoq/demo|sub|<expiry>|<mac>` where there is a key and is null without one; a `{type: "chat"}` message reaches nobody; the hello of one who comes later is `{type, connected, broadcast}` with no `history`, and says 3 are connected; when one leaves, the room says 2 and the others are told 2.
- The demo (`bun demo/serve.ts` on other ports), a script of headless Chromium opening `/publish?room=R` and `/watch?room=R`: a line typed on either page shows on both pages once, as `HH:MM:SS <number> <name>: <text>`, with no `[track]` or `[ws]`; the numbers are the gatherer's, from 0. A page that comes later shows the earlier lines once, as history. Both pages show the count of the room, and a reaction pressed on one page reaches the other.
- `bun demo/serve.ts --auth`: no gatherer starts, both pages say `no chat: the gatherer has no token for --auth` where the chat was, have no form, and still show the count and play the broadcast.

## Environment

macOS 14.4, arm64, loopback; the working copy at `8239ed0a` with this change; Node 23.10.0 and Bun 1.3.14; Miniflare 4.20260730.0 of the spike; Chromium of playwright-core 1.64.0, headless; the own relay and `xmoq-chat` of this tree (debug builds). The demo on other ports: `bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 --chat-store <a file in a scratch directory>`, and the same with `--auth`. The rooms alone: `node run.mjs --port 45384`.

## Result

**The room, as expected**, once without a key and once with `AUTH_KEY_HEX` (`node check.mjs http://127.0.0.1:45384`, `ALL CHECKS PASSED` both times):

| Check | Without a key | With a key |
|---|---|---|
| The broadcast | registered, gone with its publisher's socket (`connected` 0) | the same |
| `token-for` sub in `xmoq/demo` | `token` null | `v1\|xmoq/demo\|sub\|<expiry>\|<mac>` |
| A `{type: "chat"}` message | reached nobody (0 messages of type `chat` among 3 clients) | the same |
| The late one's hello | `{"type":"hello","connected":3,"broadcast":null}` | the same |
| The count as the first client was told | 1, 2, 3; then 2 after one left | the same |

**The demo, as expected**, in 3 runs of a script of headless Chromium (`/publish?room=R`, `/watch?room=R`, and a second `/watch?room=R` after the two lines), a new room each run:

- "hello from the publisher" typed on the publishing page and "hi back" on the watching page showed on both pages as `HH:MM:SS 0 publisher: hello from the publisher` and `HH:MM:SS 1 viewer-<n>: hi back`, live, once each, with no mark of a way; the line under the form said `sent`.
- The page that came later showed the same two lines once each, as history (the Joining FETCH).
- Both pages said that 2 were connected, and 3 with the later page; a reaction pressed on the watching page was counted on the publishing page (`data-reactions` 1).

**With `--auth`, as expected**, in 1 run: `serve.ts` printed `chat:    no chat: the gatherer has no token for --auth` and started no gatherer; both pages had `no chat: the gatherer has no token for --auth` in place of the chat and no form, said that 2 were connected, and the watching page played (30 frames a second shown); a reaction reached the publishing page.

Each serve process was stopped with SIGINT, and the relay, the rooms and the gatherer with it.

## What was not run

- A page in a real browser window, and a person reading the chat.
- The terminal viewer (`xmoq-watch`) against the changed room: it still sends its lines to the room's WebSocket, which the room no longer answers.
- `interop/`: nothing there spoke to the room's WebSocket, and nothing of it was changed.
