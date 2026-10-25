# An agent keeps the meeting over a GOAWAY

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 6, an agent in a meeting.

## Objective

The audit of the Rust code (2026-10-25) found that `xmoq-agent --meeting` made who is in the meeting anew with each session. After a GOAWAY or a new connection, everybody there was written as `joined` again. Nobody who went meanwhile was written as `left`, and the reader forgot who spoke last. That contradicts the `participant` row of `crates/agent/README.md`. The meeting is now the reader's, as the chat's `presence` is, and is told when each session ends. Does the reader keep it over a GOAWAY?

## Expected

Written before the run, at the top of `interop/agent-meeting.ts`:

- After the parts that were there before, p4 joins on the first relay.
- p1 (again) and p3 publish on a second own relay, which the first names in its GOAWAY (`--redirect-uri`).
- The first relay is sent SIGTERM. The reader moves to the second relay and writes p3 as joined, then p4 as left once 2 s have passed without it.
- p1 is written as joined once in the whole run: 6 `participant` lines in all.

## Environment

macOS 14.4, arm64, loopback, on `main` at `fe5f5dc1` with this change. The relays and the reader were debug builds, and the pages ran in Chromium 156.0.8078.4 of playwright-core.

## Result

**As expected.** `mise run interop -- --base-port 33000 --only agent`: `4 of 4 pairings ended as expected`. The pairing of the meeting said:

```
after the relay's GOAWAY it moved to the second relay, wrote p3 as joined and p4 as left, and p1, there again, not as joined a second time; events.jsonl had the same 11 lines
```

## What was not run

- The same run with the reader as it was before the change: what it would have written is read from the code, not seen.
- A new connection after an error, and the room's reconnection with `--room`: those go through the same end of a session in `run` (`reader.meeting` is told there), and were not run.
