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

`matrix.ts` starts a driver once per pairing and relay, each on its own port, prints a table, and exits 0 only if every pairing ended as expected. The log of each pairing is in `.logs/matrix/`. The pairings run one after the other, except seven that mostly wait, four for the relay to give up on a client that vanished (6 to 9 seconds each), one for a publisher that answers the relay 3 seconds late, one for a source that drains for 2 seconds, and one for a page to give up on late streams (5 seconds): those start together after the pairings that move one Object, and go on beside the ones that follow.

| Pairings, through each relay | What is expected |
|---|---|
| Every publisher with every subscriber, except moqtail with moqtail (12) | The received payload equals the sent one. Where the subscriber is an own client, the publisher then leaves and the subscriber has to see PUBLISH_DONE |
| One publisher with three subscribers: `own-rust` to `own-browser`, `moqtail-browser` to `own-rust` (2) | Every subscriber receives the payload and then sees PUBLISH_DONE |
| Each own client to each own client, with two Object Properties on the Object that no relay knows: an odd type with bytes and an even type with an integer (4) | The subscriber gets the payload and the two properties, with the types, values and order that were sent (draft 18 §2.5: a relay forwards a property it does not support unchanged), and then sees PUBLISH_DONE. The run tells apart an Object that came without properties, one that came with others, and one that did not come |
| `own-browser` and `own-rust` subscribing to a track nobody published (2) | The run fails, and its log shows REQUEST_ERROR `0x10` |

The runs in which a client leaves go through the own relay only, because the driver waits for lines only that relay prints. Their names begin with `leaving:`. What each has to show, and why, is in the [record](../docs/experiments/2026-10-10-disconnects-through-own-relay.md).

| Runs with a client that leaves, or with two publishers of the track (26) | What is expected |
|---|---|
| The publisher is killed, or its page closed, after its Objects arrived (3) | Every own subscriber sees PUBLISH_DONE `TRACK_ENDED` with the Stream Count of the streams it got, and a stream the publisher left open ends with a reset with CANCELLED. A killed publisher has to be noticed within 10.5 seconds, and a killed subscriber within 7.5 |
| The publisher sends one Object and closes its session at once (4) | With PUBLISH_DONE before the close (`own-browser`, `own-rust`): every subscriber receives the Object, and every own subscriber then sees PUBLISH_DONE. Without it (`own-browser`, `moqtail-rust`): the Object reaches every subscriber or none, since nothing tells such a publisher that the relay read its last stream, and every own subscriber sees PUBLISH_DONE either way. The summary of the run says which it was; a lost Object was seen once in CI and in 1 of 200 runs on a loaded machine |
| Subscribers leave, by cancelling, by closing their session or by being killed, while others stay (4) | The publisher goes on, and the subscribers that stay receive what is sent after the others left, also before the relay has noticed that a killed process is gone, which the run checks |
| `own-browser` cancels its subscription (1) | The relay ends the request, opens no stream for what the publisher sends later, and keeps the session. Where the relay printed the STOP_SENDING it got on the request stream, its code has to be CANCELLED (`0x1`). The relay does not always print it: it did in 53 of 60 rounds, and a round without the line is not judged on the code. For `moqtail-browser`, which cancels in two of the four runs above, the line has to be there with code `0x0` |
| An `own-rust` subscriber is stopped with SIGSTOP and takes nothing (1) | The relay ends that subscription with PUBLISH_DONE `TOO_FAR_BEHIND` and keeps its session; the other subscribers receive every Object |
| An `own-browser` page subscribes and never reads its Objects (1) | The same. The Objects are small, one a stream: with one large Object a stream the page is not told that its subscription ended, a known gap of `@xmoq/moqt`, and that case is not run. The streams the relay cut short have to carry `TOO_FAR_BEHIND`, read from the relay's log, and the relay opens no stream for what is sent after. The run takes 8 to 11 seconds, 5 of them the library's wait for late streams, and goes on beside the runs that wait for the relay's idle timeout |
| Two publishers send the same Group, one after the other: two `own-rust`, `moqtail-rust` and then `own-rust`, `own-browser` and then `own-rust` (3) | Both get PUBLISH_OK. The relay takes the Group from the one that sent it first, stops the stream of the other (`STOP_DATA … code=0x1` in its log) and opens one stream of the Group towards each subscriber; every own subscriber has one Object for the Group at the end, and where both publishers are own clients it is the first one's. When the first publisher has left and the relay has closed its session, no subscriber has been told; when the second has left, every own subscriber sees PUBLISH_DONE `TRACK_ENDED` with Stream Count 1. An own second publisher, whose one stream the relay stopped, still sees the relay finish its request stream after PUBLISH_DONE. The moqtail client is never the second one: it ends with an error when its stream is stopped before it has written it out |
| An `own-rust` publisher sends Group 0 again when another has sent ten Groups (1) | The same, for a Group the relay no longer remembers a source for: the stream is stopped, and the subscriber keeps the Object of the first publisher and gets no second stream of Group 0 |
| The first of two publishers sends Group 0 and goes, by PUBLISH_DONE, by being killed, or as a page that is closed with a stream of Group 1 open; the second then sends the next Group (3) | The relay sends no subscriber PUBLISH_DONE while the second is there. The next Group arrives with the second publisher's payload. A stream the first left open ends with a reset with CANCELLED. When the second has left: PUBLISH_DONE `TRACK_ENDED`, the Stream Count of every Group, the reset one among them. A killed publisher has to be noticed within 10.5 seconds |
| The first of two `own-rust` publishers sends ten Groups and leaves with PUBLISH_DONE; the second then sends the same ten again, and after them the next (1) | No subscriber is told when the first leaves. All ten streams the second sends again are stopped (`STOP_DATA … code=0x1`), the two for Groups the relay no longer remembers a source for among them, and none is opened towards the subscriber a second time. The next Group arrives from the second. At the end: PUBLISH_DONE `TRACK_ENDED` with eleven Groups |
| A second publisher joins a track whose subscribers already have an Object, and sends the next Group (1) | The PUBLISH is accepted, the Group arrives with the second publisher's payload, and the subscriptions end when both have left |
| An `own-rust` publisher announces the namespace and answers the relay's SUBSCRIBE 3 seconds late; another sends PUBLISH for the track meanwhile (1) | The subscribers get no SUBSCRIBE_OK while only the first publisher is there. The PUBLISH answers them, before the first publisher has answered the relay. They get Group 0 from the one that sent PUBLISH and, when the other has answered, Group 1 from that one. The track ends when both have left |
| An `own-rust` publisher sends a Group, says PUBLISH_DONE with the Stream Count of a publisher that cannot count, publishes the track anew on the same session at once, under the next Track Alias or the same, and sends another Group (2) | The PUBLISH is accepted and the subscribers stay: they get no PUBLISH_DONE then, and both Groups within one subscription. The relay resets the first PUBLISH stream with CANCELLED, at once under the same alias and after its 2 seconds of waiting under another. When the publisher leaves, the subscribers see PUBLISH_DONE with Stream Count 2 |

