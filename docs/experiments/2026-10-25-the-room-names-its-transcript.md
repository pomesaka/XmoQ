# The room names its transcript

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, rooms; the reader for agents and the transcriber.

## Objective

The reader for agents followed the transcript of the broadcast's namespace unless `--transcript` named another prefix, and the room did not say what a transcriber listens under. The room's hello now names it (`transcript`, from `TRANSCRIPT_PREFIX` with `{room}` put in the room's name; decided on 2026-10-25). Does the room say it, and does the reader follow it?

## Expected

Written before the runs:

- `check.mjs` against `run.mjs` with `TRANSCRIPT_PREFIX=xmoq/meet/{room}`: the hello of room `check-<n>` has `transcript: ["xmoq","meet","check-<n>"]`; with `xmoq/voice`, `["xmoq","voice"]`; the other checks pass as before.
- `xmoq-agent --room <room> --token no` in a room with `TRANSCRIPT_PREFIX=xmoq/voice` and no broadcast, where a transcriber listens under `xmoq/voice` to a page's voice in `xmoq/voice/page`: the reader, on its session at the room's relay for the chat, follows `speech` of `xmoq/transcript/xmoq/voice` and writes a `speech` line from `["xmoq","voice","page"]` with "relay" in it.

## Environment

macOS 14.4, arm64, loopback; Node 23.10.0 and Miniflare of `spikes/durable-objects-room`; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; the own relay and `xmoq-agent` (debug builds) and `xmoq-transcriber` (release) of this tree, with `ggml-tiny.bin`, on `main` at `6710d54f` with this change. The room on port 33190, the relay on 33191; the second by a script that was not kept.

## Result

**As expected.**

- `check.mjs` with `xmoq/meet/{room}`: `"transcript":["xmoq","meet","check-1792893093739"]`, `ALL CHECKS PASSED`. With `xmoq/voice` (and the relay): `"transcript":["xmoq","voice"]`, `ALL CHECKS PASSED`.
- The reader subscribed to the transcript at once (`XMOQ_AGENT_SPEECH track_alias=0`) and wrote `{"from":["xmoq","voice","page"],"number":1792893078203,...,"text":"Hello from the relay.","type":"speech"}`; no `broadcast` line, as the room had none.

### With tokens

Written before the run: in a room with `AUTH_KEY_HEX` and `TRANSCRIPT_PREFIX=xmoq/voice`, the relay with `--auth-key-hex` of the same key, and a transcriber with that key under `xmoq/voice`, the reader (asking the room for tokens) asks the room for `sub` in `xmoq/transcript/xmoq/voice`, and its SUBSCRIBE to `speech`, which carries that token alone, is answered with `SUBSCRIBE_OK`.

Run once by a script that was not kept, on the same tree and ports: **as expected**. The reader printed `XMOQ_AGENT_CHAT room=r name=claude` and `XMOQ_AGENT_SPEECH track_alias=0`, and no `XMOQ_AGENT_SPEECH_REFUSED`. No page published a voice: a page's `publishVoice` sends no token with its PUBLISH_NAMESPACE, which such a relay refuses, so no piece was read; the answer to the SUBSCRIBE is what was looked at. The same reader with `--token no` was meant to be refused there, and was not a comparison: the relay closed its session at the SETUP, which carried no token, before any SUBSCRIBE.

## What was not run

- A piece of speech through a relay with tokens (no page publishes a voice with one).
- A room's name with a `/` or a `*` in it, for which the hello says `null`.
- These in `mise run interop`, which runs no room.
