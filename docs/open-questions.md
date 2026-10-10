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
- Receive limits of the MoQ implementation (Object size, buffered bytes per stream, concurrent streams, key-value pairs per message, drain timeout after PUBLISH_DONE) and the limits of the relay's routing (announced namespaces per session, subscribers waiting for one upstream answer, requests open on one publisher, the time a publisher gets to answer) and of the Rust client (the time it waits for its session) : **provisional values in the code**, marked as such and listed in [implementation notes](moqt-implementation-notes.md). What happens when one is exceeded is decided and is in the same document
- How a viewer who joins mid-broadcast gets its first picture: **undecided**, to be compared in Phase 1 with video. The draft (§5.1.3) offers waiting for the next Group, asking the publisher for a new Group (NEW_GROUP_REQUEST, which needs REQUEST_UPDATE), and fetching the current Group from a relay cache (needs FETCH and a cache). The relay keeps no cache for now
- Whether a slow subscriber should lose whole old Groups and stay subscribed (delivery timeout, §8 of the draft) instead of having its subscription ended: **to look at in Phase 2**, with measurements of how often subscriptions are ended
- Messages the relay does not accept, and what each waits for: REQUEST_UPDATE (the late-join decision above), GOAWAY (hosting), SUBSCRIBE_NAMESPACE (whether rooms are discovered through MoQ or through Durable Objects), TRACK_STATUS (no use planned)
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
