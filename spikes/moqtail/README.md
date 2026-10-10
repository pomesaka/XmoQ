# moqtail spike (Phase 0)

Throwaway code for the Phase 0 gate in [the roadmap](../../docs/roadmap.md): move one MoQ Object between a browser and Rust through a relay, using [moqtail](https://github.com/moqtail/moqtail) at MOQT draft 18. It is not the application layout. `browser/run.mjs` is also the driver for the interoperability runs of XmoQ's own Rust client in Phase 0b. Results are in the experiment records for [the moqtail transfer](../../docs/experiments/2026-10-10-moqtail-object-transfer.md) and for [the own Rust client](../../docs/experiments/2026-10-10-own-rust-client-vs-moqtail.md).

## Pinned versions

| Part | Version | Source |
|---|---|---|
| Relay | `relay` 0.15.0 (`moqtail/0.15.0+dd8623d`) | `moqtail/moqtail` tag `relay@0.15.0`, built with the upstream `Cargo.lock` |
| Rust client library | `moqtail` 0.15.0, `wtransport` 0.7.2 | crates.io, `rust-client/Cargo.lock` |
| Browser library | `moqtail` 0.13.0 | npm, `browser/package-lock.json` |
| Browser driver | `playwright` 1.64.0 (Chromium 156.0.8078.4) | npm |
| Wire | MOQT draft 18, WebTransport subprotocol `moqt-18` | |

## Prerequisites

`cargo`, `node` with `npm`, and `openssl` on `PATH`. Nothing is installed system-wide and no root CA is added.

## Run

From the repository root:

```sh
cargo install --git https://github.com/moqtail/moqtail --tag relay@0.15.0 --locked --root spikes/moqtail/.tools relay
cargo build --locked --manifest-path spikes/moqtail/rust-client/Cargo.toml
npm --prefix spikes/moqtail/browser ci
npm --prefix spikes/moqtail/browser exec -- playwright install chromium
node spikes/moqtail/browser/run.mjs
```

A run that names `own` for a Rust side also needs XmoQ's client, built with `cargo build --workspace --locked`.

`run.mjs` generates a certificate, starts the relay, serves the page, launches headless Chromium, sends one Object and stops everything it started. It prints a `RESULT` line with `"match":true` and exits 0 when the received payload equals the sent one.

| Option | Values (default first) |
|---|---|
| `--direction` | `browser-to-rust`, `rust-to-browser`, `browser-to-browser`, `rust-to-rust` |
| `--rust-publisher`, `--rust-subscriber` | `moqtail` (the client in `rust-client/`), `own` (XmoQ's `xmoq-moqt-client`). Ignored for a side that `--direction` puts in the browser |
| `--trace` | off. Makes the `own` client print the raw bytes of every stream |
| `--await-publish-done` | off. After the transfer, ends the Rust publisher and waits until the `own` subscriber reports that PUBLISH_DONE ended its subscription. Needs `--direction rust-to-rust --rust-subscriber own` |
| `--browser` | `chromium` (full browser, new headless), `headless-shell`, `chrome` (the installed Google Chrome) |
| `--cert-trust` | `hash`, `spki-flag`, `none` |
| `--port` | `44318` |
| `--verbose` | off. Sets `RUST_LOG=debug` for the relay |

## Certificate

The relay needs a PEM certificate and key. `run.mjs` creates a self-signed ECDSA P-256 certificate valid for 13 days under `cert/`, and re-creates it when it is about to expire.

| `--cert-trust` | How the browser accepts it |
|---|---|
| `hash` | The page passes the certificate's SHA-256 as `serverCertificateHashes` to `WebTransport`. No browser flag. Chromium requires ECDSA P-256 and a validity under two weeks for this |
| `spki-flag` | Chromium is launched with `--ignore-certificate-errors-spki-list=<SPKI hash>` and `--origin-to-force-quic-on=<host:port>`. The page is unchanged |
| `none` | Negative control. The connection fails in the TLS handshake |

The Rust client pins the same SHA-256 through `wtransport`'s `with_server_certificate_hashes`.

## Layout

| Path | Content |
|---|---|
| `rust-client/` | Rust subscriber and publisher on the `moqtail` crate |
| `browser/src/page.ts` | Browser publisher and subscriber on the `moqtail` npm package |
| `browser/run.mjs` | Driver |
| `.tools/`, `cert/`, `.logs/` | Generated, not tracked |

Everything here is provisional for this check: no authentication, the namespace `xmoq/phase0` and track `object` are arbitrary, and relay logs go to `.logs/` with the relay's defaults.
