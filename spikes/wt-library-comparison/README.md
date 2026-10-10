# WebTransport library comparison spike

Throwaway code that runs `wtransport` 0.7.2 and `web-transport-quinn` 0.13.2 through the same probes, to collect facts for the choice of the WebTransport library under XmoQ's Rust side. It decides nothing and is not the application layout. Results are in the [experiment record](../../docs/experiments/2026-10-10-webtransport-library-comparison.md).

## Pinned versions

| Part | Version | Source |
|---|---|---|
| `web-transport-quinn` | 0.13.2, feature `ring` instead of the default `aws-lc-rs` | crates.io, `Cargo.lock` |
| `wtransport` | 0.7.2, feature `quinn` | crates.io, `Cargo.lock` |
| `quinn`, `rustls` | 0.11.12, 0.23.45, shared by both libraries | `Cargo.lock` |
| MOQT coding | `xmoq-moqt` from `crates/moqt` | path dependency |
| Relay, moqtail Rust client, browser driver | as pinned in [`spikes/moqtail`](../moqtail/README.md) | |

## Prerequisites

The build steps of [`spikes/moqtail/README.md`](../moqtail/README.md) (relay, Rust client, `npm ci`, Chromium), because the drivers here import Playwright and esbuild from `spikes/moqtail/browser/node_modules` and start that spike's relay and client. `openssl` on `PATH`. Nothing is installed system-wide and no root CA is added.

## Run

From the repository root:

```sh
cargo build --locked --manifest-path spikes/wt-library-comparison/Cargo.toml
cargo build --workspace --locked   # XmoQ's own client, for interop.mjs

node spikes/wt-library-comparison/probe.mjs --library wtq --rounds 100
node spikes/wt-library-comparison/probe.mjs --library wt --rounds 20
node spikes/wt-library-comparison/probe.mjs --library wt-map --rounds 100
node spikes/wt-library-comparison/interop.mjs
```

Each driver creates the certificate on first use, starts what it needs and stops it again.

`probe.mjs` starts one probe server and drives it from a page in headless Chromium. It prints the server's lines and one `RESULT` line with what the page saw.

| `--library` | Server |
|---|---|
| `wtq` | `web-transport-quinn`, with the stream and window limits of `src/common.rs` |
| `wtq-default` | `web-transport-quinn`, quinn's default transport settings |
| `wt` | `wtransport`, with the limits |
| `wt-map` | `wtransport`, with the limits; stream codes mapped to the WebTransport range by hand |
| `wt-map-default` | `wtransport`, default transport settings, codes mapped by hand. This is the setup of the `wt-error-codes` probe |

`--rounds` (default 20) is how often the STOP_SENDING probe and the probe for a stream reset by the page are repeated. `--port` defaults to 44400.

`interop.mjs` pairs the `web-transport-quinn` client and server with the `wtransport`-based peers: the moqtail relay, the probe server on `wtransport`, the moqtail Rust client, XmoQ's own client and the moqtail-ts page. It uses three ports from `--port` (default 44430).

## Server paths

Both servers answer the same paths, and select `moqt-18` whenever the client offers it.

| Path | Server behaviour |
|---|---|
| `/close` | closes the session with code 3 and the reason `protocol violation` |
| `/reset` | opens a unidirectional stream, writes one byte, resets it with code 0x12 |
| `/stop` | accepts a unidirectional stream and sends STOP_SENDING with code 0x5 |
| `/observe`, `/protocol` | reads the peer's first unidirectional stream to its end and prints how the stream and the session ended |
| `/bidi` | accepts a bidirectional stream, reads to FIN and replies; then opens one itself |
| `/streams` | accepts nothing for 1.5 s, then counts the streams that arrived |
| `/window` | reads nothing for 1.5 s and prints the UDP bytes received meanwhile |
| `/` | `web-transport-quinn` only: the server side of a MOQT SETUP exchange, then prints the first request |

## Layout

| Path | Content |
|---|---|
| `src/wtq_server.rs`, `src/wt_server.rs` | probe servers on each library |
| `src/wtq_client.rs` | client on `web-transport-quinn`: `setup` (SETUP exchange) and `probe` |
| `src/common.rs` | codes, limits and the MOQT SETUP shared by the binaries |
| `probe.mjs`, `interop.mjs`, `cert.mjs` | drivers and the certificate |
| `cert/`, `.logs/`, `target/` | generated, not tracked |

The certificate is self-signed ECDSA P-256, valid for 13 days, and is passed to Chromium through `serverCertificateHashes` and to the Rust clients as a pinned SHA-256.
