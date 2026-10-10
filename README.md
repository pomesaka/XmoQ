<div align="center">
  <img src="docs/assets/xmoq-logo.svg" alt="XmoQ logo" width="160">
  <h1>XmoQ</h1>
  <p><strong>Experimental real-time media over QUIC.</strong></p>
</div>

> [!IMPORTANT]
> **Early implementation.** No working streaming implementation is claimed. What exists is a MoQ Transport codec and a one-shot client that exchange single Objects with `moqtail` peers.

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
- [MoQ library candidates](docs/moq-libraries.md)
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
| `mise run check:ts` | Biome (format and lint), oxlint (`as` assertions), `typecheck` and `bun test` per package |
| `mise run check:layers` | Dependency rules: `xmoq-moqt` stays free of I/O crates, nothing depends on `spikes/` |
| `mise run check:workflows` | `actionlint` and `shellcheck` |
| `mise run fmt` | Apply formatting and safe fixes |

CI (`.github/workflows/ci.yml`) runs the same tasks. A task whose target does not exist yet prints `SKIP` and succeeds; `spikes/` is outside every check.

License, deployment, supported platforms, codec profiles, authentication, and performance targets are undecided.
