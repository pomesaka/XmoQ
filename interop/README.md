# Interop

Runs the own clients and parts through a relay with each other and with those of others, and checks what each run is to show, which is at the top of its driver (the list of files below). The own clients are the browser client on [`@xmoq/moqt`](../packages/moqt/) and the Rust client [`xmoq-moqt-client`](../crates/moqt-client/). The relay is the [moqtail](https://github.com/moqtail/moqtail) relay or the own relay [`xmoq-relay`](../crates/relay/). The browser clients run in a real Chromium. Results go to [`docs/experiments/`](../docs/experiments/).

This is the one entry point for interop runs. `spikes/moqtail/browser/run.mjs` is kept only to reproduce the Phase 0 records.

## Peers

| Part | Version | Source |
|---|---|---|
| Relay | `relay` 0.15.0 | `moqtail/moqtail` tag `relay@0.15.0`, built with the upstream `Cargo.lock` |
| Rust client (`moqtail-rust`, "moqtail Rust client" in the records) | `moqtail` 0.15.0, `wtransport` 0.7.2 | crates.io, `peers/moqtail/rust-client/Cargo.lock` |
| Test client (`moqtail-test`, "moqtail's test client" in the records) | `client` 0.15.0 | the same repository and tag as the relay. It is the publisher of the FETCH runs: the one client here that answers a FETCH. It publishes by itself and takes no commands |
| Browser client (`moqtail-browser`, "moqtail-ts client" in the records) | `moqtail` 0.13.0 | npm, the root `bun.lock` |
| Publisher and subscriber with an MSF catalog (`mlmpub`, `mlmsub`) | `moqlivemock` v0.16.1 | `Eyevinn/moqlivemock`, built from the tag with the Go of `mise.toml`. `mlmpub` is a server: a client connects to it without a relay. `mlmsub` is built with a patch of one line, without which it connects to no relay here. See [`peers/moqlivemock/`](peers/moqlivemock/README.md) |
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
cargo install --git https://github.com/moqtail/moqtail --tag relay@0.15.0 --locked --root interop/peers/moqtail/.tools client
# moqlivemock v0.16.1: a clone of the tag, `go build` of mlmpub, a patch of one line and `go build` of mlmsub, and its asset (peers/moqlivemock/README.md)
cargo build --locked --manifest-path interop/peers/moqtail/rust-client/Cargo.toml
cargo build --locked -p xmoq-moqt-client -p xmoq-relay -p xmoq-quiet-peer -p xmoq-chat -p xmoq-agent
cargo build --locked --release -p xmoq-watch -p xmoq-transcriber
crates/speech/scripts/fetch-model.sh tiny    # the models of the transcriber's run, into ~/.cache/xmoq/whisper/
crates/speech/scripts/fetch-model.sh silero-v5.1.2
bun install --frozen-lockfile
bun run --cwd interop playwright-core install --no-shell chromium
bun interop/matrix.ts
```

The Chromium is the build that the pinned `playwright-core` drives; it goes to Playwright's cache directory, outside the repository.

**`mlmsub` is rebuilt whenever the patch changes or is not there**, and `interop/peers/moqlivemock/.tools` is not part of a working copy: moving a working copy to a revision without the patch and running the script there leaves a `mlmsub` that connects to no relay here (`expected webtransport, got: Some("webtransport-h3")`), also after moving back. `scripts/ci/interop.sh` builds it again; a driver that is run by itself (`bun catalog.ts`) does not.

`interop` and `packages/*` are one Bun workspace with one `bun.lock` in the repository root, so `@xmoq/moqt` and `@xmoq/msf` here are the working copies of `packages/moqt` and `packages/msf`.

## Checks

`mise run check:ts` lints and type-checks every `.ts` file of this directory. It does not perform a run. `peers/` is outside lint and formatting, because it holds copies of code written for moqtail that are kept as they came.

The runs are the CI job `interop`, which calls `mise run interop`. On a pull request that changes nothing the run depends on (`scripts/ci/interop-scope.sh` has the list), the job ends green without running the pairings and says so in its summary and in a notice. A job that ran shows the table of pairings in its summary and keeps their logs as the artifact `interop-logs`. Its last step fails if the job did neither.

## Run

Every pairing, from `interop/`:

```sh
bun matrix.ts
```

`matrix.ts` starts a driver once per pairing and relay, each on its own port, prints a table, and exits 0 only if every pairing ended as it expects. The pairings and what each expects are the list in `matrix.ts`; its flags (`--relay`, `--only`, `--base-port`) are at its top. The log of each pairing is in `.logs/matrix/`. A SIGINT or SIGTERM to `matrix.ts` stops the pairings that are running, which stop their relay, browser and clients, and starts no further one.

One pairing that moves a single Object, and one run with a client that leaves:

```sh
bun run.ts --publisher own-browser --subscriber own-rust --await-publish-done
bun disconnect.ts --scenario subscriber-leaves --publisher own-browser --leaver own-rust:kill --stayer own-browser
```

Every driver starts what it needs (the certificate, the relay, the pages in a headless Chromium, the clients) and stops everything it started, also when it is stopped by a signal. How to start each one alone, and what each of its runs has to show, is at the top of the file ([Files](#files)).

| Client | Code |
|---|---|
| `own-browser` | `page.ts`, on `@xmoq/moqt` |
| `own-rust` | `crates/moqt-client`, on `xmoq-moqt` |
| `moqtail-browser` | `peers/moqtail/browser/page.ts`, on the `moqtail` npm package |
| `moqtail-rust` | `peers/moqtail/rust-client/`, on the `moqtail` crate |

One side of a pairing has to be an own client: a run between two moqtail peers checks nothing of this repository.

**The runs wait for lines of the own relay's log**, so changing the form of such a line breaks them. What a run counts in the relay's log, or expects not to be there, it counts before a line of the relay that it names, and never "so far": the lines of the relay and of a client reach the driver in no order, and a count taken when a client has reported something can miss the relay's line that is on its way. `RelayLog` in `harness.ts` has no other way to count, so a run that does not name such a line does not pass the type check. A run with no step of its own after the moment it checks has `connectedNow` connect a client, whose session line is the fence.

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

<!-- The list below is written by scripts/ci/interop-index.sh (mise run fmt). -->

## Files

The drivers that `matrix.ts` starts. How to start one alone, and what each of its runs has to show, is at the top of the file.

| Driver | What it runs |
|---|---|
| [`agent-frames.ts`](agent-frames.ts) | Pictures of a broadcast for an agent: a page publishes its broadcast (`publishBroadcast`: a catalog, 640x360 H.264 and Opus) through the own relay, and `xmoq-agent --namespace xmoq/watch` is asked for a picture with `/frame` on its stdin, twice, and once more after it moved to another relay. |
| [`agent-meeting-speech.ts`](agent-meeting-speech.ts) | Who speaks in a meeting, for an agent: two pages each publish a broadcast of a picture and a voice (`publishBroadcast` with the voice of `fixtures/speech/`, the same for both) under `xmoq/meet/r/<id>`, the transcriber listens under `xmoq/meet/r` with the smallest model, and `xmoq-agent --meeting r` follows its transcript and asks for `/frame` alone. |
| [`agent-meeting.ts`](agent-meeting.ts) | An agent in a meeting: two pages each publish a broadcast of their own (`publishBroadcast`, 640x360) under `xmoq/meet/r/<id>`, as the participants of a meeting do (`docs/scale-and-meetings.md`, Part 2), and `xmoq-agent --meeting r` is told of them and asked for their pictures. |
| [`agent.ts`](agent.ts) | The reader of a room for an agent (`crates/agent`, `xmoq-agent`) through the own relay and the gatherer (`xmoq-chat`): a page takes part in the chat of room r as `p`, and `xmoq-agent --chat-room broadcast/r --name claude` writes what happens as lines of JSON on stdout and in `<dir>/events.jsonl`. |
| [`auth-edge.ts`](auth-edge.ts) | Two own relays that both ask for tokens (`--auth-key-hex`, the same key): the relay of the run, and a second one below it (`--parent`), on the run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET. |
| [`catalog.ts`](catalog.ts) | The MSF catalog of a broadcast is published and read through `@xmoq/msf`, with a side that is not ours where there is one. |
| [`chat-edge.ts`](chat-edge.ts) | The chat through two relays: the gatherer (`xmoq-chat`) at the own relay of the run, and the participants and the viewers at a second own relay below it (`--parent`), on the run's port plus MOQLIVEMOCK_SIDE_PORT_OFFSET. |
| [`chat-pages.ts`](chat-pages.ts) | Pages chat over the tracks of the chat through a relay and the gatherer (`crates/chat`, `xmoq-chat`): each page is a participant with `joinChat` of `@xmoq/chat`, which the demo's pages use. |
| [`chat-tokens.ts`](chat-tokens.ts) | Pages join the chat through the own relay when it asks for tokens (`--auth-key-hex`), with the two tokens the demo's pages get from the room: `sub` for `xmoq/chat/r` on the connection, and `pub` for `xmoq/chat/r/<name>` on the PUBLISH_NAMESPACE alone (`publishNamespaceToken` of `joinChat`). |
| [`chat.ts`](chat.ts) | The gatherer of the chat (`crates/chat`, `xmoq-chat`) through a relay: own-rust clients are the participants of one room, each announcing `xmoq/chat/r/<name>` and sending a line on its track `said` for every line of its input; own-rust clients are the viewers of the room's `lines`. |
| [`disconnect.ts`](disconnect.ts) | Interop driver for the runs in which a client leaves: a publisher that is killed or closes right after its last Object, a subscriber that leaves while others stay, a subscriber that takes nothing, and two publishers of one track. |
| [`fetch-rust.ts`](fetch-rust.ts) | The own Rust client sends one FETCH through a relay, a Standalone one or a Joining one for its own subscription, and what it prints is compared with what is known of that relay and that publisher. |
| [`fetch.ts`](fetch.ts) | An own page sends one FETCH through a relay, and what it gets is compared with what is known of that relay and that publisher. |
| [`half-object.ts`](half-object.ts) | A viewer comes while the own relay is receiving an Object, and the relay counts that Object from its first byte (§5.1.2). |
| [`meeting-pages.ts`](meeting-pages.ts) | A meeting of three pages: each publishes a broadcast of its own (`publishBroadcast`, 640x360 and a tone) under `xmoq/meet/r/<id>`, and takes part with `createMeeting` of `@xmoq/meeting` (`attendMeeting`), as the participants of a meeting do (`docs/scale-and-meetings.md`, Part 2). |
| [`namespaces.ts`](namespaces.ts) | Namespace discovery through a relay (draft 18 §6.1, §10.16 to §10.18): a watcher sends SUBSCRIBE_NAMESPACE for the prefix (xmoq) and is told of the namespace (xmoq, phase0) when an own-rust publisher announces it, and of its end when the publisher goes. |
| [`new-group.ts`](new-group.ts) | A subscriber asks for a new Group with NEW_GROUP_REQUEST (transport §10.2.13), and the own relay passes it on to the publisher. |
| [`no-answer.ts`](no-answer.ts) | A request the peer does not answer: the own clients give it up after the time its answer gets, and their session stays open. |
| [`quiet-peer.ts`](quiet-peer.ts) | The own clients against a peer that stays quiet: `xmoq-quiet-peer` (`crates/quiet-peer`), a server that takes their session and sends no SETUP, or answers no request. |
| [`rendezvous.ts`](rendezvous.ts) | A SUBSCRIBE that comes before its publisher: with RENDEZVOUS_TIMEOUT the relay holds it (§10.2.6 of draft-ietf-moq-transport-18). |
| [`run.ts`](run.ts) | Moves one MOQT Object through a relay between two clients, at least one of which is an own client, and compares the received payload with the sent one. |
| [`track-status.ts`](track-status.ts) | TRACK_STATUS (draft 18 §10.14) through a relay, asked by an own client (`--client own-rust`, the Rust client's `--mode track-status`, or `--client own-browser`, `MoqtConnection.trackStatus`). |
| [`transcriber.ts`](transcriber.ts) | The transcriber (`crates/transcriber`, `xmoq-transcriber`) through the own relay, and through moqtail's for a catalog refused at first: a page publishes a broadcast of sound alone (`publishVoice`): a voice that says "Hello from the relay." (`fixtures/speech/`) and 1.5 s of silence, over and over. |
| [`video-rust.ts`](video-rust.ts) | The own Rust client takes a video track a page publishes, through the own relay, and writes its frames as an H.264 byte stream (`--annex-b-out`). |
| [`video.ts`](video.ts) | A page plays a LOC video track: the Objects of a subscription go through the receiver of `@xmoq/media` into the H.264 decoder of the browser. |
| [`watch-rust.ts`](watch-rust.ts) | The viewer for the terminal, `xmoq-watch`, plays a broadcast a page publishes, through the own relay: it finds the video and the audio track in the catalog of MSF, takes both on one session, and puts them out by the Timestamps of LOC, with no terminal and no sound output (`--out /dev/null`, `--sound off`). |
| [`when-full.ts`](when-full.ts) | A relay that is full sends the next session to another (`--when-full`): a GOAWAY right after its SETUP names that relay, with `sent-away=1` in the query, and a relay that is full itself closes a session that came so rather than send it on. |

The parts they share.

| File | What it is |
|---|---|
| [`agent-pictures.ts`](agent-pictures.ts) | What the interop runs with `xmoq-agent` check of its pictures: its `frame` lines, the PNG files they name, and what a page decodes of them. |
| [`clients.ts`](clients.ts) | The four clients of the interop runs behind one shape each for a publisher and a subscriber, so that a driver can put any of them in any role. |
| [`fetch-publishers.ts`](fetch-publishers.ts) | The two publishers that answer a FETCH, as the drivers of the FETCH runs start them (fetch.ts for a page that fetches, fetch-rust.ts for the own Rust client). |
| [`harness.ts`](harness.ts) | What every interop driver needs: the certificate, the relay, the browser with its pages, the Rust clients, and the teardown of all of them. |
| [`matrix.ts`](matrix.ts) | Runs the interop drivers, each run with its arguments and its relay, and prints a table of the outcomes. |
| [`page.ts`](page.ts) | Own browser client of the interop run: a publisher and a subscriber built on @xmoq/moqt. |
| [`voice-file.ts`](voice-file.ts) | The voice of `fixtures/speech/`, as the pages take it (`publishVoice`, `publishBroadcast` with a voice). |
| [`xmoq-page.ts`](xmoq-page.ts) | Contract between the driver (`run.ts`) and every browser page it drives. |
