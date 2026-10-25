# An agent takes a seat in a room's meeting

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, rooms and meetings; the reader for agents.

## Objective

`xmoq-agent --meeting` ran with `--url` alone: nothing let it into a room's meeting or gave it the tokens. The room now gives the seats of its meeting, 49 at most (`join-meeting`, Q58, decided on 2026-10-25). With `--room <room> --meeting <name>` the reader asks the room for a seat, and uses the seat's `sub` in `xmoq/meet/<room>/*`. It announces nothing in the meeting. Does it get in through a relay that asks for tokens, and does its seat count among the 49?

## Expected

Written before the run:

- Two pages each ask the room for a seat as `p1` and `p2`, connect with the seat's `sub`, and publish under `xmoq/meet/r/<id>` with its `pub`. The own relay was started with `--auth-key-hex`, and the room has the same key.
- `xmoq-agent --room http://127.0.0.1:33987/rooms/r --meeting r --speech off` writes `participant` `joined` for `p1` and `p2`, and no other.
- `/frame p1` gives a `frame` line with `"id":"p1"`.
- The room counts three seats: of 47 more sockets that ask, 46 get one and the 47th gets `meeting-full`.
- The reader writes no `error` line, and no `XMOQ_AGENT_*REFUSED` line.

## Environment

macOS 14.4, arm64, loopback. Bun 1.3.12; Chromium 156.0.8078.4 of playwright-core, headless; Node 23.10.0 and Miniflare of `spikes/durable-objects-room`. The room is `worker.js` of #362, which was not yet on `main`. The own relay and `xmoq-agent` are debug builds of `main` at `9c91aec1` with this change. The relay was on port 33191 and the room on 33987. The driver used `interop/harness.ts` and was a script that was not kept.

## Result

**As expected.**

```
[agent] XMOQ_AGENT_MEETING room=r
[agent] {"at":1792908182745,"id":"p1","state":"joined","type":"participant"}
[agent] {"at":1792908182745,"id":"p2","state":"joined","type":"participant"}
[agent] XMOQ_AGENT_MEETING_VIDEO id=p1
[agent] {"asked":"command",...,"height":360,"id":"p1",...,"type":"frame","width":640}
[driver] of 47 more: 46 seated, then meeting-full
[driver] participant lines: 2; refused or error lines: []
```

The pages' SETUP and requests carried the room's tokens, `v1|xmoq/meet/r/*|sub|...` and `v1|xmoq/meet/r/p2|pub|...`, and their PUBLISH_NAMESPACE was answered with `PUBLISH_NAMESPACE_OK`.

## What was not run

- A session longer than the token's 600 s, after which the reader's new requests would be refused (Known gaps in `crates/agent/README.md`).
- A reader that finds the meeting full, which ends with an `error`.
- The transcript of the meeting with the room's tokens (`--speech off`).
- These in `mise run interop`, which runs no room.
