# @xmoq/room

A room on a Durable Object: where its broadcast and its relay are, how many are connected, reactions, the seats of its meeting, and the tokens the relay checks. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** It was a spike until 2026-10-26 ([record](../../docs/experiments/2026-10-11-durable-objects-room.md)).

| File | What it is |
|---|---|
| `src/core/rules.ts` | What the room decides, with no Durable Object, socket or clock: which token-for it answers, a meeting's seats (`seatFor`, 49 at most, held by a lease; `seatTimesOf`), the prefixes of its transcripts |
| `src/token.ts` | The text of a token (`tokenText`), with WebCrypto; the form is the one `crates/room-token` makes and reads, and both read [`fixtures/room-token/`](../../fixtures/room-token/README.md) |
| `src/worker.ts` | The Worker and its Durable Objects: `Room` (`/rooms/<name>`, its WebSocket) and `MeetingRoom` (`/meetings/<name>`, over HTTP); the paths, the messages and why each is so, at its top |
| `run.ts` | Builds the Worker with `bun build` and runs it in Miniflare, under Node.js |
| `check.ts` | Checks a running room over HTTP and its WebSocket: the broadcast, a token, the count, no chat, and the seats of a meeting of 49, asked with [`@xmoq/room-client`](../room-client/README.md) |

```sh
bun install                  # in the repository root: Miniflare among the workspace's dependencies
node run.ts                  # the rooms, on http://127.0.0.1:8787 (--port N for another)
node check.ts                # against it, with the settings run.ts was given
```

`mise run demo` starts these rooms with the relay and the pages: the publishing page tells the room where it publishes, over its WebSocket, and the watching page learns it there; both show the count. `?room=NAME` on both pages is another room.

**The settings** (in the environment of `run.ts`, and of `check.ts`, which then checks them):

| Setting | What it does | Without it |
|---|---|---|
| `AUTH_KEY_HEX` | The key the room shares with the relay: `token-for` and the seats of a meeting give tokens. To anybody who asks: who may publish and who may watch is not decided here. One that is set and is not hex of one byte at least, the empty one among them, stops `run.ts`, as it stops the relay | Every token is `null` |
| `BROADCAST_NAMESPACES` | The namespaces of the broadcasts `token-for` gives `pub` and `sub` in, each its fields joined with `/`, separated by `,`; a field that is empty or has a `*` stops `run.ts` | None: no token in a broadcast's namespace. `demo/serve.ts` gives `xmoq/demo` |
| `RELAY_URL`, `RELAY_CERT_SHA256_HEX` | The relay the hello names (`relay: {relayUrl, certSha256Hex}`), with a broadcast and without one, so that a participant can take part in the chat before anybody broadcasts (decided on 2026-10-25) | `relay: null` |
| `TRANSCRIPT_PREFIX`, `MEETING_TRANSCRIPT_PREFIX` | The prefixes a transcriber listens under for the room's broadcast (`transcript`) and its meeting (`meetingTranscript`), each as fields, `{room}` put in the room's name (decided on 2026-10-25) | `null`, and for a room whose name has a `/` or a `*` |
| `MEETING_SEAT_MS`, `MEETING_RENEW_MS` | How long a seat of a meeting holds, and how often it is renewed (**provisional**); one that is no whole number from 1, or a renewal not shorter than the seat, stops `run.ts` | 60,000 and 20,000 |
| `REACTION_BATCH_MS` | The reactions of that long go out as one message, for a measurement ([record](../../docs/experiments/2026-10-12-reactions-over-the-rooms-websocket.md)) | Each as it comes |

**Tokens in the namespaces of the room alone** (decided on 2026-10-26): `token-for` answers `sub` in `xmoq/chat/<room>`, in `xmoq/transcript/<prefix>` of the room's transcripts and in a namespace of `BROADCAST_NAMESPACES`, and `pub` in `xmoq/chat/<room>/<one field>` and in a namespace of `BROADCAST_NAMESPACES`; any other ask is answered with `token: null`, so that a reader that matches the answers to its asks by their order goes on. A token for `sub` in `xmoq` allowed a SUBSCRIBE_NAMESPACE that tells of every room's meeting and the ids of its participants (`tokenForNamespace` has the table and why).

**No chat** (decided on 2026-10-24): the chat goes over MoQ tracks through the gatherer `xmoq-chat` ([`crates/chat`](../../crates/chat/README.md)), and the room neither carries nor keeps a line ([record](../../docs/experiments/2026-10-24-the-chat-off-the-rooms-websocket.md)).

**A meeting** (decided on 2026-10-25; over HTTP since 2026-10-26) is a room of its own, under `/meetings/<name>`: what it answers is [`fixtures/room-api/`](../../fixtures/room-api/README.md), which [`@xmoq/room-client`](../room-client/README.md) and the Rust client read. A seat holds by its lease until it runs out, 60 s after it was taken or last renewed, and is renewed every 20 s; its id and its place are free once it runs out or is left. Who is in the meeting is not the room's: the participants find each other at the relay. Why the room counts and why by a lease, at the top of `src/worker.ts` and at `seatFor`.
