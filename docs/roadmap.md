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
- [x] Own Rust relay against the `moqtail` browser client and the own Rust client. See the [record](experiments/2026-10-10-own-relay-vs-moqtail-peers.md). The relay is the minimal one: no cache, WebTransport only, no authorization. What the draft requires of a relay and is not implemented is listed in [implementation notes](moqt-implementation-notes.md).
- [x] Own TypeScript browser client against the `moqtail` relay. See the [record](experiments/2026-10-10-own-ts-client-vs-moqtail.md). Done ahead of the relay, because it needs only `moqtail` peers.
- [x] Own TypeScript browser client against the own relay. It runs in the interop matrix below; neither side needed a change.
- [x] One interop runner for every pairing (`interop/`), including the own Rust client, and a CI job that runs it. See the [record](experiments/2026-10-10-own-rust-and-ts-through-moqtail-relay.md). Every pairing runs through the `moqtail` relay and through the own relay, with one-to-many pairings on the own relay.
- [x] Route by namespace in the relay: a SUBSCRIBE for a track that was announced with PUBLISH_NAMESPACE and not yet published (§9.5 of the draft, a MUST for relays). The relay sends its own SUBSCRIBE to the publisher whose matching namespace was announced first. See the [record](experiments/2026-10-10-relay-namespace-routing.md).
- [ ] Several publishers of one track in the relay, before Phase 1: send the SUBSCRIBE to every matching publisher, take and deduplicate the Objects of one track from several of them, and subscribe to a publisher that announces a namespace later (§9.3 and §9.5 of the draft, MUSTs for relays).
- [x] Put the runs in which a publisher or a subscriber disconnects mid-transfer into the interop matrix, so that CI runs them. See the [record](experiments/2026-10-10-disconnects-through-own-relay.md). The defect of `@xmoq/moqt` these runs found, a browser client that could neither cancel nor reset, is fixed; see the [record](experiments/2026-10-10-webtransport-error-in-chromium.md).

**Exit:** one Object between browser and Rust through the own relay with own clients, and the same Object exchange against `moqtail` peers. Reached on 2026-10-10. The open item above is a known gap of the relay, not part of the exit.

## Phase 1 — Video

The browser Phase 1 is built for and judged in is Chromium (Chrome, Edge) on desktop; decided on 2026-10-10. WHY NOT Firefox as well: its Playwright build connected to the own relay and exchanged video with Chromium in every codec tried, but one build on one machine is all that was measured, and a second engine doubles what each step of Phase 1 has to be checked in. WHY NOT Safari: nothing could be run in it ([record of browser support](experiments/2026-10-10-browser-support-webtransport-webcodecs.md)). Which browsers are supported beyond Phase 1 stays an [open decision](open-questions.md).

Media is packaged as LOC says and tracks are described by an MSF catalog, in full; decided on 2026-10-10 (`draft-ietf-moq-msf-01`; which revision of LOC is an [open decision](open-questions.md)). That puts FETCH into Phase 1: MSF has "Subscribers accessing the catalog MUST use SUBSCRIBE with a Joining FETCH (offset = 0)", and the own implementation has no FETCH. It settles how Groups are numbered: a publisher that restarts "MUST ensure the new starting Group ID is greater than any previously published Group ID for that track" (MSF §6.1), for which MSF suggests the wall clock time. WHY NOT only the shape of LOC and MSF with a fixed track name and no catalog: it would start sooner, and would leave the players of other MoQ implementations unable to find the tracks. What following these drafts takes, and which peers speak them, is in the [survey](experiments/2026-10-10-loc-msf-fetch-survey.md). Two things it leaves [open](open-questions.md): whether a relay cache is built in Phase 1, which neither MSF nor draft 18 asks for, and whether every Object goes onto a stream of its own, which MSF §6 asks for and the nearest peer does not do.

- [ ] FETCH and Joining FETCH in the Rust and the TypeScript session, against peers.
- [ ] The relay answers a FETCH, from the publisher or from a cache.
- [ ] MSF catalog track: published by the browser publisher, read by the viewers.
- [ ] Browser WebCodecs publisher.
- [ ] Rust relay fan-out to browser viewers.
- [ ] Decode and render video.
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
