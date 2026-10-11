# Requirements and status

**Discussed** means explicitly raised in planning; **proposed** means suggested but not confirmed; **decided** means confirmed by the project owner and not yet implemented; **implemented** means the code is in `main` and a recorded run in `docs/experiments/` exercised it; **validation required** means not proven.

| Requirement | Status |
| --- | --- |
| Own MoQ Transport implementation in Rust and TypeScript, pinned to draft 18 | Implemented for the Phase 0b scope: the codec and session in Rust and TypeScript, a client in each, and a minimal relay in Rust that routes by track and by announced namespace and takes one track from up to 4 publishers. One Object passes between every pairing of own and `moqtail` clients through either relay. The parts of the draft that are not implemented are listed in [implementation notes](moqt-implementation-notes.md) |
| One-to-many live broadcast | Discussed. The relay fans one Object out to several subscribers; no media has been sent |
| Browser publishing and viewing | Decided for Phase 1: Chromium (Chrome, Edge) on desktop, video only, shown on loopback. Other browsers are undecided |
| WebCodecs and MoQ/WebTransport | Discussed; validation required |
| Media packaged as LOC, tracks described by an MSF catalog (`draft-ietf-moq-msf-01`), with the FETCH that needs | The MSF catalog is implemented: `@xmoq/msf` reads, follows and publishes a catalog track, run against `moqlivemock` in both directions; delta updates and GZIP are not. Media in LOC is decided and not implemented. A relay cache of the latest Group of a track is implemented for a track that has a publisher, with the bounds decided on 2026-10-11 (8 MiB and 4,096 Objects a track, 256 MiB in all); keeping it for 30 seconds after the last publisher has gone is decided and not implemented. Which revision of LOC and whether every Object gets a stream of its own are undecided ([open decisions](open-questions.md)) |
| Audio | Decided: Opus, from Phase 2 |
| Rust relay and Rust CLI | Discussed |
| Ghostty video, audio, chat | Desired; validation required |
| Durable Objects for room/chat state | Discussed; integration undecided |
| SNS event integration | Future goal |
| Multiple publishers / bidirectional calls | Future possibility |
| Reactions | Proposed; delivery undecided |
| Video codec | Decided for Phase 1: H.264 in Annex B format. Profile, level and key frame interval are undecided ([open decisions](open-questions.md)) |
| Latency, capacity | Undecided |
| Authentication, privacy, recording | Undecided |
| License and contribution policy | Undecided |

No user-facing CLI or installation steps are implemented yet. `xmoq-moqt-client` is a one-shot client for interop runs, not the terminal viewer.
