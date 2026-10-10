# Requirements and status

**Discussed** means explicitly raised in planning; **proposed** means suggested but not confirmed; **decided** means confirmed by the project owner and not yet implemented; **implemented** means the code is in `main` and a recorded run in `docs/experiments/` exercised it; **validation required** means not proven.

| Requirement | Status |
| --- | --- |
| Own MoQ Transport implementation in Rust and TypeScript, pinned to draft 18 | Decided; partly implemented. The codec, the client-side session and a client are implemented in Rust and in TypeScript, each against `moqtail` peers. The relay is not, and the two own sides have not been run against each other |
| One-to-many live broadcast | Discussed |
| Browser publishing and viewing | Discussed |
| WebCodecs and MoQ/WebTransport | Discussed; validation required |
| Rust relay and Rust CLI | Discussed |
| Ghostty video, audio, chat | Desired; validation required |
| Durable Objects for room/chat state | Discussed; integration undecided |
| SNS event integration | Future goal |
| Multiple publishers / bidirectional calls | Future possibility |
| Reactions | Proposed; delivery undecided |
| Specific codec, latency, capacity | Undecided |
| Authentication, privacy, recording | Undecided |
| License and contribution policy | Undecided |

No user-facing CLI or installation steps are implemented yet. `xmoq-moqt-client` is a one-shot client for interop runs, not the terminal viewer.
