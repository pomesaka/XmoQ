# Roadmap (proposed, no committed dates)

## Phase 0 — Feasibility
- [x] Select candidate Rust and browser MoQ libraries; record supported draft revisions. See [MoQ library candidates](moq-libraries.md).
- [x] Establish browser ↔ Rust WebTransport/MoQ connectivity.
- [x] Transfer one MoQ Object and document reproducible commands. See the [moqtail](experiments/2026-10-10-moqtail-object-transfer.md) and [moq-dev](experiments/2026-10-10-moq-dev-object-transfer.md) records.

**Exit:** reproducible Object exchange or documented blocker. Reached on 2026-10-10 with both library pairs.

## Phase 0b — Own MoQ Transport implementation (draft 18)

Each step replaces one `moqtail` component with an own one, so that one side of every run is a known-good peer.

- [x] Rust wire codec and session for SETUP, PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE and subgroup streams, with byte fixtures captured from `moqtail`. The session covers the side that opens requests; answering a peer's requests comes with the relay. See [`fixtures/moqt-18/`](../fixtures/moqt-18/README.md).
- [x] Own Rust client against the `moqtail` relay and the `moqtail` browser client. See the [record](experiments/2026-10-10-own-rust-client-vs-moqtail.md).
- [x] Own Rust relay against the `moqtail` browser client and the own Rust client. See the [record](experiments/2026-10-10-own-relay-vs-moqtail-peers.md). The relay of that run was the minimal one: no cache, WebTransport only, no authorization; the cache came in Phase 1. What the draft requires of a relay and is not implemented is listed in [implementation notes](moqt-implementation-notes.md).
- [x] Own TypeScript browser client against the `moqtail` relay. See the [record](experiments/2026-10-10-own-ts-client-vs-moqtail.md). Done ahead of the relay, because it needs only `moqtail` peers.
- [x] Own TypeScript browser client against the own relay. It runs in the interop matrix below; neither side needed a change.
- [x] One interop runner for every pairing (`interop/`), including the own Rust client, and a CI job that runs it. See the [record](experiments/2026-10-10-own-rust-and-ts-through-moqtail-relay.md). Every pairing runs through the `moqtail` relay and through the own relay, with one-to-many pairings on the own relay.
- [x] Route by namespace in the relay: a SUBSCRIBE for a track that was announced with PUBLISH_NAMESPACE and not yet published (§9.5 of the draft, a MUST for relays). The relay sends its own SUBSCRIBE to the publisher whose matching namespace was announced first. See the [record](experiments/2026-10-10-relay-namespace-routing.md).
- [x] Several publishers of one track in the relay, before Phase 1: send the SUBSCRIBE to every matching publisher, take and deduplicate the Objects of one track from several of them, and subscribe to a publisher that announces a namespace later (§9.3 and §9.5 of the draft, MUSTs for relays). See the records on [a second publisher](experiments/2026-10-10-relay-several-publish-sources.md), on [asking every publisher](experiments/2026-10-11-relay-ask-every-publisher.md) and on [a Subgroup that comes with two priorities](experiments/2026-10-11-relay-subgroup-priority-mismatch.md), and the pairings with two publishers in the [interop matrix](experiments/2026-10-10-two-publishers-in-the-interop-matrix.md). The MUSTs are met in part: what is left short of them, and why, is listed under "Several publishers of one track" and "Sources of a track that do not send the same thing" in the [implementation notes](moqt-implementation-notes.md). Duplicates are removed by Subgroup, not by Object, and of what two sources send only the Publisher Priority is compared.
- [x] Put the runs in which a publisher or a subscriber disconnects mid-transfer into the interop matrix, so that CI runs them. See the [record](experiments/2026-10-10-disconnects-through-own-relay.md). The defect of `@xmoq/moqt` these runs found, a browser client that could neither cancel nor reset, is fixed; see the [record](experiments/2026-10-10-webtransport-error-in-chromium.md).

**Exit:** one Object between browser and Rust through the own relay with own clients, and the same Object exchange against `moqtail` peers. Reached on 2026-10-10. The item on several publishers was not part of the exit and was closed on 2026-10-11.

## Phase 1 — Video

