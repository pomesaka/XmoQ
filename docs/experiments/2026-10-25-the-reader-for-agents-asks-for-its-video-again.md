# The reader for agents asks for its video again

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4; the reader for agents (`crates/agent`).

## Objective

`xmoq-agent` gave its video up on a session when the subscription ended, and a `/frame` typed while it connected (again) was refused at once (`frame: waiting for the chat`, seen by `anna-4aa4` after a GOAWAY). Does the reader now ask for the video again, and keep a `/frame` until a video can answer it?

## Expected

Written before the runs:

- A `/frame` typed at once, before the reader has a session, is answered with a `frame` line once the video is subscribed to, and no `error`.
- A page ends its broadcast's tracks with PUBLISH_DONE and publishes anew: the reader writes `XMOQ_AGENT_VIDEO_ENDED ... asking again in 1000 ms` and then `XMOQ_AGENT_VIDEO_AGAIN`, and a `/frame` after it is answered with a picture of the new video.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; the own relay and `xmoq-agent` (debug builds) of this tree, on `main` at `f3f2edd5` with this change; a page with `publishBroadcast("xmoq/watch")`. By a script that was not kept, then `mise run interop -- --base-port 33000` with the machine's lock.

## Result

**As expected.**

- The `/frame` typed at once was answered by `frame` `000001-g1792891884787-o0.png` (640x360), after `XMOQ_AGENT_VIDEO` and `XMOQ_AGENT_CHAT`; no `error`.
- `endBroadcast`: `XMOQ_AGENT_VIDEO_ENDED PUBLISH_DONE status_code=0x2; asking again in 1000 ms`, then, the page publishing anew, `XMOQ_AGENT_VIDEO_AGAIN track_alias=2`. A `/frame` 1.5 s later was answered by `000002-g1792891886132-o12.png`, of a Group of the new video.

The full run ended 230 of 230 as expected (495.5 s). Put on `main` at `12de5ac0` (the transcript, and `interop/agent-frames.ts`), the pairings with the reader ran alone (`--only "agent"` and `--only "transcriber:"`) and ended as expected; `agent-frames.ts` among them, whose third `/frame` is answered on the second relay after a GOAWAY.

## What was not run

- The full run on `main` at `12de5ac0`.
- A video refused when asked for again (`XMOQ_AGENT_VIDEO_REFUSED`), and one given up (a SUBSCRIBE not answered in time).
