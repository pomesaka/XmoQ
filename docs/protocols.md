# Protocols and layers

![XmoQ protocol layers](assets/protocol-layers.svg)

The layers of the browser's media path. They run together in the demo, on one machine.

MoQ is a media Object distribution protocol, not a replacement for the entire WebRTC media stack. One WebTransport session can multiplex streams and datagrams. A MoQ Track is **not** the same thing as a QUIC stream.

## MoQ implementation — decided, partly implemented

XmoQ implements the MoQ Transport layer itself, in Rust (relay and client) and in TypeScript (browser), pinned to `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`). QUIC and WebTransport are not reimplemented: Rust uses existing crates and the browser uses its native `WebTransport`.

What of the draft is implemented, and what is not, is in the [implementation notes](moqt-implementation-notes.md).

The relay accepts WebTransport only, not native QUIC. Browsers can use nothing else, the Rust CLI can use WebTransport as well, and one path means one implementation and one set of interop runs.

The Rust side uses `web-transport-quinn` 0.13.2 for WebTransport (the latest release on crates.io when read on 2026-10-10), with `ring` as its crypto backend. Everything that touches it is in one file of `crates/moqt-io`, behind a `Connection` trait whose contract is checked by the same tests on an in-memory connection and on a real loopback connection. Records: the [comparison](experiments/2026-10-10-webtransport-library-comparison.md) and the [switch](experiments/2026-10-10-webtransport-library-switch.md).

- **Why `web-transport-quinn`:** a viewer has to be told apart "the broadcast ended", "the server refused" and "the connection broke". That needs the Session Termination Error Code and reason of a session the relay closes to reach the browser, and this library sends them (CLOSE_WEBTRANSPORT_SESSION); Chromium 156 read both from the own relay. It also maps stream error codes into the WebTransport range, so no mapping is kept in XmoQ.
- **Why not `wtransport` 0.7.2,** which the relay was first written on and the `moqtail` peers use: it cannot send CLOSE_WEBTRANSPORT_SESSION, so a browser sees "Connection lost." without code or reason, and no issue or pull request upstream asks for it. It does not map stream codes either; the pull request that would has not moved since 2024-09.
- **What the switch cost:** a stream that has arrived and was not accepted yet when the connection closes can be lost. `wtransport` handed such a stream over; this library returns the connection error first (`poll_accept_uni` and `poll_accept_bi`, `src/session.rs`). So "the peer's transport has it" is not "the peer will read it", and a publisher that closes right after its last stream cannot rely on the transport. XmoQ answers this one level up: after PUBLISH_DONE the relay ends its direction of the request stream with a FIN once every stream the Stream Count named has ended, and the publisher waits for that before closing (see the [implementation notes](moqt-implementation-notes.md)).
- **What stays as it was:** the `moqtail` peers do not map stream codes. A code they send arrives without a code on the XmoQ side, and a code XmoQ sends arrives at them as the mapped number. The interop runs pass with that.

Where draft 18 is silent or contradicts itself, both implementations follow [MoQ Transport implementation notes](moqt-implementation-notes.md).

- **Why own implementation:** the goal of the project is to understand and own the protocol. The Phase 0 runs also showed limits in each library: `moq-dev/moq` keeps the moq-lite model in MOQT mode with deliberate deviations and failed most browser-to-browser late joins; the `moqtail` crate has no client API, so a Rust client already has to drive SETUP and request streams itself.
- **Why not moq-lite:** it is an individual draft with its own wire format, not IETF MoQ Transport.
- **Why draft 18:** it is the version that passed in Phase 0 with both `moqtail` and `moq-dev/moq`, and the current target of the cross-implementation interop runner. Draft 22 was the newest when the libraries were surveyed on 2026-10-10 ([survey](moq-libraries.md)), but the published `moq-dev/moq` packages encoded it as draft 21, so no tested peer for it exists here.
- **How conformance is checked:** the `moqtail` relay and browser client from Phase 0 stay as interop peers. Writing both ends alone would let the same misreading pass on both sides. Each own component is first run against a `moqtail` peer.

The libraries that were considered, and what each Phase 0 run showed, are in [MoQ library candidates](moq-libraries.md).

The implementation follows the draft text, not library behaviour and not model memory. What each implementation does and does not do of the draft is in the [implementation notes](moqt-implementation-notes.md).

