# Architecture — proposed; the MoQ layer and a minimal relay are implemented

![XmoQ system architecture](assets/architecture.svg)

The diagram shows **intended component boundaries**, not a tested deployment. Solid arrows are proposed media paths; the dashed connection to Durable Objects is **undecided**.

## Broadcast flow

![Proposed publish/subscribe sequence](assets/broadcast-flow.svg)

This sequence is conceptual. Connection setup, session authentication, MoQ draft interoperability, codec and media timing details are **not yet validated**.

## Responsibilities

- **Browser:** capture, encode, subscribe, decode and render. TypeScript and WebCodecs.
- **Rust Relay:** media subscription and fan-out. The MoQ layer is implemented in this repository (see [protocol layers](protocols.md)). A minimal relay exists: it registers published tracks, answers subscriptions, forwards Objects to every subscriber and ends the subscription of a subscriber that falls too far behind. It keeps no cache and has no authorization. Deployment remains open.

  A slow subscriber has its own subscription ended (`TOO_FAR_BEHIND`), and the application can subscribe again and resume at the next key frame. The publisher is not made to wait, because one viewer that does not read would stall the broadcast for everyone. Queued Objects are not dropped one by one, because a missing delta frame makes H.264 and VP8 decoders show corrupt pictures without an error (see the [WebCodecs record](experiments/2026-10-10-webcodecs-loopback.md)).

  The relay is four crates. `xmoq-moqt` holds encoding and session state. `xmoq-relay-core` decides which Object goes to which subscriber, which publisher a SUBSCRIBE is passed on to, and what a disconnect ends; it takes events and returns actions and does no I/O, so it is tested without sockets. `xmoq-moqt-io` reads and writes MoQ streams over a `Connection` trait. `xmoq-relay` accepts sessions and connects the other three.
- **Rust CLI:** media subscription, video decoding, Ghostty Kitty Graphics rendering, native audio and chat. Backends and compatibility require testing.
- **Durable Objects:** proposed room/application state and chat persistence. Relay integration is not decided; direct WebTransport termination at DO is not assumed.

## Boundaries and constraints

Each client uses its own connection to the relay. MoQ Track is a logical data series, not a QUIC stream. WebCodecs does not provide a complete jitter buffer or A/V synchronization pipeline.

Future multiple publishers should not be structurally excluded, but bidirectional media is **not** an initial implementation requirement.

See [protocol layers](protocols.md), [validation](validation.md) and [open decisions](open-questions.md).
