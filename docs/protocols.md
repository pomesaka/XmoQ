# Protocols and layers

![XmoQ protocol layers](assets/protocol-layers.svg)

This is the **proposed browser media path**, not a claim that all layers have been integrated.

MoQ is a media Object distribution protocol, not a replacement for the entire WebRTC media stack. One WebTransport session can multiplex streams and datagrams. A MoQ Track is **not** the same thing as a QUIC stream.

## Discussed

- MoQ over WebTransport for browser streaming.
- WebCodecs for browser media processing.
- Rust relay and terminal client.

## Undecided or requiring validation

- Rust/browser MoQ implementations, draft version and interoperability.
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