## Streaming format: decided, the catalog partly implemented

Tracks are described by a catalog of `draft-ietf-moq-msf-01` (MSF), and media Objects are packaged as LOC says. `@xmoq/msf` reads, follows and publishes a catalog track; what of MSF-01 it does not do is in [its README](../packages/msf/README.md).

- **The layer above MoQ Transport, in a package of its own.** `@xmoq/msf` may use `@xmoq/moqt`, never the other way round, and the part that reads and checks a catalog uses neither a connection nor `@xmoq/moqt`. A relay does not read a catalog (MSF-01 §5: "The payload of the Catalog object is opaque to Relays"), so of the Rust crates only the clients' `xmoq-msf` (below) reads one.
- **The revision of LOC is LOC-04, with its property numbers on transport draft 18**; decided on 2026-10-11. No document names that pair: MSF-01 cites LOC-02, whose Timestamp is a number draft 18 uses for something else, and LOC-04 cites draft 19. Why, and why not the others, is at the top of the [README of `@xmoq/loc`](../packages/loc/README.md). The catalog names the packaging with the string `"loc"` and carries no property number.
- Where MSF-01 leaves a choice or contradicts itself, what `@xmoq/msf` does is in [its README](../packages/msf/README.md), "How the draft is read".
- **LOC is in a package of its own, `@xmoq/loc`, which depends on neither `@xmoq/moqt` nor `@xmoq/msf`**: it makes and reads the Object Properties of a frame as types and values, and turns a video payload with lengths into one with start codes. It uses the numbers of LOC-04, none of which transport draft 18 defines, and which `moqlivemock`'s publisher sends on draft 18; [its README](../packages/loc/README.md) has the two tables side by side, and what else is read into the draft. **What is of that revision is in a directory with its name (`src/draft-04/`), and one line chooses it**, so that another revision is a small change; `xmoq-loc` in Rust (below) has the same shape.
- **In Rust, the catalog of MSF is read by the crate `xmoq-msf`** (`crates/msf`): the version, and of every track what a viewer chooses by (name, packaging, role, codec, size). It checks far less than `@xmoq/msf`, and does not read the table of hand-written catalogs both are to agree on (`fixtures/msf-01/catalogs.tsv`); the Rust client uses it to find the first H.264 video track (`--catalog-video`).
- **In Rust, LOC is the crate `xmoq-loc`** (`crates/loc`), which depends on no other crate and has what depends on the revision in a module with its name (`draft_04`), as `@xmoq/loc` does: the Timestamp and the Timescale of an Object, and a video payload with a start code for every length prefix. Both read the payloads of `fixtures/loc-04/`.
- **What orders the frames of a video track for a decoder is `@xmoq/media`**, the layer above the three. Its rules are its own where no draft has one (which frame a decoder begins with, when a frame counts as lost): [its README](../packages/media/README.md), "How the receiver decides". It plays the video of `moqlivemock`'s publisher in Chromium (`playVideoTrack`, in the interop run), and a page encodes video that another page plays through the relay (`createVideoTrackEncoder`).
- **A video track is sent with a Group on one stream** (Subgroup 0; a Group begins with a key frame). Decided on 2026-10-11. That leaves the MUST of MSF-01 §6 for a stream for every Object, on purpose; why, and where that sentence stands in its working group, is at `DEFAULT_MAPPING` in `packages/media/src/core/video-sender.ts` and in the README of `@xmoq/media`. The receiver reads both.

## Not decided

What of the protocols is open (the details of the codec, a newer draft, what the relay and the room say to each other, the CLI's audio output) is in the [open questions](open-questions.md). Reactions go over the room's WebSocket in the spike; how they are carried and kept is undecided.

## Primary specifications

- [IETF MoQ](https://datatracker.ietf.org/doc/draft-ietf-moq-transport/)
- [MOQT Streaming Format (MSF)](https://datatracker.ietf.org/doc/draft-ietf-moq-msf/)
- [Low Overhead Media Container (LOC)](https://datatracker.ietf.org/doc/draft-ietf-moq-loc/)
- [W3C WebTransport](https://www.w3.org/TR/webtransport/)
- [W3C WebCodecs](https://www.w3.org/TR/webcodecs/)
- [Cloudflare Durable Objects](https://developers.cloudflare.com/durable-objects/)
