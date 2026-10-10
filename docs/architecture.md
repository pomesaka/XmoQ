# Architecture — proposed, not implemented

![XmoQ system architecture](assets/architecture.svg)

The diagram shows **intended component boundaries**, not a tested deployment. Solid arrows are proposed media paths; the dashed connection to Durable Objects is **undecided**.

## Broadcast flow

![Proposed publish/subscribe sequence](assets/broadcast-flow.svg)

This sequence is conceptual. Connection setup, session authentication, MoQ draft interoperability, codec and media timing details are **not yet validated**.

## Responsibilities

- **Browser:** capture, encode, subscribe, decode and render. TypeScript and WebCodecs.
- **Rust Relay:** media subscription and fan-out. The MoQ layer is implemented in this repository (see [protocol layers](protocols.md)). Buffering, slow subscriber policy and deployment remain open.
- **Rust CLI:** media subscription, video decoding, Ghostty Kitty Graphics rendering, native audio and chat. Backends and compatibility require testing.
- **Durable Objects:** proposed room/application state and chat persistence. Relay integration is not decided; direct WebTransport termination at DO is not assumed.

## Boundaries and constraints

Each client uses its own connection to the relay. MoQ Track is a logical data series, not a QUIC stream. WebCodecs does not provide a complete jitter buffer or A/V synchronization pipeline.

Future multiple publishers should not be structurally excluded, but bidirectional media is **not** an initial implementation requirement.

See [protocol layers](protocols.md), [validation](validation.md) and [open decisions](open-questions.md).
