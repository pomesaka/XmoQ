# Validation plan

No tests described here have been completed.

## Interoperability
Record exact Rust/browser library versions, MoQ draft compatibility, certificates, connection setup, publish/subscribe and Object delivery.

## Media
Validate frame ordering, timestamps, keyframes, late join, reconnect, codec behavior, jitter buffering and A/V synchronization.

## CLI
Validate Ghostty Kitty Graphics rendering, achieved FPS, artifacts, audio output and terminal compatibility. Kitty Graphics support does not imply native video decoding.

## Performance
Measure glass-to-glass latency (p50/p95/p99), dropped frames, stalls, relay RSS, per-subscriber memory, CPU, bandwidth, and recovery after packet loss.

**Targets and test environment are undecided.** Do not invent benchmark results.

## Security
Before real use, decide authentication, authorization at relay, TLS, quotas, privacy and log retention.

For each experiment record commit SHA, commands, environment, observed results and limitations.