Each of these runs ends by moving one more Object over the same track with a fresh publisher and two fresh subscribers, and by having that publisher leave. One subscriber stays and has to see PUBLISH_DONE: a relay that still held a source of the run would take the fresh publisher as one more source and pass the Object, but it could not end the track when that publisher leaves.

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
| `--object-properties` | off. The Object carries the two Object Properties, and the subscriber has to report the same. Both clients have to be own clients, the ones that send properties with content and report the ones they get |
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
| `--scenario` | `publisher-killed`, `last-object-then-close`, `subscriber-leaves`, `slow-subscriber`, `same-group-twice`, `second-source`, `publish-while-asked`, `publish-anew` |
| `--publisher` | A client, `own-rust` by default. `same-group-twice`, `second-source` and `publish-while-asked` take it twice, the first publisher and the second (in `publish-while-asked` the first is the one that is asked and has to be `own-rust`); the other scenarios take one and refuse more |
| `--subscriber` | A client; give it once per subscriber. Used by every scenario but `subscriber-leaves` |
| `--leaver` | `subscriber-leaves` only, once per subscriber that leaves: `<client>:cancel` (the subscription ends, the session stays), `<client>:close` (the client closes its session in order) or `<client>:kill` (the process is killed, or the page closed) |
| `--stayer` | `subscriber-leaves` only: a client, once per subscriber that stays |
| `--with-publish-done` | `last-object-then-close` only: the publisher sends PUBLISH_DONE before it closes. `own-rust` always does and needs the flag; the moqtail clients cannot |
| `--same-alias` | `publish-anew` only: the new PUBLISH has the Track Alias of the first. Without it, the next one |
| `--slow` | `slow-subscriber` only: `stopped-own-rust` (default), an own-rust client whose process is stopped, or `unread-own-browser`, a page that subscribes and never reads |
| `--first-sends` | `same-group-twice` and `second-source`: how many Groups the first publisher sends before the second sends; 1 by default. In `same-group-twice`, above 1 the two publishers send different payloads |
| `--second-resends` | `second-source` only: the second publisher starts at Group 0 and sends every Group of the first again before it sends the next one |
| `--first` | `second-source` only, what the first publisher does before the second sends: `done` (default; PUBLISH_DONE), `kill`, `kill-with-open-stream` (own-browser: it sends one more Group on a stream it leaves open, and its page is closed) or `stays` |
| `--second-joins` | `second-source` only: `before` (default) or `after` the subscribers have their first Object |
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
