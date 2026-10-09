# Roadmap (proposed, no committed dates)

## Phase 0 — Feasibility
- [ ] Select candidate Rust and browser MoQ libraries; record supported draft revisions.
- [ ] Establish browser ↔ Rust WebTransport/MoQ connectivity.
- [ ] Transfer one MoQ Object and document reproducible commands.

**Exit:** reproducible Object exchange or documented blocker.

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
