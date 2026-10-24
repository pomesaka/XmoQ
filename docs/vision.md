# Vision

XmoQ is an experimental real-time media platform using QUIC, WebTransport and MoQ.

## Discussed direction
- Initially: one publisher, multiple viewers.
- Browser publisher/viewer using TypeScript and WebCodecs.
- Rust relay and Rust terminal viewer (Ghostty).
- Experiment with terminal video, audio and chat.
- Cloudflare Durable Objects for room/application state.
- Explore future live streaming for events on a separately developed SNS.
- Bidirectional audio/video may be explored later, but is **not** part of the initial scope.

## Goals
Decided on 2026-10-24: a delay of 500 ms at most from a frame being made to its being shown, 100 viewers a room, 10 rooms open at once ([requirements](requirements.md)).

## Undecided
Browser/OS support, deployment, recording, moderation, authentication, licensing and operational requirements.

No production readiness or shipped features are claimed.
