# Vision

XmoQ is an experimental real-time media platform using QUIC, WebTransport and MoQ.

## Purpose

**An agent such as Claude takes part in a broadcast or a meeting.** A CLI subscribes to the tracks and writes what they carry to stdout or to files, where the agent reads it. The agent speaks in the chat, as text, to begin with. Meetings, many to many, are in scope.

## Direction

- One publisher, multiple viewers, then meetings.
- Browser publisher and viewer using TypeScript and WebCodecs.
- Rust relay and Rust terminal viewer (Ghostty), with video, audio and chat in the terminal.
- Cloudflare Durable Objects for room and application state.

## Out of scope

- Production readiness: nothing here is claimed to be shipped or operated.
- Live streaming for events on a separately developed SNS is a future direction, not committed; its contract is not discussed.
