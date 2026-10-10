# Interop

Moves one MOQT draft 18 Object through a relay between two clients, at least one of which is an own client, and checks that the received payload equals the sent one. The own clients are the browser client on [`@xmoq/moqt`](../packages/moqt/) and the Rust client [`xmoq-moqt-client`](../crates/moqt-client/). The relay is the [moqtail](https://github.com/moqtail/moqtail) relay or the own relay [`xmoq-relay`](../crates/relay/). The browser clients run in a real Chromium. Results go to [`docs/experiments/`](../docs/experiments/).

This is the one entry point for interop runs. `spikes/moqtail/browser/run.mjs` is kept only to reproduce the Phase 0 records.

## Peers

| Part | Version | Source |
|---|---|---|
| Relay | `relay` 0.15.0 | `moqtail/moqtail` tag `relay@0.15.0`, built with the upstream `Cargo.lock` |
| Rust client (`moqtail-rust`, "moqtail Rust client" in the records) | `moqtail` 0.15.0, `wtransport` 0.7.2 | crates.io, `peers/moqtail/rust-client/Cargo.lock` |
| Browser client (`moqtail-browser`, "moqtail-ts client" in the records) | `moqtail` 0.13.0 | npm, the root `bun.lock` |
| Browser driver | `playwright-core` 1.64.0 | npm, the root `bun.lock` |
| Wire | MOQT draft 18, WebTransport subprotocol `moqt-18` | |

## Build

Needs `cargo`, `openssl` and the tools of `mise.toml`. From the repository root, this builds everything and runs every pairing:

```sh
mise run interop
```

It runs `scripts/ci/interop.sh`, which does these steps and skips what is already built:

```sh
cargo install --git https://github.com/moqtail/moqtail --tag relay@0.15.0 --locked --root interop/peers/moqtail/.tools relay
cargo build --locked --manifest-path interop/peers/moqtail/rust-client/Cargo.toml
cargo build --locked -p xmoq-moqt-client -p xmoq-relay
bun install --frozen-lockfile
bun run --cwd interop playwright-core install --no-shell chromium
bun interop/matrix.ts
```

The Chromium is the build that the pinned `playwright-core` drives; it goes to Playwright's cache directory, outside the repository.

`interop` and `packages/*` are one Bun workspace with one `bun.lock` in the repository root, so `@xmoq/moqt` here is the working copy of `packages/moqt`.

## Checks

`mise run check:ts` lints and type-checks `run.ts`, `matrix.ts`, `page.ts` and `xmoq-page.ts`. It does not perform a run. `peers/` is outside lint and formatting, because it holds copies of code written for moqtail that are kept as they came.

The runs are the CI job `interop`, which calls `mise run interop`. On a pull request that changes nothing the run depends on (`scripts/ci/interop-scope.sh` has the list), the job ends green without running the pairings and says so in its summary and in a notice. A job that ran shows the table of pairings in its summary and keeps their logs as the artifact `interop-logs`. Its last step fails if the job did neither.

## Run

Every pairing, from `interop/`:

```sh
bun matrix.ts
```

`matrix.ts` starts `run.ts` once per pairing and relay, one after the other and each on its own port, prints a table, and exits 0 only if every pairing ended as expected. The log of each pairing is in `.logs/matrix/`.

| Pairings, through each relay | What is expected |
|---|---|
| Every publisher with every subscriber, except moqtail with moqtail (12) | The received payload equals the sent one. Where the subscriber is an own client, the publisher then leaves and the subscriber has to see PUBLISH_DONE |
| One publisher with three subscribers: `own-rust` to `own-browser`, `moqtail-browser` to `own-rust` (2) | Every subscriber receives the payload and then sees PUBLISH_DONE |
| `own-browser` and `own-rust` subscribing to a track nobody published (2) | The run fails, and its log shows REQUEST_ERROR `0x10` |

| Option of `matrix.ts` | Values (default first) |
|---|---|
| `--relay` | `moqtail` and `own`. Give it once to run through one relay only |
| `--base-port` | `44400`. The first of the UDP ports used, one per pairing |
| `--only` | Empty. Runs the pairings whose name contains the text, such as `--only own-rust` or `--only "own relay"` |
| `--pairing-timeout-seconds` | `120` (provisional). A pairing that takes longer is stopped and counted as failed |

A SIGINT or SIGTERM to `matrix.ts` stops the pairing that is running, which stops its relay, browser and clients, and starts no further one.

One pairing:

```sh
bun run.ts --publisher own-browser --subscriber own-rust --await-publish-done
```

`run.ts` generates the certificate, starts the relay, serves the pages and launches headless Chromium when a client is a page, sends one Object and stops everything it started. It prints a `RESULT` line and exits 0 when the payloads match.

| Option of `run.ts` | Values (default first) |
|---|---|
| `--publisher` | `own-browser`, `own-rust`, `moqtail-browser`, `moqtail-rust` |
| `--subscriber` | `moqtail-rust`, `own-browser`, `own-rust`, `moqtail-browser` |
| `--relay` | `moqtail`, `own` |
| `--subscribers` | `1`. How many subscribers of the kind `--subscriber` take part; each has to receive the Object |
| `--port` | `44330`. UDP port of the relay |
| `--subscribe-track` | `object`. Any other name makes the relay answer REQUEST_ERROR, and the run fails by design |
| `--await-publish-done` | off. After the transfer, makes the publisher leave and waits until the subscriber reports that PUBLISH_DONE ended its subscription. The subscriber has to be an own client |
| `--verbose` | off. Sets `RUST_LOG=debug` for the moqtail relay and makes `own-rust` print the bytes of every stream |

One side has to be an own client: a run between two moqtail peers checks nothing of this repository.

An own publisher leaves by sending PUBLISH_DONE. A moqtail publisher has no way to and closes its session, after which the relay sends PUBLISH_DONE to the subscriber on its own.

| Client | Code |
|---|---|
| `own-browser` | `page.ts`, on `@xmoq/moqt` |
| `own-rust` | `crates/moqt-client`, on `xmoq-moqt` |
| `moqtail-browser` | `peers/moqtail/browser/page.ts`, on the `moqtail` npm package |
| `moqtail-rust` | `peers/moqtail/rust-client/`, on the `moqtail` crate |

## Certificate

The relay runs on a self-signed certificate that `run.ts` creates under `.cert/` and re-creates when less than a day of validity is left. No root CA is installed and no browser flag is set.

| Requirement | Reason |
|---|---|
| ECDSA P-256, valid for under two weeks (13 days here) | Chromium accepts nothing else through `serverCertificateHashes`, which is how the browser clients pin the certificate's SHA-256 |
| Key from `openssl ecparam -genkey`, certificate from `openssl req -x509 -new -key` | With LibreSSL 3.3.6, the relay could not load a key written by the one-step `openssl req -newkey ec` |

The Rust clients pin the same SHA-256: the own client through `web-transport-quinn`'s `with_server_certificate_hashes`, the moqtail client through `wtransport`'s.

## Generated files

None of these is tracked.

| Path | Content |
|---|---|
| `.cert/` | Certificate and key |
| `.logs/` | Relay log files, and in `.logs/matrix/` the output of each pairing |
| `.dist/` | Bundled pages |
| `peers/moqtail/.tools/` | Binary of the moqtail relay. The own relay is built into `target/` of the repository root |
| `peers/moqtail/rust-client/target/` | Build of the moqtail Rust client. The own Rust client is built into `target/` of the repository root |
| `node_modules/` | Links into the `node_modules/` of the repository root |
