<div align="center">
  <img src="docs/assets/xmoq-logo.svg" alt="XmoQ logo" width="160">
  <h1>XmoQ</h1>
  <p><strong>Experimental real-time media over QUIC.</strong></p>
</div>

> [!IMPORTANT]
> **Early implementation.** No working streaming implementation is claimed. No media has been sent. What exists is an own MoQ Transport implementation: a client in Rust and in TypeScript and a minimal relay in Rust, which exchange single Objects with each other and with `moqtail` peers.

XmoQ explores one-to-many live streaming using MoQ/WebTransport, browser WebCodecs, a Rust relay and terminal CLI, and Cloudflare Durable Objects for application state. Future directions include bidirectional media and integration with events in a separately developed SNS; neither is committed for the MVP.

## Architecture at a glance

![Proposed XmoQ architecture](docs/assets/architecture.svg)

*Conceptual design — interoperability and integration are not yet validated.*

## Documentation

- [Vision](docs/vision.md)
- [Requirements](docs/requirements.md)
- [Architecture and broadcast flow](docs/architecture.md)
- [Protocol layers](docs/protocols.md)
- [MoQ Transport implementation notes](docs/moqt-implementation-notes.md)
- [Wire fixtures captured from moqtail](fixtures/moqt-18/README.md)
- [Bytes written by hand from the draft, read by both implementations](fixtures/moqt-18-by-hand/README.md)
- [The points at which a stream of the peer has said what it is](fixtures/first-bytes/README.md)
- [What becomes of a SUBSCRIBE the peer sends](fixtures/peer-subscribe/README.md)
- [What becomes of a FETCH the peer sends](fixtures/peer-fetch/README.md)
- [Whether the SUBSCRIBE of a Joining FETCH is yet to come](fixtures/joining-request-id/README.md)
- [What a publisher answers a FETCH range with, from what it published](fixtures/published-fetch/README.md)
- [Catalogs of MSF, from another implementation and written by hand](fixtures/msf-01/README.md)
- [A LOC video stream of another implementation](fixtures/loc-04/README.md)
- [Browser client library `@xmoq/moqt`](packages/moqt/README.md)
- [The Low Overhead Media Container in TypeScript, `@xmoq/loc`](packages/loc/README.md)
- [The MOQT Streaming Format in TypeScript, `@xmoq/msf`](packages/msf/README.md)
- [Between a video track and a decoder, `@xmoq/media`](packages/media/README.md)
- [The viewer for the terminal, `xmoq-watch`](crates/watch/README.md)
- [Interop runs against moqtail peers](interop/README.md)
- [MoQ library candidates](docs/moq-libraries.md)
- [The audio and video demo: what it does and where it ends](docs/av-demo.md)
- [Roadmap](docs/roadmap.md)
- [Validation](docs/validation.md)
- [Open questions](docs/open-questions.md)
- [Experiment records](docs/experiments/README.md)
- [Agent guide](docs/agent-guide.md)

## Checks

Tool versions are pinned in `mise.toml` and `rust-toolchain.toml`. Run `mise install` once, then:

| Command | What it runs |
|---|---|
| `mise run check` | Everything CI runs |
| `mise run check:rust` | `cargo fmt --check`, `cargo clippy` with warnings as errors, `cargo test` |
| `mise run check:ts` | Biome (format and lint), oxlint (`as` assertions), `typecheck` for packages and `interop/`, `bun test` per package |
| `mise run check:layers` | Dependency rules: `xmoq-moqt` stays free of I/O crates, nothing depends on `spikes/`, packages do not depend on `interop/` |
| `mise run check:workflows` | `actionlint` and `shellcheck` |
| `mise run demo` | Starts the own relay and a web server; a page publishes video (a painted clock or the camera) and other pages watch it, in Chromium. See [`demo/README.md`](demo/README.md). Outside lint and type checking |
| `mise run watch -- <flags>` | The viewer for the terminal, `xmoq-watch`: the picture of a broadcast in Ghostty or kitty, its sound, and the chat of its room. `-- --room http://127.0.0.1:8787/rooms/demo` next to `mise run demo`. See [`crates/watch/README.md`](crates/watch/README.md) |
| `mise run interop` | Builds the `moqtail` relay and the Rust clients, then moves an Object through each relay for every pairing of own and `moqtail` clients, and runs the scenarios in which a client leaves through the own relay, in a real Chromium. Not part of `check` |
| `mise run fmt` | Apply formatting and safe fixes |

CI (`.github/workflows/ci.yml`) runs the same tasks. A task whose target does not exist yet prints `SKIP` and succeeds; `spikes/` is outside every check, and so are the copies of `moqtail` code under `interop/peers/`. The interop run is a CI job of its own, which calls `mise run interop`; for a pull request that touches nothing the run depends on, the job reports that it skipped the pairings.

License, deployment, supported platforms, codec profiles, authentication, and performance targets are undecided.