The browser Phase 1 is built for and judged in is Chromium (Chrome, Edge) on desktop; decided on 2026-10-10. WHY NOT Firefox as well: its Playwright build connected to the own relay and exchanged video with Chromium in every codec tried, but one build on one machine is all that was measured, and a second engine doubles what each step of Phase 1 has to be checked in. WHY NOT Safari: nothing could be run in it ([record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md)). Which browsers are supported beyond Phase 1 stays an [open decision](open-questions.md).

Media is packaged as LOC says and tracks are described by an MSF catalog, in full; decided on 2026-10-10 (`draft-ietf-moq-msf-01`; of LOC the numbers of `draft-ietf-moq-loc-04` on draft 18, decided on 2026-10-11, with the reasons in [`packages/loc/README.md`](../packages/loc/README.md); a Group of video goes onto one stream, decided the same day, with the reasons at `DEFAULT_MAPPING` in `@xmoq/media`; a catalog names H.264 `avc3`). That puts FETCH into Phase 1: MSF has "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining FETCH (offset = 0)", and the own implementation has no FETCH. It settles how Groups are numbered: a publisher that restarts "MUST ensure the new starting Group ID is greater than any previously published Group ID for that track" (MSF §6.1), for which MSF suggests the wall clock time. WHY NOT only the shape of LOC and MSF with a fixed track name and no catalog: it would start sooner, and would leave the players of other MoQ implementations unable to find the tracks. What following these drafts takes, and which peers speak them, is in the [survey](experiments/2026-10-10-loc-msf-fetch-survey.md). One thing it leaves [open](open-questions.md): whether every Object goes onto a stream of its own, which MSF §6 asks for and the nearest peer does not do.

The video codec is H.264, in Annex B format; decided on 2026-10-11. WHY H.264: in Chromium it is the one codec with a hardware encoder, which took about a tenth of the CPU time of the software encoders at 1280x720 and 30 frames a second (4% of a core, against 33% for VP8, 54% for VP9, 55% for AV1 and 71% for H.264 in software), and where no hardware decoder is used it was the cheapest to decode ([record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md)). WHY Annex B and not `avc` with a `description`: every key chunk carries the parameter sets, so a viewer that joins late decodes from the next key frame with the codec string alone, where WebCodecs emits the `description` once ([WebCodecs loopback record](experiments/2026-10-10-webcodecs-loopback.md)). WHY NOT VP9: it has no choice of format to get wrong and its decoder reports a missing frame, which H.264 decoders do not (they show corrupt pictures without an error); it costs over ten times the CPU time to encode. What the decision rests on is one Mac, a generated picture and CPU time, not power or picture quality. One thing is not settled by it: in one run of the shipped Chrome 155 the hardware encoder's queue reached 91 frames, about three seconds of picture, and whether it keeps up in real time was not looked at further. That is to be measured when the publisher exists.

The relay gets a cache of the latest Group of each track in Phase 1; decided on 2026-10-11. Neither draft 18 nor MSF asks for one (§9.1: "Relays MAY cache Objects, but are not required to"), and the relay answers a FETCH without it, by sending a FETCH of its own to one publisher ([implementation notes](moqt-implementation-notes.md)). WHY a cache all the same: what building that relay showed it cannot do. A browser publisher has to answer every FETCH itself, and once it is gone nobody can get the catalog. A FETCH with FILL_TIMEOUT 0, which asks for what the relay holds, gets nothing. The relay cannot keep the end of a Joining FETCH and the start of the subscription from overlapping, since the publisher answers the one and the relay said where the other begins. With several publishers of a track a gap can open between the two: the relay tells a subscriber the largest Location of all of them and fetches from the first one only. WHY the latest Group and not more: it is what a Joining FETCH with offset 0 asks for, the form MSF has a subscriber get the catalog with, and one of the ways a viewer that joins late can get its first picture (which way is taken is [open](open-questions.md)); a FETCH for older Groups still goes to the publisher. WHY NOT leave it for later: the browser publisher would be built to answer FETCHes that a cache then takes from it, and the gaps above would be in the first demo. The publisher still answers a FETCH the cache cannot. What the cache holds and how much is [open](open-questions.md).

