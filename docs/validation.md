# Validation plan

What a run looks at, and what a record of it has. The runs so far are in [the experiment records](experiments/README.md), and what they showed for the demo is gathered in [the A/V demo and its limits](av-demo.md). All of them ran on one machine.

## Interoperability
Record exact Rust/browser library versions, MoQ draft compatibility, certificates, connection setup, publish/subscribe and Object delivery.

## Media
Validate frame ordering, timestamps, keyframes, late join, reconnect, codec behavior, jitter buffering and A/V synchronization.

## CLI
Validate Ghostty Kitty Graphics rendering, achieved FPS, artifacts, audio output and terminal compatibility. Kitty Graphics support does not imply native video decoding.

## Performance
Measure glass-to-glass latency (p50/p95/p99), dropped frames, stalls, relay RSS, per-subscriber memory, CPU, bandwidth, and recovery after packet loss.

The goals to aim at are in the [requirements](requirements.md) (row "Latency, capacity"). Not run yet: over real networks, on more than one machine, with real viewers. Do not invent benchmark results.

## Security
Before real use, decide authentication, authorization at relay, TLS, quotas, privacy and log retention.

For each experiment record commit SHA, commands, environment, observed results and limitations.
