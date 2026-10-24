# The demo's chat with tokens: the pages' side

- Date: 2026-10-24
- Commit SHA: the commit that adds this record, on `fcb4fd87`
- Roadmap item: none closed. The pages' side of the chat with `--auth`; the gatherer's side (`xmoq-chat --auth-key-hex`) is another change and was not in this tree

## Objective

With `bun demo/serve.ts --auth` the relay refuses a request without a token of the room, and the pages had no chat. The relay now takes a token whose last field is `*` for the namespaces under it (Q50). What a participant needs: `pub` for `xmoq/chat/<room>/<name>` for its PUBLISH_NAMESPACE, and `sub` for `xmoq/chat/<room>` for its SUBSCRIBE for `lines` and its Joining FETCH. Does a page with the second on its connection and the first on its PUBLISH_NAMESPACE alone (`publishNamespaceToken` of `joinChat`) get through the relay, and are the wrong tokens refused? Does the demo start the gatherer with `--auth` only where the gatherer takes the relay's key?

## Expected

**Written before the first run** (the head of `interop/chat-tokens.ts`). Through the own relay with `--auth-key-hex`, the tokens made by the script with the relay's key, and a page publishing `lines` in the gatherer's place:

- The page in the gatherer's place connects with `pub` for `xmoq/chat/r`, publishes `lines`, and sends line 0.
- Page a connects with `sub` for `xmoq/chat/r` and joins as `a` with `pub` for `xmoq/chat/r/a`: its PUBLISH_NAMESPACE is accepted, it follows the room (SUBSCRIBE), has line 0 by its Joining FETCH, and line 1, sent then, live.
- Page b, with the same `sub` and `pub` for `xmoq/chat/r/a`, is refused its PUBLISH_NAMESPACE as `b` with REQUEST_ERROR 0x1 (UNAUTHORIZED).
- Page c, with `sub` and no token of its own, is refused the same way: the connection's token allows no announcing.
- Page d connects with `pub` for `xmoq/chat/r/d` and gives the same token to `joinChat` (the library sends it once): its PUBLISH_NAMESPACE is accepted, and its SUBSCRIBE for `lines` refused with 0x1.

**The demo.** Not written down before it ran; what the change was made for: with `--auth` and the gatherer of this tree, which takes no `--auth-key-hex`, the gatherer is not started and both pages say "no chat", while publishing and watching go as with tokens before; without `--auth`, the chat goes as before.

## Environment

macOS 14.4, arm64, loopback; this tree on `fcb4fd87` with the change; Bun 1.3.14; Chromium of playwright-core 1.64.0, headless; the own relay and `xmoq-chat` of this tree (debug builds), and Miniflare of `spikes/durable-objects-room` for the rooms.

## Commands

```sh
cargo build --workspace
cargo build --locked --release -p xmoq-watch -p xmoq-quiet-peer -p xmoq-moqt-client -p xmoq-relay
mise run interop -- --only "chat: with tokens" --base-port 46300   # 3 times
mise run interop -- --only "chat:" --base-port 46300               # once
bun demo/serve.ts --auth --port 45381 --relay-port 45382 --rooms-port 45384 --chat-store <a file outside the tree>
bun demo/serve.ts --port 45381 --relay-port 45382 --rooms-port 45384 --chat-store <the same>
# a script outside the repository opened /publish and /watch of one room in each, and said a line on /watch
mise run check
```

## Result

**The pairing, as expected, 3 of 3** (1.5 to 2.3 s each): page a joined and had `0 history, 1 live`; b and c were refused their PUBLISH_NAMESPACE with 0x1; d announced, and its SUBSCRIBE was refused with 0x1. The line of Group 0 that the Joining FETCH of a brought can only have come from what the relay keeps of the track (the latest Group): the page in the gatherer's place names no `serve` handler for a FETCH, and refuses every one. **`--only "chat:"`**: 5 of 5 pairings ended as expected, the four of the gatherer and this one.

**The demo with `--auth`**: `serve.ts` printed `chat: no chat: this build of the gatherer takes no --auth-key-hex, and the relay of --auth would refuse it`, and started no gatherer. Both pages showed that line in place of the chat; the publishing page published ("publishing (session 1) · 31 frames a second") and the watching page played ("30 frames a second shown"), with their tokens of the room.

**The demo without `--auth`**: the gatherer started; a line said on the watching page showed on both (`0 viewer-303: hello from the viewer`), and the line under the form said "the gatherer listens, following the room".

**How `serve.ts` asks the gatherer**: given `--auth-key-hex 00` alone, the gatherer of this tree prints `unknown flag --auth-key-hex` and exits with 2. A stand-in that prints `--url is needed` instead was read as taking the flag, and a path that is not there as not; a script outside the repository called the same check on the three.

## Not run

- **The pages' chat with `--auth` end to end**: the two tokens a page asks the room for each time it connects for the chat (`demo/shared.ts`), and the gatherer with the relay's key. It needs a gatherer that takes `--auth-key-hex`; once there, `bun demo/serve.ts --auth` starts it without a change. The pairing above stands in a page for the gatherer.
- A token that runs out during a session: a participant's SUBSCRIBE made again after its subscription ended is then refused until the page connects again, which asks the room anew. Not tried.
- The same pairing through moqtail's relay: it checks no tokens.

After that, `mise run check` passed and `mise run interop -- --base-port 45300` ended 202 of 202 as expected (405 s).

After the review of the change, the check of the gatherer's flag was taken out of `serve.ts`: with `--auth` it starts the gatherer with `--auth-key-hex` whatever the gatherer is. With the gatherer of this tree, `bun demo/serve.ts --relay-port 45381 --port 45382 --rooms-port 45384 --auth` printed `chat:    over the tracks; ...`, then the gatherer's `unknown flag --auth-key-hex` and its usage, and `the gatherer of the chat exited with 2`, as for any other exit of the gatherer; the pages were not opened. The two runs of the probe above were of the code as it was before.
