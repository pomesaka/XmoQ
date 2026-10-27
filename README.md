<div align="center">
  <img src="docs/assets/xmoq-logo.svg" alt="XmoQ logo" width="160">
  <h1>XmoQ</h1>
  <p><strong>Experimental real-time media over QUIC.</strong></p>
</div>

> [!IMPORTANT]
> **Early implementation, on one machine.** An own MoQ Transport (draft 18) in Rust and TypeScript and a relay in Rust carry a page's video and sound to other pages and to a terminal viewer, with a chat and captions, and a CLI lets an agent read a broadcast or a meeting. Nothing has run over a real network or on Cloudflare. What is done and what is open is in the [requirements](docs/requirements.md).

XmoQ explores one-to-many live streaming using MoQ/WebTransport, browser WebCodecs, a Rust relay and terminal CLI, and Cloudflare Durable Objects for application state. Its purpose is to let an agent such as Claude take part in a broadcast or a meeting: a CLI subscribes and writes what the tracks carry where the agent reads it ([vision](docs/vision.md)).

## Architecture at a glance

![XmoQ architecture](docs/assets/architecture.svg)

*The intended components. What of it is built is in the [architecture](docs/architecture.md).*

## Documentation

Start with the [vision](docs/vision.md), what is done and what is open ([requirements](docs/requirements.md)), the [architecture](docs/architecture.md) and the [open questions](docs/open-questions.md).

The design:

- [Glossary](docs/glossary.md): the words of broadcasts and meetings
- [Protocol layers](docs/protocols.md)
- [MoQ Transport state models](docs/moqt-state-models.md): the states of a session and of each request, which both implementations keep
- [MoQ Transport implementation notes](docs/moqt-implementation-notes.md): what both implementations do where draft 18 is silent or contradicts itself
- [Scaling a broadcast and meetings: a design draft](docs/scale-and-meetings.md)
- [Roadmap](docs/roadmap.md)
- The [experiment records](docs/experiments/README.md): what a run looks at, and every run

The parts, each with its scope, its flags and what it lacks:

- Browser packages: [`@xmoq/moqt`](packages/moqt/README.md) (MoQ Transport), [`@xmoq/loc`](packages/loc/README.md) (LOC), [`@xmoq/json`](packages/json/) (the payloads' JSON), [`@xmoq/tracks`](packages/tracks/) (a track kept on one session, the tracks a connection serves), [`@xmoq/msf`](packages/msf/README.md) (MSF catalogs), [`@xmoq/chat`](packages/chat/README.md), [`@xmoq/transcript`](packages/transcript/), [`@xmoq/media`](packages/media/README.md) (video and sound tracks), [`@xmoq/meeting`](packages/meeting/README.md), the room on a Durable Object, [`@xmoq/room`](packages/room/README.md), run with Miniflare, and [`@xmoq/room-client`](packages/room-client/), which asks it
- Rust programs: [`xmoq-relay`](crates/relay/README.md) (the relay), [`xmoq-watch`](crates/watch/README.md) (the viewer for the terminal), [`xmoq-agent`](crates/agent/README.md) (a room read for an agent), [`xmoq-chat`](crates/chat/README.md) (the chat's gatherer), [`xmoq-transcriber`](crates/transcriber/README.md) (the transcriber), [`xmoq-moqt-client`](crates/moqt-client/README.md) (the Rust client of the interop runs), [`xmoq-quiet-peer`](crates/quiet-peer/src/main.rs) (a peer of the interop runs that does not answer)
- Rust libraries: [`xmoq-moqt`](crates/moqt/README.md) (MoQ Transport, no I/O), [`xmoq-moqt-io`](crates/moqt-io/src/lib.rs) (its streams over WebTransport), [`xmoq-relay-core`](crates/relay-core/src/lib.rs) (what the relay decides), [`xmoq-loc`](crates/loc/src/lib.rs) (LOC), [`xmoq-msf`](crates/msf/src/lib.rs) (MSF catalogs, read), [`xmoq-room-token`](crates/room-token/src/lib.rs) (a room's token), [`xmoq-viewer`](crates/viewer/src/lib.rs) and [`xmoq-viewer-io`](crates/viewer-io/src/lib.rs) (what a viewer of a room decides, and does on its sockets), [`xmoq-picture`](crates/picture/README.md) (pictures of a video track), [`xmoq-speech`](crates/speech/README.md) (the words of an audio track), [`xmoq-again`](crates/again/src/lib.rs) (when a track is asked for again), [`xmoq-publisher`](crates/publisher/src/lib.rs) (serving tracks from a client), [`xmoq-subscriber`](crates/subscriber/src/lib.rs) (reading the tracks of others on a client's session), [`xmoq-chat-tracks`](crates/chat-tracks/README.md) and [`xmoq-transcript-tracks`](crates/transcript-tracks/src/lib.rs) (the payloads of the chat and of a transcript), [`xmoq-meeting-tracks`](crates/meeting-tracks/src/lib.rs)
- [The demo](demo/README.md), and the [interop runs against `moqtail` peers](interop/README.md) with [`moqlivemock`](interop/peers/moqlivemock/README.md)

Bytes and tables that more than one implementation reads in its tests are under [`fixtures/`](fixtures/), each directory with a README that says where they come from.

## Checks

Tool versions are pinned in `mise.toml` and `rust-toolchain.toml`. Run `mise install` once. The Rust build also needs a C and a C++ compiler, which mise does not install (for SQLite in `xmoq-chat` and OpenH264 in `xmoq-watch`). Then:

| Command | When |
|---|---|
| `mise run check` | Every check CI runs. A pull request's CI runs it; locally, run the checks of what was touched |
| `mise run fmt` | After formatting, and after a record or a driver of `interop/` is added: it writes the generated parts of the documents again |
| `mise run interop` | Not part of `check`. A pull request's CI runs all of it; locally, run the pairings of the parts touched with `--only` |

`mise tasks` lists every task with what it does, from the descriptions in `mise.toml`, which are written there alone; what a check holds to is in its script under `scripts/ci/`. A program's flags and their defaults are in its `USAGE`, or the doc of its `Args` where it has none; the pages' flags are in [the demo's README](demo/README.md).

CI (`.github/workflows/ci.yml`) runs the same tasks.
