# An agent reads the demo with xmoq-agent

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4; the reader for agents (`crates/agent`).

## Objective

Can an agent, by the commands at the top of `crates/agent/README.md`, start a demo of its own beside the one a person runs, put `xmoq-agent` in its room with `mise run agent`, say a line, and have a picture of the broadcast?

## Expected

Written before the run:

- `bun demo/serve.ts --relay-port 33443 --port 33080 --rooms-port 33787 --chat-store <file>` starts the relay, the rooms, the gatherer and the pages on those ports, and nothing on 4443, 8080 or 8787 is touched.
- With the publishing page open (in a headless Chromium, which a person would open by hand), `mise run agent -- --room http://127.0.0.1:33787/rooms/demo --name claude --out <dir> < <pipe>` writes `broadcast` `began` for `xmoq/demo`.
- `hello from the agent` written to the pipe comes back as a `chat` line from `claude`; `/frame` is answered with a `frame` line whose PNG is the page's picture at about that moment.

## Environment

macOS 14.4, arm64, loopback; Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless, for the publishing page alone; the relay and the gatherer (debug builds) and `xmoq-agent` (the release build `mise run agent` makes) of this tree, on `main` at `3da4dddd` with this change. No transcriber: the demo starts none. The commands were those of the README with a directory of the session in place of `/tmp/room`, and a pipe held open by a `sleep` as there.

## Result

**As expected.**

- The demo printed `publish: http://localhost:33080/publish`, `rooms: http://127.0.0.1:33787/` and the chat's store.
- `mise run agent` built the release binary (8.2 s) and the reader wrote, in this order: `{"name":"publisher","type":"joined"}` (on its session at the room's relay, before the broadcast was registered), `{"namespace":["xmoq","demo"],"state":"began","type":"broadcast"}`, `{"from":"claude","number":0,...,"text":"hello from the agent","type":"chat"}`, and `{"asked":"command",...,"height":360,"path":".../frames/000001-g1792894060961-o2.png","timestamp_us":1792894082626000,"type":"frame","width":640}`, 3 ms after the chat line.
- The PNG showed the demo's painted clock at `02:08:02.622` (UTC) and `frame 653`; the line's `timestamp_us` is `02:08:02.626`.
- At the end, `XMOQ_AGENT_FRAMES given=1317 decoded=3 passed_over=1298 taken=1`. The SUBSCRIBE to the transcript (`speech` of `xmoq/transcript/xmoq/demo`, which nobody published) was refused with `0x2` as the relay stopped.
- Every process of the run was stopped by its PID; 4443, 8080 and 8787 were not touched.

## What was not run

- A transcript in the demo: the demo starts no transcriber.
- The commands with `--auth`.
