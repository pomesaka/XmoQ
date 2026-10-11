# A room on a Durable Object, run locally

- Date: 2026-10-11
- Commit SHA: the commit that adds `spikes/durable-objects-room`, together with this record; the runs were made on its working copy
- Roadmap item: Phase 4, "Validate Durable Objects integration". Material for the open questions on the relay and Durable Objects, on how a room is found, and on chat; **nothing is decided here**

## Objective

The smallest room that works: a publisher says where it publishes, a viewer finds that by the name of the room and plays, a chat goes over a WebSocket of the same object, and the room knows how many are connected.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Durable Objects | Miniflare 4.20260730.0 (workerd, local), Node 23.10.0. **Not Cloudflare**: nothing was deployed |
| Browser, relay | Chromium 156.0.8078.4, headless; the own relay; the pages of `demo/` |

## Commands

```sh
(cd spikes/durable-objects-room && npm install && node run.mjs --port 45384)
(cd spikes/durable-objects-room && node check.mjs http://127.0.0.1:45384)
bun demo/serve.ts --port 45381 --relay-port 45382
# a script outside the repository opened /publish?room=studio and /watch?room=studio with ?rooms=http://127.0.0.1:45384
mise run check
```

## What was built

One Durable Object for a room name (`idFromName`). `PUT /rooms/<room>/broadcast` stores `{relayUrl, namespace, certSha256Hex}`; `GET /rooms/<room>` gives it with the count; `GET /rooms/<room>/ws` is a WebSocket (the hibernation API) on which the room sends the last 50 chat messages first and then every message with a number it gives, and the count when it changes. The pages of the demo take `?room=NAME`: the publishing page registers, the watching page asks.

## Observed results

- **A viewer found the broadcast by the room and played it**: the room gave the relay's URL, the hash of its certificate and the namespace as the publishing page had said them; the watching page connected to that and decoded 90 frames and 150 pieces of audio in 3.5 seconds. A room without a publisher: the page says "nobody publishes in this room".
- **Chat, one order for all**: two clients sent 20 messages each at once; both saw the same 40 in the same order, numbered 1 to 40 without a gap, each client's own in the order it sent them.
- **One who comes later** got the last 50 of 70 (numbers 21 to 70), the count (3) and the broadcast in its first message.
- **The count**: 1, 2, 3 as clients came; 2 after one left, told to the others and given by `GET`.
- One thing met: in `webSocketClose` the socket that closes is still among `getWebSockets()`; the count has to leave it out.

## Limitations and blockers

- **Local only.** Limits, prices, how long an object sleeps and wakes, and regions were not seen. That a Worker cannot take a WebTransport or QUIC connection, so that media cannot end at a Durable Object, is what `docs/architecture.md` assumes; it was not tried.
- **No authentication**: anybody can register a broadcast for any room, and name itself anything in the chat.
- **A registration outlives its publisher**: a page that closes leaves the room saying it publishes. Nothing here expires it.
- The relay knows nothing of rooms. No chat in the pages of the demo: the chat was run by a script.
- One run each.

## Open questions raised: how the relay and the room meet

| | Who tells the room | For | Against |
|---|---|---|---|
| A | The publishing page, as here | Nothing in the relay. Works with any relay | The room believes the page; a page that vanishes leaves a stale entry, unless the registration lives as long as a WebSocket of the publisher to the room |
| B | The relay, when a namespace is announced and when its session ends | The room is right about who is live. The relay can say more (viewers, bytes) | The relay gets an HTTP client, a secret, and a rule for which namespace is which room; a relay that cannot reach Cloudflare still has to carry media |
| C | Nobody: the room asks the relay when a viewer asks | No state to go stale | The relay gets an HTTP endpoint, and every lookup costs a request to it |
| D | Nobody: viewers find broadcasts through MoQ (SUBSCRIBE_NAMESPACE), and the room is for chat alone | One way to find things, and it is the protocol's | The relay does not have SUBSCRIBE_NAMESPACE; a viewer needs a relay to ask before it knows of any |

What speaks for A with the publisher's WebSocket as its sign of life, for now: it needs nothing of the relay, and the publisher has that WebSocket anyway, for the chat. B becomes worth it with authentication and hosting, which are undecided.
