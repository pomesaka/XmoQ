# Vision

XmoQ is an experimental real-time media platform using QUIC, WebTransport and MoQ.

## Purpose

Stated by the project owner on 2026-10-25: **an agent such as Claude takes part in a broadcast or a meeting.** A CLI subscribes to the tracks and writes what they carry to stdout or to files, where the agent reads it.

Decided on 2026-10-25:
- The agent gets what is said as text: a transcriber beside the relay reads the sound with whisper.cpp and publishes the words as a track, which the agent subscribes to, as any viewer can.
- The agent speaks in the chat, as text, to begin with.
- Meetings, many to many, are in scope: 49 participants at most.
- First an agent follows a one-to-many broadcast; meetings come after that.

## Discussed direction
- One publisher, multiple viewers, then meetings.
- Browser publisher/viewer using TypeScript and WebCodecs.
- Rust relay and Rust terminal viewer (Ghostty).
- Experiment with terminal video, audio and chat.
- Cloudflare Durable Objects for room/application state.
- Explore future live streaming for events on a separately developed SNS.

## Goals
The goals of a broadcast (its delay, and how many viewers a relay and a broadcast have) and the size of a meeting are in the [requirements](requirements.md), rows "Latency, capacity" and "Meetings (many to many)".

## Undecided
What is not decided, for an agent, a meeting and the rest, is in the [open questions](open-questions.md).

No production readiness or shipped features are claimed.
