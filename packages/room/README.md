# @xmoq/room

A room on a Durable Object: where its broadcast and its relay are, how many are connected, reactions, the seats of its meeting, and the tokens the relay checks. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** It was a spike until 2026-10-26 ([record](../../docs/experiments/2026-10-11-durable-objects-room.md)).

| File | What it is |
|---|---|
| `src/core/rules.ts` | What the room decides, with no Durable Object, socket or clock: which token-for it answers, a meeting's seats (`seatFor`, 49 at most), the prefixes of its transcripts |
| `src/token.ts` | The text of a token (`tokenText`), with WebCrypto; the form is the one `crates/room-token` makes and reads, and both read [`fixtures/room-token/`](../../fixtures/room-token/README.md) |
| `src/worker.ts` | The Worker and its Durable Object `Room`: the paths, the messages and why each is so, at its top |
| `run.ts` | Builds the Worker with `bun build` and runs it in Miniflare, under Node.js |
| `check.ts` | Checks a running room over HTTP and its WebSocket: the broadcast, a token, the count, no chat, and a meeting of 49 |

```sh
bun install                  # in the repository root: Miniflare among the workspace's dependencies
node run.ts                  # the rooms, on http://127.0.0.1:8787 (--port N for another)
node check.ts                # against it, with the settings run.ts was given
```

`mise run demo` starts these rooms with the relay and the pages: the publishing page tells the room where it publishes, over its WebSocket, and the watching page learns it there; both show the count. `?room=NAME` on both pages is another room.

**The settings** (in the environment of `run.ts`, and of `check.ts`, which then checks them):

| Setting | What it does | Without it |
|---|---|---|
| `AUTH_KEY_HEX` | The key the room shares with the relay: `token-for` and `join-meeting` give tokens. To anybody who asks: who may publish and who may watch is not decided here | Every token is `null` |
| `RELAY_URL`, `RELAY_CERT_SHA256_HEX` | The relay the hello names (`relay: {relayUrl, certSha256Hex}`), with a broadcast and without one, so that a participant can take part in the chat before anybody broadcasts (decided on 2026-10-25) | `relay: null` |
| `TRANSCRIPT_PREFIX`, `MEETING_TRANSCRIPT_PREFIX` | The prefixes a transcriber listens under for the room's broadcast (`transcript`) and its meeting (`meetingTranscript`), each as fields, `{room}` put in the room's name (decided on 2026-10-25) | `null`, and for a room whose name has a `/` or a `*` |
| `REACTION_BATCH_MS` | The reactions of that long go out as one message, for a measurement ([record](../../docs/experiments/2026-10-12-reactions-over-the-rooms-websocket.md)) | Each as it comes |

**No chat** (decided on 2026-10-24): the chat goes over MoQ tracks through the gatherer `xmoq-chat` ([`crates/chat`](../../crates/chat/README.md)), and the room neither carries nor keeps a line ([record](../../docs/experiments/2026-10-24-the-chat-off-the-rooms-websocket.md)).

**A meeting** (decided on 2026-10-25): a page sends `{type: "join-meeting", participant: "<id>"}` and, while fewer than 49 sockets of the room have joined, gets `{type: "meeting", participant, publish: {namespace: ["xmoq", "meet", "<room>", "<id>"], token}, subscribe: {namespace: ["xmoq", "meet", "<room>", "*"], token}}`; else `{type: "meeting-full", most: 49}`. What is not answered is at `seatFor`; why the room and not the relay counts, at the top of `src/worker.ts`.
