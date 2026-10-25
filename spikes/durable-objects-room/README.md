# Spike: a room on a Durable Object

A room that says where its broadcast is, knows how many are connected, and passes reactions on. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** Record: [docs/experiments/2026-10-11-durable-objects-room.md](../../docs/experiments/2026-10-11-durable-objects-room.md).

**No chat** (decided on 2026-10-24): the chat goes over MoQ tracks through the gatherer `xmoq-chat` ([`crates/chat`](../../crates/chat/README.md)), and the room neither carries nor keeps a line. A `{type: "chat"}` message is not answered ([record](../../docs/experiments/2026-10-24-the-chat-off-the-rooms-websocket.md)).

**Reactions** (2026-10-12): a client sends `{type: "reaction", emoji}` and everybody in the room gets it; nothing is kept. `REACTION_BATCH_MS=100 node run.mjs` makes the room send the reactions of that long as one message, which was for a measurement ([record](../../docs/experiments/2026-10-12-reactions-over-the-rooms-websocket.md)).

```sh
npm install            # miniflare, into this directory
node run.mjs           # the rooms, on http://127.0.0.1:8787
node check.mjs         # the broadcast, a token, the count, no chat, and a meeting of 49, against it
```

`mise run demo` in the repository root starts these rooms with the relay and the pages: the publishing page tells the room where it publishes, over its WebSocket, and the watching page learns it there; both show the count. `?room=NAME` on both pages is another room.

With `AUTH_KEY_HEX` in the environment of `run.mjs`, the room gives out tokens the relay checks (`token-for` on the WebSocket). To anybody who asks: who may publish and who may watch is not decided here.

**A meeting** (decided on 2026-10-25): a page sends `{type: "join-meeting", participant: "<id>"}`, the id a random one it makes, and while fewer than 49 sockets of the room have joined it gets `{type: "meeting", participant, publish: {namespace: ["xmoq", "meet", "<room>", "<id>"], token}, subscribe: {namespace: ["xmoq", "meet", "<room>", "*"], token}}` (the tokens null without `AUTH_KEY_HEX`); else `{type: "meeting-full", most: 49}`. A socket counts from its join until it closes, and the same id on the same socket is answered again, with new tokens. An id that is empty, over 64 characters, or has a `/` or a `*`, another id on a socket that joined, and an id another open socket has are not answered. `token-for` makes no token in `xmoq/meet` or under it, so that the 49 cannot be passed by. Why the room and not the relay counts, at the top of `worker.js`.

With `RELAY_URL` and `RELAY_CERT_SHA256_HEX` in the environment of `run.mjs` (and of `check.mjs`, which then checks them), the hello names the relay of the rooms as `relay: {relayUrl, certSha256Hex}`, with a broadcast and without one, so that a participant can take part in the chat before anybody broadcasts (decided on 2026-10-25; why, at the top of `worker.js`). Without them it is `null`. `mise run demo` gives them.

With `TRANSCRIPT_PREFIX` and `MEETING_TRANSCRIPT_PREFIX` (and the same to `check.mjs`), the hello names the prefixes a transcriber listens under for the room's broadcast, as `transcript`, and for its meeting, as `meetingTranscript`, each as fields (`["xmoq", "meet", "<room>"]`), `{room}` in it put in the room's name; a reader follows `speech` of `xmoq/transcript/<prefix>`, a reader of the meeting the second (decided on 2026-10-25; why, and why the meeting's is not fixed, at the top of `worker.js`). Without its setting, and for a room whose name has a `/` or a `*`, each is `null`. `mise run demo` gives `xmoq/demo`, the namespace of its one broadcast, and `xmoq/meet/{room}`.

`worker.js` has the paths and the messages at its top.
