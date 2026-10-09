# Protocols and layers

```text
Application: video / audio / chat
Media processing: codec, jitter buffer, A/V synchronization
MoQ: Track, Group, Object, publish / subscribe
WebTransport: session, streams, datagrams
HTTP/3
QUIC: multiplexing, encryption, congestion control
UDP/IP
```

MoQ is a media object distribution protocol, not a replacement for the full WebRTC media stack. One WebTransport session can contain multiple streams; one MoQ Track does not necessarily correspond to one stream.

## Discussed
- MoQ over WebTransport for browser streaming.
- WebCodecs for browser media processing.
- Rust relay and CLI.

## Not decided / to validate
- Rust and browser MoQ implementations, draft versions and interoperability.
- Codec/profile, media packaging and synchronization.
- Native CLI transport and media backends.
- Chat as MoQ Track vs separate transport; WebSocket is not required by decision.
- Reactions delivery semantics and persistence.
- Relay ↔ Durable Objects protocol.

## Primary specifications
- [IETF MoQ](https://datatracker.ietf.org/doc/draft-ietf-moq-transport/)
- [WebTransport](https://www.w3.org/TR/webtransport/)
- [WebCodecs](https://www.w3.org/TR/webcodecs/)
- [Durable Objects](https://developers.cloudflare.com/durable-objects/)
