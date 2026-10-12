# The terminal viewer joins a room

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, a managed room with browser and CLI interactions (the CLI's side)

## Objective

Whether the viewer of the terminal (`spikes/terminal-av`, `xmoq-watch`) can do what the watching page does with a room: learn from it where the broadcast is, get a token for a relay that asks for one, and take part in the chat.

## Environment

One machine, macOS, loopback. The demo of this working copy with authorization on ports of its own (`bun demo/serve.ts --auth`: `xmoq-relay` with a key, the pages, and the rooms of `spikes/durable-objects-room` run locally with Miniflare, which have the same key). The publishing page in headless Chromium, in a room. Nothing was deployed.

## What was built

`xmoq-watch --room http://host:port/rooms/<name>` (`--name`, `--say "a line"`, `--token no`). Nothing in `crates/`, `demo/` or the room's Worker was changed.

- It opens the room's WebSocket (crates `tokio-tungstenite` 0.30.0 and `serde_json`; `ws://` alone, no TLS), takes the broadcast from the room's `hello`, or waits up to 30 seconds for one, and connects to that relay with that hash of the certificate and that namespace.
- It asks for a token as the pages do (`{type: "token-for", role: "sub", namespace}`) and puts it, as Alias Type USE_VALUE and Token Type 0 before the text, into SETUP and into both SUBSCRIBEs.
- The chat: the last 3 lines are written under the picture, by the thread that writes the frames; where stderr is no terminal each line is also printed there (`CHAT ...`). Control characters are taken out of a line, since its text is anybody's. `--say` sends one line when the viewer has joined. **Nothing is read from the keyboard.**
- With the flags of before (`--url`, `--cert-sha256`, `--namespace`) it watches without a room, as it did.

## Commands

```sh
spikes/terminal-av/try-room.sh 15                                     # the check, headless
ROOM=http://127.0.0.1:8787/rooms/demo spikes/terminal-av/watch.sh     # for a person, next to the demo
```

## Observed results

`try-room.sh`, run twice for 15 seconds with the same lines for the room, the token, the chat and the refusal; the numbers are of the first run. A third run of 5 seconds wrote the picture to a file, for the last row. The picture went to `/dev/null` and the sound was at volume 0: nobody saw or heard it.

| What | Outcome |
|---|---|
| The room says where the broadcast is | `ROOM_BROADCAST relay=https://127.0.0.1:48910/ namespace=xmoq/demo` |
| The room gives a token | `ROOM_TOKEN given` |
| The viewer watches through the relay that asks for tokens, 15 s | 456 frames drawn of 456, 30.4 a second; 751 packets of sound |
| The viewer's line reaches the page | the page shows `1 terminal: hello from the terminal` |
| The page's line reaches the viewer | `CHAT 2 publisher: hello from the page` |
| The same without a token (`--token no`) | the relay closes the session: `ClosedByPeer { code: 2, reason: "authorization" }` (UNAUTHORIZED) |
| The chat under the picture | in the bytes written for the terminal: the cursor to the row under the picture, the line, back to the picture |

- In the first run the sound had 2 underruns in 15 seconds, where the runs without a room had none. Not looked into; the count of the second run was not kept.

## Not done, and not decided

- Seeing the chat under the picture in Ghostty.
- Typing a line while watching; a room that is deployed (`wss://`); a broadcast that begins or moves after the viewer has joined (it is noted, and the viewer stays where it is); a token that runs out while watching (the relay looks at it when a request comes, and the viewer sends none later).
- **Who gets a token is the room's to decide and is not decided**: the room gives one to anybody who asks, the terminal as the pages. The viewer was given nothing the page is not.

## Next action

None from this record. The rules of a room stay open.
