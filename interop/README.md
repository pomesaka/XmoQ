# Interop

Moves one MOQT draft 18 Object through a [moqtail](https://github.com/moqtail/moqtail) relay between two clients, at least one of which is an own client built on [`@xmoq/moqt`](../packages/moqt/), and checks that the received payload equals the sent one. The browser clients run in a real Chromium. Results go to [`docs/experiments/`](../docs/experiments/).

## Peers

| Part | Version | Source |
|---|---|---|
| Relay | `relay` 0.15.0 | `moqtail/moqtail` tag `relay@0.15.0`, built with the upstream `Cargo.lock` |
| Rust client (`moqtail-rust`, "moqtail Rust client" in the records) | `moqtail` 0.15.0, `wtransport` 0.7.2 | crates.io, `peers/moqtail/rust-client/Cargo.lock` |
| Browser client (`moqtail-browser`, "moqtail-ts client" in the records) | `moqtail` 0.13.0 | npm, the root `bun.lock` |
| Browser driver | `playwright-core` 1.64.0 | npm, the root `bun.lock` |
| Wire | MOQT draft 18, WebTransport subprotocol `moqt-18` | |

## Build

Needs `cargo`, `openssl`, a Chromium installed for Playwright 1.64.0, and `bun` (from `mise.toml`; prefix the `bun` commands with `mise exec --` where mise is not activated). From the repository root:

```sh
cargo install --git https://github.com/moqtail/moqtail --tag relay@0.15.0 --locked --root interop/peers/moqtail/.tools relay
cargo build --locked --manifest-path interop/peers/moqtail/rust-client/Cargo.toml
bun install --frozen-lockfile
```

`interop` and `packages/*` are one Bun workspace with one `bun.lock` in the repository root, so `@xmoq/moqt` here is the working copy of `packages/moqt`.

## Checks

`mise run check:ts` (the task CI runs) lints and type-checks `run.ts`, `page.ts` and `xmoq-page.ts`. It does not perform a run: that needs the relay and a browser, and is done by hand for now. `peers/` is outside lint and formatting, because it holds copies of code written for moqtail that are kept as they came.

## Run

From `interop/`:

```sh
bun run.ts --publisher own-browser --subscriber moqtail-rust
```

`run.ts` generates the certificate, starts the relay, serves the pages, launches headless Chromium, sends one Object and stops everything it started. It prints a `RESULT` line and exits 0 when the payloads match.

| Option | Values (default first) |
|---|---|
| `--publisher` | `own-browser`, `moqtail-browser`, `moqtail-rust` |
| `--subscriber` | `moqtail-rust`, `own-browser`, `moqtail-browser` |
| `--port` | `44330`. UDP port of the relay |
| `--subscribe-track` | `object`. Any other name makes the relay answer REQUEST_ERROR, and the run fails by design |
| `--verbose` | off. Sets `RUST_LOG=debug` for the relay |

One side has to be `own-browser`: a run between two moqtail peers checks nothing of this repository.

| Client | Code |
|---|---|
| `own-browser` | `page.ts`, on `@xmoq/moqt` |
| `moqtail-browser` | `peers/moqtail/browser/page.ts`, on the `moqtail` npm package |
| `moqtail-rust` | `peers/moqtail/rust-client/`, on the `moqtail` crate |

## Certificate

The relay runs on a self-signed certificate that `run.ts` creates under `.cert/` and re-creates when less than a day of validity is left. No root CA is installed and no browser flag is set.

| Requirement | Reason |
|---|---|
| ECDSA P-256, valid for under two weeks (13 days here) | Chromium accepts nothing else through `serverCertificateHashes`, which is how the browser clients pin the certificate's SHA-256 |
| Key from `openssl ecparam -genkey`, certificate from `openssl req -x509 -new -key` | With LibreSSL 3.3.6, the relay could not load a key written by the one-step `openssl req -newkey ec` |

The Rust client pins the same SHA-256 through `wtransport`'s `with_server_certificate_hashes`.

## Generated files

None of these is tracked.

| Path | Content |
|---|---|
| `.cert/` | Certificate and key |
| `.logs/` | Relay log files |
| `.dist/` | Bundled pages |
| `peers/moqtail/.tools/` | Relay binary |
| `peers/moqtail/rust-client/target/` | Rust client build |
| `node_modules/` | Links into the `node_modules/` of the repository root |
