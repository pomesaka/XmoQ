<div align="center">
  <img src="docs/assets/xmoq-logo.svg" alt="XmoQ logo" width="160">
  <h1>XmoQ</h1>
  <p><strong>Experimental real-time media over QUIC.</strong></p>
</div>

> [!IMPORTANT]
> **Early implementation, on one machine.** An own MoQ Transport (draft 18) in Rust and TypeScript and a relay in Rust carry a page's video and sound to other pages and to a terminal viewer, with a chat and captions, and a CLI lets an agent read a broadcast or a meeting. Nothing has run over a real network or on Cloudflare. What is done and what is open is in the [requirements](docs/requirements.md).

XmoQ explores one-to-many live streaming using MoQ/WebTransport, browser WebCodecs, a Rust relay and terminal CLI, and Cloudflare Durable Objects for application state. Its purpose is to let an agent such as Claude take part in a broadcast, and later in a meeting of up to 49 people: a CLI subscribes and writes what the tracks carry where the agent reads it ([vision](docs/vision.md)). Integration with events in a separately developed SNS is a future direction, not committed.

## Architecture at a glance

![XmoQ architecture](docs/assets/architecture.svg)

*The intended components. What of it is built is in the [architecture](docs/architecture.md).*

## Documentation

Start with the [vision](docs/vision.md), what is done and what is open ([requirements](docs/requirements.md)), the [architecture](docs/architecture.md) and the [open questions](docs/open-questions.md). An agent that implements reads the [agent guide](docs/agent-guide.md) first.

The design:

- [Protocol layers](docs/protocols.md)
- [MoQ Transport implementation notes](docs/moqt-implementation-notes.md): what both implementations do where draft 18 is silent or contradicts itself
- [Scaling a broadcast and meetings: a design draft](docs/scale-and-meetings.md)
- [The audio and video demo: what it does and where it ends](docs/av-demo.md)
- [Roadmap](docs/roadmap.md)
- [Validation](docs/validation.md) and the [experiment records](docs/experiments/README.md)
- [MoQ library candidates](docs/moq-libraries.md), the survey of Phase 0

The parts, each with its scope, its flags and what it lacks:

- Browser packages: [`@xmoq/moqt`](packages/moqt/README.md) (MoQ Transport), [`@xmoq/loc`](packages/loc/README.md) (LOC), [`@xmoq/json`](packages/json/README.md) (the payloads' JSON), [`@xmoq/msf`](packages/msf/README.md) (MSF catalogs), [`@xmoq/chat`](packages/chat/README.md), [`@xmoq/transcript`](packages/transcript/README.md), [`@xmoq/media`](packages/media/README.md) (video and sound tracks), [`@xmoq/meeting`](packages/meeting/README.md)
- Rust programs: [`xmoq-relay`](crates/relay/README.md) (the relay), [`xmoq-watch`](crates/watch/README.md) (the viewer for the terminal), [`xmoq-agent`](crates/agent/README.md) (a room read for an agent), [`xmoq-chat`](crates/chat/README.md) (the chat's gatherer), [`xmoq-transcriber`](crates/transcriber/README.md) (the transcriber), [`xmoq-moqt-client`](crates/moqt-client/README.md) (the Rust client of the interop runs), [`xmoq-quiet-peer`](crates/quiet-peer/src/main.rs) (a peer of the interop runs that does not answer)
- Rust libraries: [`xmoq-moqt`](crates/moqt/README.md) (MoQ Transport, no I/O), [`xmoq-moqt-io`](crates/moqt-io/src/lib.rs) (its streams over WebTransport), [`xmoq-relay-core`](crates/relay-core/src/lib.rs) (what the relay decides), [`xmoq-loc`](crates/loc/src/lib.rs) (LOC), [`xmoq-msf`](crates/msf/src/lib.rs) (MSF catalogs, read), [`xmoq-room-token`](crates/room-token/src/lib.rs) (a room's token), [`xmoq-viewer`](crates/viewer/src/lib.rs) and [`xmoq-viewer-io`](crates/viewer-io/src/lib.rs) (what a viewer of a room decides, and does on its sockets), [`xmoq-picture`](crates/picture/README.md) (pictures of a video track), [`xmoq-speech`](crates/speech/README.md) (the words of an audio track), [`xmoq-again`](crates/again/README.md) (when a track is asked for again), [`xmoq-track-serving`](crates/track-serving/README.md) (serving tracks from a client), [`xmoq-chat-tracks`](crates/chat-tracks/README.md) and [`xmoq-transcript-tracks`](crates/transcript-tracks/README.md) (the payloads of the chat and of a transcript)
- [The demo](demo/README.md), and the [interop runs against `moqtail` peers](interop/README.md) with [`moqlivemock`](interop/peers/moqlivemock/README.md)

Bytes and tables that more than one implementation reads in its tests (`fixtures/`):

- Captured on the wire: [from `moqtail`](fixtures/moqt-18/README.md), [Chromium closing a session](fixtures/webtransport-close/README.md), [a LOC video stream](fixtures/loc-04/README.md), [MSF catalogs](fixtures/msf-01/README.md)
- Built by another library in a script: [`moqt-18-from-libraries`](fixtures/moqt-18-from-libraries/README.md); written by hand from the draft: [`moqt-18-by-hand`](fixtures/moqt-18-by-hand/README.md)
- What both implementations do in the same case: [first bytes of a stream](fixtures/first-bytes/README.md), [a SUBSCRIBE of the peer](fixtures/peer-subscribe/README.md), [a FETCH of the peer](fixtures/peer-fetch/README.md), [the answers to a subscriber's REQUEST_UPDATEs](fixtures/update-answers/README.md), [a REQUEST_ERROR with a Redirect](fixtures/redirect/README.md), [the SUBSCRIBE of a Joining FETCH](fixtures/joining-request-id/README.md), [a FETCH answered from what was published](fixtures/published-fetch/README.md), [Objects before SUBSCRIBE_OK](fixtures/early-objects/README.md), [Immutable Properties](fixtures/immutable-properties/README.md), [the wait for an answer](fixtures/request-answer-wait/README.md), [the Retry Interval](fixtures/retry-interval/README.md), [when a client asks again](fixtures/ask-again/README.md), [playout](fixtures/playout/README.md), [the chat's history](fixtures/chat-history/README.md), [who is in a meeting](fixtures/meeting-presence/README.md), [the lines an agent reads](fixtures/agent-lines/README.md)
- Payloads of the tracks: [the chat's](fixtures/chat-tracks/README.md), [a transcript's](fixtures/transcript-tracks/README.md); [a voice for the transcriber's interop run](fixtures/speech/README.md)

## Checks

Tool versions are pinned in `mise.toml` and `rust-toolchain.toml`. Run `mise install` once. The Rust build also needs a C compiler, which mise does not install: `xmoq-chat` builds SQLite from source (on macOS the Command Line Tools' `clang`, on Linux `gcc`), and `xmoq-watch` builds OpenH264 with a C++ compiler. Then:

| Command | What it runs |
|---|---|
| `mise run check` | Everything CI runs |
| `mise run check:rust` | `cargo fmt --check`, `cargo clippy` with warnings as errors, `cargo test` |
| `mise run check:ts` | Biome (format and lint), oxlint (`as` assertions), `typecheck` for packages and `interop/`, `bun test` per package |
| `mise run check:layers` | Dependency rules: the layers of the packages (`package_layer` in `scripts/ci/check-layers.sh`), `xmoq-moqt` free of I/O crates, nothing depending on `spikes/`, packages not depending on `interop/` |
| `mise run check:workflows` | `actionlint` and `shellcheck` |
| `mise run check:docs` | The index of `docs/experiments/` and the list of files of `interop/` are what `scripts/ci/experiments-index.sh` and `scripts/ci/interop-index.sh` write (`mise run fmt` writes them) |
| `mise run interop` | Builds the `moqtail` relay and the Rust clients, then moves an Object through each relay for every pairing of own and `moqtail` clients, and runs the scenarios in which a client leaves through the own relay, in a real Chromium. Not part of `check` |
| `mise run fmt` | Apply formatting and safe fixes, and write the generated parts of the documents again (the index of `docs/experiments/` and the list of files of `interop/`) |

The tasks that run a part (the demo, the terminal viewer, the chat's gatherer, the transcriber and its model, the reader for an agent and its demo) are listed with a line of use each by `mise tasks`; their flags are in the part's README (above).

The relay's flags are in [its README](crates/relay/README.md), the pages' and `demo/serve.ts`'s in [the demo's README](demo/README.md), the terminal viewer's in `USAGE` of `crates/watch/src/args.rs`.

CI (`.github/workflows/ci.yml`) runs the same tasks. A task whose target does not exist yet prints `SKIP` and succeeds; `spikes/` is outside every check, and `interop/peers/`, copies of `spikes/moqtail/` written with the `moqtail` library, is outside lint and formatting. The interop run is a CI job of its own, which calls `mise run interop`; for a pull request that touches nothing the run depends on, the job reports that it skipped the pairings.

What is undecided is in the [open questions](docs/open-questions.md).
