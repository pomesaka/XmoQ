# Open decisions

Do not infer answers.

- When to move the MoQ implementation from draft 18 to a newer draft: **undecided**
- Codec/profile, Track/Object packaging, audio pipeline: **undecided**. Facts for the decision are in the [WebCodecs loopback record](experiments/2026-10-10-webcodecs-loopback.md) and, for the other browsers, in the [record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md). The parts to decide:
  - Video codec, and for H.264 or HEVC the bitstream format (`avc`/`hevc` with a `description`, or `annexb`)
  - How a late joiner gets the current `decoderConfig`, which WebCodecs emits only once
  - Key frame interval, and whether a receiver can ask for a key frame
  - What a receiver does when a chunk is missing (H.264 and VP8 decoders show corrupt pictures without an error)
  - `latencyMode` and `optimizeForLatency`; whether `duration` travels; whether resolution may change during a broadcast
  - Whether the Opus `description` is passed to the decoder
- Receive limits of the MoQ implementation (Object size, buffered bytes per stream, concurrent streams, key-value pairs per message, drain timeout after PUBLISH_DONE) and the limits of the relay's routing (announced namespaces per session, subscribers waiting for one upstream answer, requests open on one publisher, the time a publisher gets to answer) and of the Rust client (the time it waits for its session) : **provisional values in the code**, marked as such and listed in [implementation notes](moqt-implementation-notes.md). What happens when one is exceeded is decided and is in the same document
- Idle timeout of the relay: **6 seconds, provisional**, with a keep-alive every 3, so that a peer that vanished is noticed within 9 seconds. QUIC takes the smaller idle timeout of the two ends, so the clients time out after 6 seconds without a packet as well. Whether that is too short for a viewer on a poor link is to be measured on real links in Phase 1.
- What the relay counts a slow subscriber by: **per subscriber session today (4 MiB, 4096 Objects and 256 streams waiting to be written, over all the tracks the session subscribes to), provisional; to be decided in Phase 1**. The subscription whose stream reaches the limit is the one that ends, so one track can use up the session's allowance and get another track of the same session cut off: a video track with many short Subgroups could end the audio subscription of the same viewer. The choices are a count per subscription, per session as now, or both.
- How many sources one track may have at the relay: **4, provisional**. A PUBLISH beyond that is refused with `EXCESSIVE_LOAD`. Every source costs the relay a subscription and its streams, whether anything of it is passed on or not; what number a deployment needs depends on how many publishers send one track, which is not known.
- How long the relay remembers which stream a Subgroup was taken from: **the 8 newest Groups and 256 Subgroups of a track, provisional** ([implementation notes](moqt-implementation-notes.md)). The numbers decide how late a second copy of a Subgroup can come and still be recognised.
- Whether a Subgroup whose stream was cut short is continued from another source of the track: **not done; to be looked at in Phase 2 with measurements**. The draft allows it (§2.2: a second stream for a Subgroup is permitted when "one of the streams was reset prematurely"; §11.4.3: a relay "might attempt to recover the missing Objects"). Today the Subgroup ends for the subscribers where its stream was cut, and the next Subgroup is taken from whichever source sends it first.
- How a viewer who joins mid-broadcast gets its first picture: **undecided**, to be compared in Phase 1 with video. The draft (§5.1.3) offers waiting for the next Group, asking the publisher for a new Group (NEW_GROUP_REQUEST, which needs REQUEST_UPDATE), and fetching the current Group from a relay cache (needs FETCH and a cache). The relay keeps no cache for now
- Whether a slow subscriber should lose whole old Groups and stay subscribed (delivery timeout, §8 of the draft) instead of having its subscription ended: **to look at in Phase 2**, with measurements of how often subscriptions are ended
- Whether a browser client should end a subscription itself when its application does not read: **undecided**. The client holds back instead: it reads a bounded number of streams and leaves the rest to the transport, so that the relay ends the subscription (`TOO_FAR_BEHIND`). Facts are in the [backlog record](experiments/2026-10-10-subscription-backlog-in-chromium.md): the relay did end it in every shape of traffic tried, but with many streams of one large Object each the page was not told until it read again, and the answers to other requests of the same session, a SUBSCRIBE_OK among them, did not arrive either. Until this is decided, many streams of one large Object each (100,000 bytes in the runs) are a constraint on how Phase 1 cuts Objects into streams: with 30 such Objects a stream, and with Objects of 57 bytes, it did not happen
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
