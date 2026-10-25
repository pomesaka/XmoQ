# The room names two transcripts

- Date: 2026-10-25
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 4, rooms and meetings; the reader for agents.

## Objective

The room named one transcript, and the demo set it to its meeting's (`xmoq/meet/{room}`). A reader of the room's broadcast then had to be given `--transcript xmoq/demo`. Now the hello names the broadcast's as `transcript` (from `TRANSCRIPT_PREFIX`) and the meeting's as `meetingTranscript` (from `MEETING_TRANSCRIPT_PREFIX`). The reader takes the second with `--meeting` and the first otherwise. Does each reader of the demo follow its own transcript without the flag?

## Expected

Written before the runs:

- `check.mjs` against `run.mjs` with `TRANSCRIPT_PREFIX=xmoq/demo` and `MEETING_TRANSCRIPT_PREFIX=xmoq/meet/{room}` prints `ALL CHECKS PASSED`. The hello has `"transcript":["xmoq","demo"]` and `"meetingTranscript":["xmoq","meet","check-<n>"]`.
- `mise run agent-demo -- start`, whose reader is no longer given `--transcript`, writes a `speech` line from `["xmoq","demo"]`.
- The run of the [meeting page](2026-10-25-the-demo-has-a-meeting-page.md) passes as before. Three `/meet` pages and `mise run agent -- --room … --meeting demo` are in it, and the agent writes a `speech` line from `["xmoq","meet","demo",<id>]`.

## Environment

macOS 14.4, arm64, loopback. Node 23.10.0 and Miniflare of `spikes/durable-objects-room`. Bun 1.3.12, and Chromium 156.0.8078.4 of playwright-core, headless. The demo ran on ports 33443, 33080 and 33787, with `--transcribe-language en`. It ran on `main` at `80a87234` with this change. The driver of the meeting was a script that was not kept.

## Result

**As expected.**

```
the late one's hello: {…,"transcript":["xmoq","demo"],"meetingTranscript":["xmoq","meet","check-1792912984793"]}
ALL CHECKS PASSED
agent-demo: {"from":["xmoq","demo"],…,"text":"Hello from the relay.","type":"speech"}
meeting:    {"from":["xmoq","meet","demo","19f6c0bc20be5ee8"],…,"text":"Hello from the relay.","type":"speech"}
```

## What was not run

- `MEETING_TRANSCRIPT_PREFIX=xmoq/meet` with one transcriber for every room, which the setting is there for.
- A room without either setting, where the reader falls back to `xmoq/meet/<room>` in a meeting and to the broadcast's namespace otherwise (unit tests of `xmoq_agent::transcript`).
