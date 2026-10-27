# Protocols and layers

![XmoQ protocol layers](assets/protocol-layers.svg)

The layers of the browser's media path. They run together in the demo, on one machine.

MoQ is a media Object distribution protocol, not a replacement for the entire WebRTC media stack. One WebTransport session can multiplex streams and datagrams. A MoQ Track is **not** the same thing as a QUIC stream.

## MoQ implementation — decided, partly implemented

XmoQ implements the MoQ Transport layer itself, in Rust (relay and client) and in TypeScript (browser), pinned to `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`). QUIC and WebTransport are not reimplemented: Rust uses existing crates and the browser uses its native `WebTransport`.

What of the draft is implemented, and what is not, and what both implementations do where draft 18 is silent or contradicts itself, is in the [implementation notes](moqt-implementation-notes.md). The implementation follows the draft text, not library behaviour and not model memory.

The relay accepts WebTransport only, not native QUIC. Browsers can use nothing else, the Rust CLI can use WebTransport as well, and one path means one implementation and one set of interop runs.

The Rust side uses `web-transport-quinn` 0.13.2 for WebTransport, with `ring` as its crypto backend. Why it and not `wtransport` is at the top of `crates/moqt-io/src/wt.rs`, the one file that touches it. Records: the [comparison](experiments/2026-10-10-webtransport-library-comparison.md) and the [switch](experiments/2026-10-10-webtransport-library-switch.md).

- **Why own implementation:** the goal of the project is to understand and own the protocol. The Phase 0 runs also showed limits in each library: `moq-dev/moq` keeps the moq-lite model in MOQT mode with deliberate deviations and failed most browser-to-browser late joins; the `moqtail` crate has no client API, so a Rust client already has to drive SETUP and request streams itself.
- **Why not moq-lite:** it is an individual draft with its own wire format, not IETF MoQ Transport.
- **Why draft 18:** it is the version that passed in Phase 0 with both `moqtail` and `moq-dev/moq`, and the current target of the cross-implementation interop runner. Draft 22 was the newest when the libraries were surveyed ([survey](experiments/2026-10-10-moq-library-survey.md)), but the published `moq-dev/moq` packages encoded it as draft 21, so no tested peer for it exists here.
- **How conformance is checked:** the `moqtail` relay and browser client from Phase 0 stay as interop peers. Writing both ends alone would let the same misreading pass on both sides. Each own component is first run against a `moqtail` peer.

## Streaming format: decided, the catalog partly implemented

Tracks are described by a catalog of `draft-ietf-moq-msf-01` (MSF), and media Objects are packaged as LOC says. `@xmoq/msf` reads, follows and publishes a catalog track; what of MSF-01 it does not do is in [its README](../packages/msf/README.md). WHY NOT only the shape of LOC and MSF, with a fixed track name and no catalog: it would have started sooner, and would leave the players of other MoQ implementations unable to find the tracks ([survey](experiments/2026-10-10-loc-msf-fetch-survey.md)).

- **The layer above MoQ Transport, in a package of its own.** `@xmoq/msf` may use `@xmoq/moqt`, never the other way round, and the part that reads and checks a catalog uses neither a connection nor `@xmoq/moqt`. A relay does not read a catalog (MSF-01 §5: "The payload of the Catalog object is opaque to Relays"), so of the Rust crates only the clients' `xmoq-msf` reads one. It keeps less of a catalog than `@xmoq/msf`; both read the table of hand-written catalogs (`fixtures/msf-01/catalogs.tsv`). Where MSF-01 leaves a choice or contradicts itself, what the two readers do is in [`fixtures/msf-01/`](../fixtures/msf-01/README.md), "How the draft is read".
- **The revision of LOC is LOC-04, with its property numbers on transport draft 18.** No document names that pair: MSF-01 cites LOC-02, whose Timestamp is a number draft 18 uses for something else, and LOC-04 cites draft 19. Why, and why not the others, is at the top of the [README of `@xmoq/loc`](../packages/loc/README.md). The catalog names the packaging with the string `"loc"` and carries no property number.
- **LOC is `@xmoq/loc` and, in Rust, `xmoq-loc`**, which depend on no other part of XmoQ. What is of the revision is in a directory or module with its name (`src/draft-04/`, `draft_04`), so that another revision is a small change. Both read the payloads of `fixtures/loc-04/`.
- **A video track is sent with a Group on one stream** (Subgroup 0; a Group begins with a key frame). That leaves the MUST of MSF-01 §6 for a stream for every Object, on purpose; why, and where that sentence stands in its working group, is at `DEFAULT_MAPPING` in `packages/media/src/core/video-sender.ts`. The receiver reads both.
- **The video codec is H.264, in Annex B format.** WHY H.264: in Chromium it is the one codec with a hardware encoder, which took about a tenth of the CPU time of the software encoders at 1280x720 and 30 frames a second (4% of a core, against 33% for VP8, 54% for VP9, 55% for AV1 and 71% for H.264 in software), and where no hardware decoder is used it was the cheapest to decode ([record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md)). WHY Annex B and not `avc` with a `description`: every key chunk carries the parameter sets, so a viewer that joins late decodes from the next key frame with the codec string alone, where WebCodecs emits the `description` once ([WebCodecs loopback record](experiments/2026-10-10-webcodecs-loopback.md)). WHY NOT VP9: it has no choice of format to get wrong and its decoder reports a missing frame, which H.264 decoders do not (they show corrupt pictures without an error), but it costs over ten times the CPU time to encode. What the choice rests on is one Mac, a generated picture and CPU time, not power or picture quality. The catalog names H.264 `avc3`.
- **The audio codec is Opus**: it is the one audio codec Chromium, Firefox and Safari all encode and decode ([record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md)).

## Not decided

What of the protocols is open (the details of the codec, a newer draft, what the relay and the room say to each other, the CLI's audio output) is in the [open questions](open-questions.md).

## Primary specifications

- [IETF MoQ](https://datatracker.ietf.org/doc/draft-ietf-moq-transport/)
- [MOQT Streaming Format (MSF)](https://datatracker.ietf.org/doc/draft-ietf-moq-msf/)
- [Low Overhead Media Container (LOC)](https://datatracker.ietf.org/doc/draft-ietf-moq-loc/)
- [W3C WebTransport](https://www.w3.org/TR/webtransport/)
- [W3C WebCodecs](https://www.w3.org/TR/webcodecs/)
- [Cloudflare Durable Objects](https://developers.cloudflare.com/durable-objects/)
