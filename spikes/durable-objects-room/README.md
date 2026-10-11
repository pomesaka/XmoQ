# Spike: a room on a Durable Object

A room that says where its broadcast is, carries a chat, and knows how many are connected. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** Record: [docs/experiments/2026-10-11-durable-objects-room.md](../../docs/experiments/2026-10-11-durable-objects-room.md).

```sh
npm install            # miniflare, into this directory
node run.mjs           # the rooms, on http://127.0.0.1:8787
node check.mjs         # the broadcast, the chat and the count, against it
```

With the demo (`mise run demo` in the repository root), in Chromium:

- <http://localhost:8080/publish?room=studio> publishes, and tells the room `studio` where: the relay's URL, the hash of its certificate, the namespace.
- <http://localhost:8080/watch?room=studio> asks the room, and connects to what it says.

`worker.js` has the paths and the messages at its top.
