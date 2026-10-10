# Roadmap (proposed, no committed dates)

## Phase 0 — Feasibility
- [x] Select candidate Rust and browser MoQ libraries; record supported draft revisions. See [MoQ library candidates](moq-libraries.md).
- [x] Establish browser ↔ Rust WebTransport/MoQ connectivity.
- [x] Transfer one MoQ Object and document reproducible commands. See the [moqtail](experiments/2026-10-10-moqtail-object-transfer.md) and [moq-dev](experiments/2026-10-10-moq-dev-object-transfer.md) records.

**Exit:** reproducible Object exchange or documented blocker. Reached on 2026-10-10 with both library pairs.

## Phase 0b — Own MoQ Transport implementation (draft 18)

Each step replaces one `moqtail` component with an own one, so that one side of every run is a known-good peer.

- [ ] Rust wire codec and session for SETUP, PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE and subgroup streams, with byte fixtures captured from `moqtail`.
- [ ] Own Rust client against the `moqtail` relay and the `moqtail` browser client.
- [ ] Own Rust relay against the `moqtail` browser client and the own Rust client.
- [ ] Own TypeScript browser client against the own relay and against the `moqtail` relay.

**Exit:** one Object between browser and Rust through the own relay with own clients, and the same Object exchange against `moqtail` peers.

## Phase 1 — Video
- [ ] Browser WebCodecs publisher.
- [ ] Rust relay fan-out to browser viewers.
- [ ] Decode and render video.
- [ ] Test late join, keyframes, reconnect, cancellation and slow subscribers.

**Exit:** reproducible one-publisher/multiple-viewer video demo.

## Phase 2 — Audio and quality
- [ ] Choose audio codec and pipeline.
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
