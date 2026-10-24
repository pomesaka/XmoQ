# Vision

XmoQ is an experimental real-time media platform using QUIC, WebTransport and MoQ.

## Purpose

Stated by the project owner on 2026-10-25: **an agent such as Claude takes part in a broadcast or a meeting.** A CLI subscribes to the tracks and writes what they carry to stdout or to files, where the agent reads it.

Decided on 2026-10-25:
- The agent gets what is said as text: the sound is transcribed with whisper.cpp on the agent's machine.
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
Decided on 2026-10-24, for a broadcast: a delay of 500 ms at most from a frame being made to its being shown, at the 99th percentile of the viewers' frames, 100 viewers a room, 10 rooms open at once, on as many relays as that takes ([requirements](requirements.md)). One relay took 500 viewers in all within it on one machine; the rooms split over relays may reach the goal, and were not measured. Decided on 2026-10-25, in two steps: one relay serves 500 viewers of a broadcast, and relays are added to build a broadcast of 10,000. For a meeting, 49 participants at most (decided on 2026-10-25); its delay is undecided.

## Undecided
What the agent's CLI writes and in what form, whether an agent is given pictures, whether an agent ever speaks with a voice, how the people of a meeting or a broadcast learn that an agent takes part and transcribes them, and how relays are added and a meeting is built ([proposed design](scale-and-meetings.md)). Browser/OS support, deployment, recording, moderation, authentication, licensing and operational requirements.

No production readiness or shipped features are claimed.
