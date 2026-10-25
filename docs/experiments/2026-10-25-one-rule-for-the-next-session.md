# One rule for the next session

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 5, the reader for agents; the terminal viewer.

## Objective

The audit of the Rust code (2026-10-25) found the loops that connect again written twice, in `xmoq-watch` and in `xmoq-agent`, and apart in three ways. After a GOAWAY the agent connected at once every time, and the viewer only once in a row. A SUBSCRIBE refused with `GOING_AWAY` moved the viewer and left the agent where it was. And the agent waited 10 s for its room where the viewer waited 5 s. A room's WebSocket that went away during a session also ended both at their next session.

The rule is now one, `xmoq_viewer::reconnect::NextSession`, which reads `fixtures/ask-again/next-session.tsv`, as `createNextSession` of `@xmoq/moqt` does: the wait counts from the try before, and the table's sixth row has sessions of a length. Both programs:
- wait for the GOAWAY behind a `GOING_AWAY` (`xmoq_viewer_io::reconnect::go_away_behind`),
- take `--room-wait-ms` (5,000) and `--connect-wait-ms` (10,000),
- and join a room that went away again before the next session.

Do they go on as before, and does the agent keep going when its room goes away?

## Expected

Written before the runs:

- `mise run check` passes, with the five rows of `next-session.tsv` and the tests of what ends a viewer and where a GOAWAY sends it.
- `--only watch` and `--only agent` pass three times each.
- By hand, with `mise run agent-demo -- start` on ports 33443, 33080 and 33787:
  - The rooms' process is stopped by its PID: the reader writes `the room is gone` and goes on.
  - The rooms are started again on the same port, and the relay is sent SIGTERM, whose GOAWAY ends the session.
  - The reader joins the room again (the room says `connected: 1`) and goes on; it does not end with `the room is gone`.

## Environment

macOS 14.4, arm64, loopback, on `main` with this change. The relay was a debug build and the reader a release build, with Chromium 156.0.8078.4 of playwright-core for the publishing page.

## Result

**As expected.**

- `mise run check`: OK. `--only watch`: `5 of 5` three times; `--only agent`: `4 of 4` three times. After the change was moved onto `main` with #380, once more each, the same.
- By hand:
  - The reader wrote `{"message":"the room is gone: WebSocket protocol error: Connection reset without closing handshake: ...","type":"error"}` and went on.
  - Stopping the publishing page did not end its session: it asked for the video again (`XMOQ_AGENT_VIDEO_REFUSED error_code=0x10`). So the relay was sent SIGTERM. The reader wrote `{"namespace":["xmoq","demo"],"state":"ended","type":"broadcast"}`.
  - 15 s later it was still running, and the room said `{"broadcast":null,"connected":1}`.
  - Its next try at the relay, which was gone, gave `the session ended: the relay: nothing within 10000 ms`, and it went on.
  - `stop` ended the reader and the holder of the pipe; nothing of the run was left.

## What was not run

- GOAWAYs in a row at a relay (the second connect waits): the table pins it, and no relay here sends GOAWAY on every session.
- A SUBSCRIBE of the agent refused with `GOING_AWAY` and moving: no run made a relay refuse so.
- `--room-wait-ms` and `--connect-wait-ms` with other values than the defaults, beyond the tests of the flags.
