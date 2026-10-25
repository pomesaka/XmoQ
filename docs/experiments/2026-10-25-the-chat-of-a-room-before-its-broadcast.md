# The chat of a room before its broadcast

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, rooms and chat; the reader for agents (`crates/agent`).

## Objective

A room said where its relay is only with its broadcast, and the chat is on the relay, so `xmoq-agent --room` could not take part in the chat before somebody broadcast. The room's hello now names its relay (`relay`) with a broadcast and without one (decided on 2026-10-25). Does the room say it, and does the reader take part in the chat of a room with no broadcast?

## Expected

Written before the runs:

- `check.mjs` against `run.mjs` with `RELAY_URL` and `RELAY_CERT_SHA256_HEX`: the hello of a room without a broadcast has `broadcast: null` and `relay` with those two; its other checks pass as before.
- `xmoq-agent --room <room> --token no` in a room without a broadcast, where a page has joined the chat as `p` through the gatherer: the reader writes `joined` p, no `broadcast` line, p's line as `chat`; a line of its stdin reaches the page; `/frame` gets the error `frame: no broadcast yet`.
- The same with tokens (the room with `AUTH_KEY_HEX`, the relay and the gatherer with `--auth-key-hex` of the same key, the reader asking the room for tokens): the relay takes the reader's SETUP with the chat's `sub` token, and the chat goes as above. Then a publisher registers a broadcast in the room: the reader ends its session, connects for the broadcast, writes `broadcast` `began`, and the page's next line comes to it as `chat`, without p joining again.

## Environment

macOS 14.4, arm64, loopback; Node 23.10.0 and Miniflare of `spikes/durable-objects-room`; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; the own relay, `xmoq-chat` and `xmoq-agent` of this tree (debug builds), on `main` at `347d0ebe` with this change. The room on port 33190, the relay on 33191. The second by a script that was not kept.

## Result

**As expected.**

- `check.mjs`: the late one's hello was `{"type":"hello","connected":3,"broadcast":null,"relay":{"relayUrl":"https://127.0.0.1:33191/","certSha256Hex":"abab…ab"}}` (the values given to `run.mjs`), and it ended `ALL CHECKS PASSED`.
- The reader wrote, without `at`: `{"name":"p","type":"joined"}`, p's line `hello from p` as `chat` 0, `{"message":"frame: no broadcast yet","type":"error"}`, and its own line as `chat` 1; no `broadcast` line. The page had `1 claude hello from claude`.
- With tokens (a second script, not kept; `--frames off`, the registered broadcast `xmoq/nobody` having no tracks): `joined` p, `chat` 0 `hello from p`, `{"namespace":["xmoq","nobody"],"state":"began","type":"broadcast"}`, `chat` 1 `again from p`; the reader printed `XMOQ_AGENT_CHAT` twice, once a session. The relay closed no session of the reader for its SETUP.

## What was not run

- `check.mjs` against a room run without the two, where the hello's `relay` is `null`.
- A broadcast with tracks that comes while the reader is at the room's relay, and its pictures (`--frames on`).
- These in `mise run interop`, which runs no room. The full run with this change ended 225 of 225 as expected (443.2 s) on `347d0ebe`, and 228 of 228 (476.2 s) once it was put on `main` at `75233f71`: nothing else changed.
