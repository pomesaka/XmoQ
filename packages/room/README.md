# @xmoq/room

The rooms, asked over HTTP: where a room's relay is, the namespace of its broadcast, the tokens the relay checks, and the seats of a meeting, on a Durable Object. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** It was a spike until 2026-10-26 ([record](../../docs/experiments/2026-10-11-durable-objects-room.md)).

| File | What it is |
|---|---|
| `src/core/rules.ts` | What the room decides, with no Durable Object, socket or clock: which asks for a token it answers, a meeting's seats (`seatFor`, 49 at most, held by a lease; `seatTimesOf`), the prefixes of its transcripts |
| `src/token.ts` | The text of a token (`tokenText`), with WebCrypto; the form is the one `crates/room-token` makes and reads, and both read [`fixtures/room-token/`](../../fixtures/room-token/README.md) |
| `src/worker.ts` | The Worker: the room of a broadcast (`/broadcasts/<name>`, the same answer for everybody) and the Durable Object `MeetingRoom` (`/meetings/<name>`); the paths and why each is so, at its top |
| `run.ts` | Builds the Worker with `bun build` and runs it in Miniflare, under Node.js |
| `check.ts` | Checks a running room: the room of a broadcast and its tokens, no path but the table's, and the seats of a meeting of 49, asked with [`@xmoq/room-client`](../room-client/README.md) |

```sh
bun install                  # in the repository root: Miniflare among the workspace's dependencies
node run.ts                  # the rooms, on http://127.0.0.1:8787 (--port N for another)
node check.ts                # against it, with the settings run.ts was given
```

`mise run demo` starts these rooms with the relay and the pages: the publishing page publishes in the namespace its room names, and the watching page subscribes there, waiting for a publisher. `?room=NAME` on both pages is another room.

**The settings** (in the environment of `run.ts`, and of `check.ts`, which then checks them):

| Setting | What it does | Without it |
|---|---|---|
| `AUTH_KEY_HEX` | The key the room shares with the relay: `tokens` and the seats of a meeting give tokens. To anybody who asks: who may publish and who may watch is not decided here. One that is set and is not hex of one byte at least, the empty one among them, stops `run.ts`, as it stops the relay | Every token is `null` |
| `RELAY_URL`, `RELAY_CERT_SHA256_HEX` | The relay every room names (`relay: {url, certSha256Hex}`; decided on 2026-10-25) | `relay: null` |
| `TRANSCRIPT_PREFIX`, `MEETING_TRANSCRIPT_PREFIX` | The prefixes a transcriber listens under for the room of a broadcast and for that of a meeting (`transcript` of each), as fields, `{room}` put in the room's name (decided on 2026-10-25); `demo/serve.ts` gives `xmoq/broadcast/{room}` and `xmoq/meet/{room}` | `null`, and for a room whose name has a `/` or a `*` |
| `MEETING_SEAT_MS`, `MEETING_RENEW_MS` | How long a seat of a meeting holds, and how often it is renewed (**provisional**); one that is no whole number from 1, or a renewal not shorter than the seat, stops `run.ts` | 60,000 and 20,000 |

**Tokens in the namespaces of the room alone** (decided on 2026-10-26): `tokens` answers `sub` in `xmoq/chat/<kind>/<room>` and in `xmoq/chat/<kind>/<room>/*` (the relay allows a SUBSCRIBE_NAMESPACE with a `*` token alone, and this one watches who is in the chat), in `xmoq/transcript/<prefix>` of the room's transcript and, for the room of a broadcast, in `xmoq/broadcast/<room>`, and `pub` in `xmoq/chat/<kind>/<room>/<one field>` and in `xmoq/broadcast/<room>`; any other ask is answered with `token: null`. A token for `sub` in `xmoq/*` would allow a SUBSCRIBE_NAMESPACE that tells of every room's meeting and the ids of its participants (`tokenForNamespace` has the table and why).

**What happens in a room goes over MoQ, and not through the room** (decided on 2026-10-26): a broadcast that begins is a publisher at the room's namespace, which a viewer's SUBSCRIBE waits for; the chat goes through the gatherer `xmoq-chat` ([`crates/chat`](../../crates/chat/README.md)). How many are there and reactions are to go through the gatherer as well; until it has them, the pages show neither.

**A meeting** (decided on 2026-10-25) is a room of its own, under `/meetings/<name>`: what it answers is [`fixtures/room-api/`](../../fixtures/room-api/README.md), which [`@xmoq/room-client`](../room-client/README.md) and the Rust client read. A seat holds by its lease until it runs out, 60 s after it was taken or last renewed, and is renewed every 20 s; its id and its place are free once it runs out or is left. Who is in the meeting is not the room's: the participants find each other at the relay. Why the room counts and why by a lease, at the top of `src/worker.ts` and at `seatFor`.
