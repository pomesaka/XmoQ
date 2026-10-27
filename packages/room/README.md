# @xmoq/room

The rooms, asked over HTTP: where a room's relay is, the namespace of its broadcast, the tokens the relay checks, and the seats of a meeting, on a Durable Object. **Run locally with Miniflare only; nothing is deployed, and no Cloudflare account is used.** What a room answers is [`fixtures/room-api/`](../../fixtures/room-api/README.md), and the form of a token, shared with `crates/room-token`, is [`fixtures/room-token/`](../../fixtures/room-token/README.md).

```sh
bun install                  # in the repository root: Miniflare among the workspace's dependencies
node run.ts                  # the rooms, on http://127.0.0.1:8787 (--port N for another)
node check.ts                # against it, with the settings run.ts was given
```

The settings are in the environment of `run.ts`, and of `check.ts`, which then checks them; what each does and what happens without it is at the bindings of `run.ts`. `mise run demo` starts these rooms with the relay and the pages: the publishing page publishes in the namespace its room names, and the watching page subscribes there, waiting for a publisher. `?room=NAME` on both pages is another room.

**A token is given to anybody who asks**: who may publish and who may watch is not decided here. The room answers only for its own namespaces (`tokenForNamespace` has the table and why).

**What happens in a room goes over MoQ, and not through the room** (why, at the top of `src/worker.ts`): a broadcast that begins is a publisher at the room's namespace, which a viewer's SUBSCRIBE waits for; the chat goes through the gatherer `xmoq-chat` ([`crates/chat`](../../crates/chat/README.md)). How many are there and reactions are to go through the gatherer as well; until it has them, the pages show neither.

**A meeting** is a room of its own, under `/meetings/<name>`. A seat holds by its lease until it runs out, and its id and its place are free once it runs out or is left. Who is in the meeting is not the room's: the participants find each other at the relay. Why the room counts and why by a lease, at the top of `src/worker.ts` and at `seatFor`.
