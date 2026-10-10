# Protocols and layers

![XmoQ protocol layers](assets/protocol-layers.svg)

This is the **proposed browser media path**, not a claim that all layers have been integrated.

MoQ is a media Object distribution protocol, not a replacement for the entire WebRTC media stack. One WebTransport session can multiplex streams and datagrams. A MoQ Track is **not** the same thing as a QUIC stream.

## MoQ implementation — decided, not implemented

XmoQ implements the MoQ Transport layer itself, in Rust (relay and client) and in TypeScript (browser), pinned to `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`). QUIC and WebTransport are not reimplemented: Rust uses existing crates and the browser uses its native `WebTransport`.

- **Why own implementation:** the goal of the project is to understand and own the protocol. The Phase 0 runs also showed limits in each library: `moq-dev/moq` keeps the moq-lite model in MOQT mode with deliberate deviations and failed most browser-to-browser late joins; the `moqtail` crate has no client API, so a Rust client already has to drive SETUP and request streams itself.
- **Why not moq-lite:** it is an individual draft with its own wire format, not IETF MoQ Transport.
- **Why draft 18:** it is the version that passed in Phase 0 with both `moqtail` and `moq-dev/moq`, and the current target of the cross-implementation interop runner. Draft 22 is current, but the published `moq-dev/moq` packages encode it as draft 21, so no tested peer for it exists here.
- **How conformance is checked:** the `moqtail` relay and browser client from Phase 0 stay as interop peers. Writing both ends alone would let the same misreading pass on both sides. Each own component is first run against a `moqtail` peer.

The libraries that were considered, and what each Phase 0 run showed, are in [MoQ library candidates](moq-libraries.md).

The implementation follows the draft text, not library behaviour and not model memory. The scope starts with SETUP, PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE and subgroup streams; other messages are added when a roadmap item needs them.

## Discussed

- MoQ over WebTransport for browser streaming.
- WebCodecs for browser media processing.
- Rust relay and terminal client.

## Undecided or requiring validation

- Codec, media packaging, keyframe handling and A/V synchronization.
- Native CLI transport, decoder and audio output.
- Chat as MoQ Track versus a different application transport.
- Reactions semantics and persistence.
- Relay ↔ Durable Objects protocol.

## Primary specifications

- [IETF MoQ](https://datatracker.ietf.org/doc/draft-ietf-moq-transport/)
- [W3C WebTransport](https://www.w3.org/TR/webtransport/)
- [W3C WebCodecs](https://www.w3.org/TR/webcodecs/)
- [Cloudflare Durable Objects](https://developers.cloudflare.com/durable-objects/)
