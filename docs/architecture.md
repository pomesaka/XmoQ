# Architecture — proposed, not implemented

```text
Browser publisher (TS, WebCodecs)
             |
      MoQ / WebTransport
             |
         Rust relay
        /          \
Browser viewer    Rust CLI viewer
(TS, WebCodecs)   (decoder, audio, Kitty Graphics)

Durable Objects (room state / chat)
Integration with relay: TBD
```

## Responsibilities
- Browser: capture, encode, subscribe, decode, render.
- Relay: subscriptions and media fan-out; bounded buffering and slow subscribers need design.
- CLI: media subscription, decoding, terminal rendering and native audio; backends undecided.
- Durable Objects: proposed room state and chat persistence; transport integration undecided.

## Boundaries
Each client connects independently to the relay. MoQ Track is a logical publish/subscribe series, **not** a QUIC stream. WebTransport multiplexes streams and datagrams within a session. MoQ sessions have defined message formats; do not mix arbitrary custom streams without an interoperability design.

WebCodecs is not a complete player: jitter buffering, A/V sync, late join and recovery need implementation. Direct WebTransport termination at Durable Objects is **not assumed**.

Future multi-publisher support should not be structurally precluded, but is not an MVP feature.
