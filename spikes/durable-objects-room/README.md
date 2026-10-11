# Spike: a room on a Durable Object

A room that says where its broadcast is, carries a chat, and knows how many are connected. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** Record: [docs/experiments/2026-10-11-durable-objects-room.md](../../docs/experiments/2026-10-11-durable-objects-room.md).

```sh
npm install            # miniflare, into this directory
node run.mjs           # the rooms, on http://127.0.0.1:8787
node check.mjs         # the broadcast, the chat and the count, against it
```

`mise run demo` in the repository root starts these rooms with the relay and the pages: the publishing page tells the room where it publishes, over its WebSocket, and the watching page learns it there; both show the chat and the count. `?room=NAME` on both pages is another room.

With `AUTH_KEY_HEX` in the environment of `run.mjs`, the room gives out tokens the relay checks (`token-for` on the WebSocket). To anybody who asks: who may publish and who may watch is not decided here.

`worker.js` has the paths and the messages at its top.