- [ ] FETCH and Joining FETCH in the Rust and the TypeScript session, against peers.
- [ ] The relay answers a FETCH, from the publisher or from a cache. A Standalone FETCH is passed on to one publisher ([record](experiments/2026-10-11-standalone-fetch-through-own-relay.md)), and a Joining FETCH as a range the relay works out ([record](experiments/2026-10-11-joining-fetch-through-own-relay.md)). From the cache: the item below.
- [ ] The relay keeps the latest Group of each track and answers a FETCH from it where it can. Done for a FETCH whose range lies in that Group while the track has a publisher ([record](experiments/2026-10-11-the-relay-answers-a-fetch-from-its-cache.md), and the row on the relay's cache in the [implementation notes](moqt-implementation-notes.md)). The interop matrix has a page read a Group in two Subgroups from it, and a page cancel a Group of 7 MB that the relay then stops writing ([record](experiments/2026-10-11-fetch-runs-with-the-relays-cache.md)). Open: a FETCH with FILL_TIMEOUT 0, keeping the Group for 30 seconds after the last publisher has gone, and an Object that comes again with other content.
- [x] MSF catalog track: published by the browser publisher, read by the viewers. `@xmoq/msf` reads and writes the catalog, follows a catalog track and publishes one; a page's catalog is read through the own relay by another page and by `moqlivemock`'s subscriber, and a page reads the catalog of `moqlivemock`'s publisher ([reading](experiments/2026-10-11-a-page-reads-a-catalog.md), [publishing](experiments/2026-10-11-a-page-publishes-a-catalog.md)). What is short of the draft is in [`packages/msf/README.md`](../packages/msf/README.md): delta updates are not applied, GZIP is not read, and while the relay has no cache a catalog cannot be had once its publisher is gone.
- [x] Browser WebCodecs publisher. `createVideoTrackEncoder` of `@xmoq/media` encodes H.264 in Annex B and publishes it as a LOC track; a page's video is played by another page through the own relay and through the `moqtail` relay, in both mappings, with what each frame shows read from its pixels, and with Groups of several MiB ([record](experiments/2026-10-11-pages-send-and-play-video.md)). A key frame of more than 1,000,000 bytes is not sent, and the caller is not told ([`packages/media/README.md`](../packages/media/README.md)).
- [ ] Rust relay fan-out to browser viewers. One viewer plays a page's video through the relay (item above); several viewers at once, and a viewer that comes after the relay has subscribed, were not run with video.
- [ ] Decode and render video. Decoding is done (`playVideoTrack`, which hands each `VideoFrame` to its caller); nothing draws the frames, and there is no buffer for playback.
- [ ] Test late join, keyframes, reconnect, cancellation and slow subscribers.

**Exit:** reproducible one-publisher/multiple-viewer video demo, on loopback. A run over real links waits for the hosting decision.

## Phase 2 — Audio and quality
- [ ] Build the audio pipeline. The codec is Opus, decided on 2026-10-10: it is the one audio codec Chromium, Firefox and Safari all encode and decode ([record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md)). Audio is not part of Phase 1. WHY NOT earlier: a second track brings the question of what a slow viewer's limit is counted by ([open decision](open-questions.md)) into Phase 1, where one video track does not raise it.
- [ ] Add playback buffers and A/V synchronization.
- [ ] Measure latency, drops, stalls and recovery under loss.

**Exit:** documented A/V demo and limitations.

## Phase 3 — Ghostty CLI
- [ ] Rust MoQ subscriber.
- [ ] Video decoding and Kitty Graphics experiment.
- [ ] Native audio output and terminal controls.
- [ ] Measure FPS, CPU and memory.

**Exit:** reproducible terminal demo with compatibility notes.

## Phase 4 — Rooms and interactions
- [ ] Design room/session identity.
- [ ] Validate Durable Objects integration.
- [ ] Decide chat transport, persistence and ordering.
- [ ] Implement chat; evaluate reactions.
- [ ] Design and test authorization before broader rollout.

**Exit:** managed room with verified browser/CLI interactions.

## Phase 5 — Future
- [ ] Relay scale/load testing.
- [ ] SNS event integration contract.
- [ ] Revisit multi-publisher and bidirectional requirements.

**Exit:** evidence-based design decisions, not assumed launch.

Agents: work in small reviewable increments, record test evidence and blockers, and never silently decide open requirements.
