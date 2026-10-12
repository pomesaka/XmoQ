# Reactions over the room's WebSocket, and whether the room has to bundle them

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, "Implement chat; evaluate reactions". How reactions are delivered stays undecided.

## Objective

A viewer presses an emoji and everybody in the room sees it. Built in the smallest way over the room's WebSocket, as the chat is. When many press at once, does the room have to bundle the reactions of a while into one message, or can it pass each on as it comes?

Expected, written before the runs: at 100 reactions a second to 10 or 20 connections each one can be passed on as it comes, within a few milliseconds; bundling costs the bundle's time in delay and saves messages, which shows in the room's processor time at 1,000 a second and hardly at 100.

## What was built

- **The room** (`spikes/durable-objects-room/worker.js`): a client sends `{type: "reaction", emoji}` and the room sends it to every socket, the sender's among them. The emoji is cut at 8 UTF-16 units. A reaction has no number and is not kept: somebody who comes later sees none.
- **For the measurement only**: with `REACTION_BATCH_MS` (0 without it) the room gathers the reactions of that long and sends one `{type: "reactions", counts}`, each emoji with how many. A timer in the object, no alarm and no storage.
- **The demo's pages**: four buttons under the chat (👍 ❤️ 😂 👏) and a line where a reaction stands for two seconds; a bundle is shown as `👍×12`.

## Environment

macOS 14.4, arm64, loopback. The room in Miniflare (`node run.mjs`), nothing deployed. Clients: Node 23.10's `WebSocket`, 10 and 20 connections to one room, which send in turn so that 100 or 1,000 reactions a second leave in all, for 8 s. Every reaction carries when it was sent; the time is to when a client got it. The processor time is that of the process that listens on the room's port (`workerd`), by `ps`.

## Commands

```sh
cd spikes/durable-objects-room && [REACTION_BATCH_MS=100] node run.mjs --port 45384
# a script of this run: N WebSockets to /rooms/<room>/ws, {type: "reaction", emoji, sent}
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384   # the buttons, with three pages
```

## Result

**In the demo**: a viewer pressed 👍 and ❤️; the publishing page, that viewer and another viewer each showed both and counted 2, and showed nothing 2.5 s later. `bun demo/check.ts` says "THE DEMO PLAYS" as before.

**Under load** (one run each; every connection got every reaction in every run):

| Room | Connections | Reactions a second | Messages a second to each | Sent to received, ms: median / 95th / most | The room's process, of one core |
|---|---|---|---|---|---|
| as they come | 10 | 100 | 100 | 0.6 / 1.5 / 3.4 | 3% |
| as they come | 10 | 1,000 | 1,000 | 1.5 / 3.0 / 7.8 | 11% |
| as they come | 20 | 100 | 100 | 0.8 / 1.5 / 5.1 | 5% |
| as they come | 20 | 1,000 | 1,000 | 2.2 / 3.8 / 16.1 | 16% |
| bundles of 100 ms | 10 | 100 | 10 | 55 / 102 / 105 | 1% |
| bundles of 100 ms | 10 | 1,000 | 10 | 55 / 103 / 105 | 3% |
| bundles of 100 ms | 20 | 100 | 10 | 55 / 103 / 106 | 2% |
| bundles of 100 ms | 20 | 1,000 | 10 | 56 / 103 / 108 | 3% |

- **The expectations held.** At these sizes the room does not have to bundle: 100 a second to 20 connections is 2,000 messages a second out of the room, 5% of a core, and a reaction is there within 5 ms. Ten times that (20,000 messages a second) took 16% and 16 ms at most.
- **What bundling buys is messages**: 10 a second to each client whatever is pressed, and a room that takes 1 to 3% of a core. It costs half the bundle's time on average and all of it at most.
- **What it is for is not the room here but the client and the link**: 1,000 messages a second to every viewer is more than a page can show and more than a phone's link should carry for emoji. That was not measured: the clients were a script that counts.

## Not done

- More than 20 connections, and a room on Cloudflare, where a message out of a Durable Object is billed and limited in ways Miniflare does not show. The price of 20,000 messages a second was not looked up.
- A limit on how often one client may react. Without one a single client can make every other client receive whatever it sends.
- A page under 1,000 reactions a second: what it draws, and what the drawing costs.
- The object's sleep: the bundling timer keeps it awake for the bundle's time; the room's sockets use the hibernation API otherwise.
- Reactions as a MoQ track, and the viewer in a terminal.
