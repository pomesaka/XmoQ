# Open decisions

Do not infer answers.

- When to move the MoQ implementation from draft 18 to a newer draft: **undecided**
- Codec/profile, Track/Object packaging, audio pipeline: **undecided**. Facts for the decision are in the [WebCodecs loopback record](experiments/2026-10-10-webcodecs-loopback.md). The parts to decide:
  - Video codec, and for H.264 or HEVC the bitstream format (`avc`/`hevc` with a `description`, or `annexb`)
  - How a late joiner gets the current `decoderConfig`, which WebCodecs emits only once
  - Key frame interval, and whether a receiver can ask for a key frame
  - What a receiver does when a chunk is missing (H.264 and VP8 decoders show corrupt pictures without an error)
  - `latencyMode` and `optimizeForLatency`; whether `duration` travels; whether resolution may change during a broadcast
  - Whether the Opus `description` is passed to the decoder
- Receive limits of the MoQ implementation (Object size, buffered bytes per stream, concurrent streams, key-value pairs per message, drain timeout after PUBLISH_DONE) and what happens when one is exceeded: **provisional values in the code**, marked as such. The behaviour on overflow is in [implementation notes](moqt-implementation-notes.md)
- Whether the relay caches Objects for late joiners, and how much: **undecided**; the first relay keeps none
- Whether the relay accepts native QUIC as well as WebTransport: **undecided**; the first relay accepts WebTransport only
- Authorization token handling in MoQ messages (§10.2.2 of the draft): **undecided**, follows the authentication decision
- Latency goal, room size, expected concurrent rooms: **undecided**
- Supported browsers, operating systems, Ghostty versions: **undecided**
- CLI decoder/audio backend: **undecided**
- Chat via MoQ Track or other channel, persistence and ordering: **undecided**
- Relay and Durable Objects integration: **to design**
- Authentication/SSO, authorization, privacy and moderation: **undecided**
- Hosting, TLS, monitoring and operating costs: **undecided**
- Recording, replay, captions and accessibility: **not discussed**
- SNS event integration contract: **not discussed**
- Bidirectional requirements: **future possibility**
- OSS license and contribution rules: **undecided**

When an item is decided, remove it from this list. Record context, alternatives, decision, consequences and evidence where the decision takes effect: as WHY / WHY NOT comments in the code, or in the document that covers the topic. Decisions are not collected in a separate place.
