# Interop

Moves MOQT draft 18 Objects through a relay between clients, at least one of which is an own client, and checks that the received payload equals the sent one. One set of runs moves a single Object between every pair of clients; another has a client leave on the way and checks what the others see. The own clients are the browser client on [`@xmoq/moqt`](../packages/moqt/) and the Rust client [`xmoq-moqt-client`](../crates/moqt-client/). The relay is the [moqtail](https://github.com/moqtail/moqtail) relay or the own relay [`xmoq-relay`](../crates/relay/). The browser clients run in a real Chromium. Results go to [`docs/experiments/`](../docs/experiments/).

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

`mise run check:ts` lints and type-checks every `.ts` file of this directory. It does not perform a run. `peers/` is outside lint and formatting, because it holds copies of code written for moqtail that are kept as they came.

The runs are the CI job `interop`, which calls `mise run interop`. On a pull request that changes nothing the run depends on (`scripts/ci/interop-scope.sh` has the list), the job ends green without running the pairings and says so in its summary and in a notice. A job that ran shows the table of pairings in its summary and keeps their logs as the artifact `interop-logs`. Its last step fails if the job did neither.

## Run

Every pairing, from `interop/`:

```sh
bun matrix.ts
```

`matrix.ts` starts a driver once per pairing and relay, each on its own port, prints a table, and exits 0 only if every pairing ended as expected. The log of each pairing is in `.logs/matrix/`. The pairings run one after the other, except three that mostly wait for the relay to give up on a client that vanished (6 to 9 seconds each): those start together after the pairings that move one Object, and go on beside the ones that follow.

| Pairings, through each relay | What is expected |
|---|---|
| Every publisher with every subscriber, except moqtail with moqtail (12) | The received payload equals the sent one. Where the subscriber is an own client, the publisher then leaves and the subscriber has to see PUBLISH_DONE |
| One publisher with three subscribers: `own-rust` to `own-browser`, `moqtail-browser` to `own-rust` (2) | Every subscriber receives the payload and then sees PUBLISH_DONE |
| `own-browser` and `own-rust` subscribing to a track nobody published (2) | The run fails, and its log shows REQUEST_ERROR `0x10` |

The runs in which a client leaves go through the own relay only, because the driver waits for lines only that relay prints. Their names begin with `leaving:`. What each has to show, and why, is in the [record](../docs/experiments/2026-10-10-disconnects-through-own-relay.md).

| Runs with a client that leaves (13) | What is expected |
|---|---|
| The publisher is killed, or its page closed, after its Objects arrived (3) | Every own subscriber sees PUBLISH_DONE `TRACK_ENDED` with the Stream Count of the streams it got, and a stream the publisher left open ends with a reset with CANCELLED. A killed publisher has to be noticed within 10.5 seconds, and a killed subscriber within 7.5 |
| The publisher sends one Object and closes its session at once (4) | Every subscriber receives the Object, and every own subscriber then sees PUBLISH_DONE |
| Subscribers leave, by cancelling, by closing their session or by being killed, while others stay (4) | The publisher goes on, and the subscribers that stay receive what is sent after the others left, also before the relay has noticed that a killed process is gone, which the run checks |
| `own-browser` cancels its subscription (1) | The relay ends the request, opens no stream for what the publisher sends later, and keeps the session. Where the relay printed the STOP_SENDING it got on the request stream, its code has to be CANCELLED (`0x1`). The relay does not always print it: it did in 53 of 60 rounds, and a round without the line is not judged on the code. For `moqtail-browser`, which cancels in two of the four runs above, the line has to be there with code `0x0` |
| An `own-rust` subscriber is stopped with SIGSTOP and takes nothing (1) | The relay ends that subscription with PUBLISH_DONE `TOO_FAR_BEHIND` and keeps its session; the other subscribers receive every Object |

Each of these runs ends by moving one more Object over the same track with a fresh publisher and subscriber. The relay refuses a PUBLISH for a track that has a publisher, so this step fails if the relay still holds the one that left.

| Option of `matrix.ts` | Values (default first) |
|---|---|
| `--relay` | `moqtail` and `own`. Give it once to run through one relay only |
| `--base-port` | `44400`. The first of the UDP ports used, one per pairing |
| `--only` | Empty. Runs the pairings whose name contains the text, such as `--only own-rust`, `--only "own relay"` or `--only "leaving: "` |
| `--pairing-timeout-seconds` | `120` (provisional). A pairing that takes longer is stopped and counted as failed |

A SIGINT or SIGTERM to `matrix.ts` stops the pairings that are running, which stop their relay, browser and clients, and starts no further one.

One pairing that moves a single Object:

```sh
bun run.ts --publisher own-browser --subscriber own-rust --await-publish-done
```

`run.ts` generates the certificate, starts the relay, serves the pages and launches headless Chromium when a client is a page, sends one Object and stops everything it started, also when it is stopped by a signal or fails in a way nothing caught. It prints a `RESULT` line and exits 0 when the payloads match.

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

An own publisher leaves by sending PUBLISH_DONE. A moqtail publisher has no way to and ends its session, after which the relay sends PUBLISH_DONE to the subscriber on its own.

One run with a client that leaves:

```sh
bun disconnect.ts --scenario subscriber-leaves --publisher own-browser --leaver own-rust:kill --stayer own-browser
```

`disconnect.ts` starts the own relay and the clients like `run.ts`, plays the scenario, and prints a `RESULT` line with how each own subscription ended and what was measured, and a `SUMMARY` line that `matrix.ts` shows in its table. It exits 0 only if everything the scenario expects was seen.

| Option of `disconnect.ts` | Values |
|---|---|
| `--scenario` | `publisher-killed`, `last-object-then-close`, `subscriber-leaves`, `slow-subscriber` |
| `--publisher` | A client. `own-rust` by default |
| `--subscriber` | A client; give it once per subscriber. Used by every scenario but `subscriber-leaves` |
| `--leaver` | `subscriber-leaves` only, once per subscriber that leaves: `<client>:cancel` (the subscription ends, the session stays), `<client>:close` (the client closes its session in order) or `<client>:kill` (the process is killed, or the page closed) |
| `--stayer` | `subscriber-leaves` only: a client, once per subscriber that stays |
| `--with-publish-done` | `last-object-then-close` only: the publisher sends PUBLISH_DONE before it closes. `own-rust` always does and needs the flag; the moqtail clients cannot |
| `--port` | `44340`. UDP port of the relay |
| `--verbose` | off. Makes `own-rust` print the bytes of every stream |

Not every client can leave in every way. `own-rust` and `moqtail-rust` close their session only by having the Objects they were started for; `moqtail-browser` cancels by its `disconnect()`, which leaves its session open; only `own-browser` can cancel and keep going. A client that is asked for what it cannot do fails the run with a message that says so.

| File | Content |
|---|---|
| `harness.ts` | The certificate, the relay, the browser, the Rust client processes, and the teardown of all of them |
| `clients.ts` | The four clients behind one shape for a publisher and one for a subscriber |
| `run.ts` | The driver that moves one Object |
| `disconnect.ts` | The driver for the runs in which a client leaves |
| `matrix.ts` | The list of pairings, and the table |
| `page.ts`, `xmoq-page.ts` | The own page, and the contract between the drivers and both pages |

| Client | Code |
|---|---|
| `own-browser` | `page.ts`, on `@xmoq/moqt` |
| `own-rust` | `crates/moqt-client`, on `xmoq-moqt` |
| `moqtail-browser` | `peers/moqtail/browser/page.ts`, on the `moqtail` npm package |
| `moqtail-rust` | `peers/moqtail/rust-client/`, on the `moqtail` crate |

## Certificate

The relay runs on a self-signed certificate that the drivers create under `.cert/` and re-creates when less than a day of validity is left. No root CA is installed and no browser flag is set.

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
| `peers/moqtail/.tools/` | Binary of the moqtail relay. The own relay is built into `target/` of the repository root |
| `peers/moqtail/rust-client/target/` | Build of the moqtail Rust client. The own Rust client is built into `target/` of the repository root |
| `node_modules/` | Links into the `node_modules/` of the repository root |
