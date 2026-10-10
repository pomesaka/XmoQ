<div align="center">
  <img src="docs/assets/xmoq-logo.svg" alt="XmoQ logo" width="160">
  <h1>XmoQ</h1>
  <p><strong>Experimental real-time media over QUIC.</strong></p>
</div>

> [!IMPORTANT]
> **Planning stage.** No working streaming implementation is claimed.

XmoQ explores one-to-many live streaming using MoQ/WebTransport, browser WebCodecs, a Rust relay and terminal CLI, and Cloudflare Durable Objects for application state. Future directions include bidirectional media and integration with events in a separately developed SNS; neither is committed for the MVP.

## Architecture at a glance

![Proposed XmoQ architecture](docs/assets/architecture.svg)

*Conceptual design — interoperability and integration are not yet validated.*

## Documentation

- [Vision](docs/vision.md)
- [Requirements](docs/requirements.md)
- [Architecture and broadcast flow](docs/architecture.md)
- [Protocol layers](docs/protocols.md)
- [MoQ library candidates](docs/moq-libraries.md)
- [Roadmap](docs/roadmap.md)
- [Validation](docs/validation.md)
- [Open questions](docs/open-questions.md)
- [Experiment records](docs/experiments/README.md)
- [Agent guide](docs/agent-guide.md)

License, deployment, supported platforms, codec profiles, authentication, and performance targets are undecided.
