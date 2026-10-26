# Tokens in the namespaces of the room

- Date: 2026-10-26
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, authorization: the room (`@xmoq/room`) made a token for any namespace anybody asked for, but for those with a `*` or a `/` in a field and a meeting's; a token for `sub` in `xmoq` allowed a SUBSCRIBE_NAMESPACE for that prefix (transport §10.18), which tells of every room's meetings and the ids of their participants. Done on the provisional answer to a question the user has not answered yet.

## Objective

With `token-for` answered in the namespaces of the room alone (`tokenForNamespace` of `packages/room/src/core/rules.ts`: the room's chat, its transcripts and the broadcasts of `BROADCAST_NAMESPACES`), do the demo's pages still get every token they ask for, and is a token above the room refused?

**Loopback, one machine.**

## Expected

Said before the first run (to the PM, not in a file): the namespaces the pages and the Rust clients ask for (read in `demo/`, `crates/viewer-io/src/room_socket.rs`, `crates/agent/src/bin/xmoq-agent/main.rs` and `crates/watch/src/bin/xmoq-watch/main.rs`) are all of the room, so the demo with `--auth` publishes, watches, meets and chats as before; `sub` in `xmoq`, in `xmoq/chat` and in another room's chat is not answered.

Written while it was made: a broadcast's namespace is taken from the setting alone, and not from the one a publisher registered, since anybody registers any namespace, `xmoq` among them.

## Environment

macOS 14.4, arm64, Node.js 24.21.0, Miniflare 4.20260730.0; `node packages/room/run.ts --port 35787` with `AUTH_KEY_HEX`, the relay and the two transcript prefixes, and `node packages/room/check.ts` against it. The demo: `bun demo/serve.ts --relay-port 35443 --port 35080 --rooms-port 35787 --auth` (`BROADCAST_NAMESPACES=xmoq/demo`), its pages in Chromium of Playwright 1.64.0, headless.

## Result

**As expected.**

- `check.ts` ended `ALL CHECKS PASSED`: a token for `sub` in `xmoq/demo` and in the room's chat, and none for `xmoq`, `xmoq/chat` and another room's chat, besides what it checked before.
- The demo with `--auth`: the watching page played after its room's socket was cut for 3 s (316 frames, 5 connections), with its token for `xmoq/demo`; a meeting page was seated and publishing, and seated again; two pages said lines in the room's chat, and a page that came later had them from the history, with their tokens for `xmoq/chat/<room>` and `xmoq/chat/<room>/<name>`.
- The rows of `tokenForNamespace` (`packages/room/test/rules.test.ts`) fail, ten of them, with every ask answered, and one with `pub` answered under a name in the chat.

After a review: the first version left an ask outside the room unanswered, and a reader matches the answers to its asks by their order (the pages, `asked` of `crates/viewer-io`), so one such ask would have had it wait, or take the next answer for it. Every ask is answered now, with `token: null` where no token is made, and `BROADCAST_NAMESPACES` is none where it is not set (`demo/serve.ts` gives `xmoq/demo`). The watching page and the meeting page take the prefix of their transcript from the room's hello. Run again:

- `check.ts`, with `BROADCAST_NAMESPACES` not set and set to `xmoq/demo`: `ALL CHECKS PASSED` both times, nine asks with nine answers, a token for the room's chat (`sub`, and `pub` under a name) and, where it is set, for `xmoq/demo`, and `null` for the rest.
- `crates/watch/scripts/try-room.sh 15`: the terminal viewer had its token from the room (`ROOM_TOKEN given`), played 15 s (448 frames drawn), said a line in the chat and had the page's; without a token the relay closed its session (`code: 2, reason: "authorization"`).
- `xmoq-agent --room <the demo's room> --seconds 10`: the broadcast, a joined participant and a line of the chat in `events.jsonl`, exit 0. With `--transcript xmoq/elsewhere`, a prefix the room gives no token in: the room answered `null`, the relay refused the SUBSCRIBE for the transcript (`XMOQ_AGENT_SPEECH_REFUSED error_code=0x1`, asked again every second), and the rest went on as without it, exit 0.
- The demo with `--auth`: the watching page played (317 frames), the meeting page was seated again, the chat had its history; the watching page's asks were answered with tokens for `xmoq/chat/demo`, `xmoq/chat/demo/<name>` and `xmoq/transcript/xmoq/demo`.

## Not looked at

- The captions with `--auth`: the transcriber ran, but the publishing page sent its tone, which is told to be no voice, so no transcript came; the token the watching page asks for it (`xmoq/transcript/xmoq/demo`) is a row of the table.
- The meeting page's token for its transcript: it is asked when Claude Code is called, which was not pressed.
- A SUBSCRIBE_NAMESPACE for `xmoq` at the relay with a token of the room: the room makes none now, and the relay's check is the same as before.
