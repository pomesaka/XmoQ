# Interop

Moves MOQT draft 18 Objects through a relay between clients, at least one of which is an own client, and checks that the received payload equals the sent one. One set of runs moves a single Object between every pair of clients; another has a client leave on the way and checks what the others see. The own clients are the browser client on [`@xmoq/moqt`](../packages/moqt/) and the Rust client [`xmoq-moqt-client`](../crates/moqt-client/). The relay is the [moqtail](https://github.com/moqtail/moqtail) relay or the own relay [`xmoq-relay`](../crates/relay/). The browser clients run in a real Chromium. Results go to [`docs/experiments/`](../docs/experiments/).

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
cargo build --locked -p xmoq-moqt-client -p xmoq-relay -p xmoq-quiet-peer -p xmoq-chat
cargo build --locked --release -p xmoq-watch
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

`matrix.ts` starts a driver once per pairing and relay, each on its own port, prints a table, and exits 0 only if every pairing ended as expected. The log of each pairing is in `.logs/matrix/`. The pairings run one after the other, except six that mostly wait, four for the relay to give up on a client that vanished (6 to 9 seconds each), one for a source that drains for 2 seconds, and one for a page to give up on late streams (5 seconds): those start together after the pairings that move one Object, and go on beside the ones that follow.

| Pairings, through each relay | What is expected |
|---|---|
| Every publisher with every subscriber, except moqtail with moqtail (12) | The received payload equals the sent one. Where the subscriber is an own client, the publisher then leaves and the subscriber has to see PUBLISH_DONE |
| One publisher with three subscribers: `own-rust` to `own-browser`, `moqtail-browser` to `own-rust` (2) | Every subscriber receives the payload and then sees PUBLISH_DONE |
| Each own client to each own client, with two Object Properties on the Object that no relay knows: an odd type with bytes and an even type with an integer (4) | The subscriber gets the payload and the two properties, with the types, values and order that were sent (draft 18 §2.5: a relay forwards a property it does not support unchanged), and then sees PUBLISH_DONE. The run tells apart an Object that came without properties, one that came with others, and one that did not come |
| An `own-browser` page that only announces the namespace, with `moqtail-rust` and with `own-rust` as the subscriber (2) | The relay asks the page for the track with a SUBSCRIBE of its own, the page accepts it (`serve` of `@xmoq/moqt`), and the subscriber gets the Object the page sends on that subscription. Both relays ask a publisher that only announced |
| `own-browser` and `own-rust` subscribing to a track nobody published (2) | The run fails, and its log shows REQUEST_ERROR `0x10` |

Three runs are about an MSF catalog, and their names have `catalog:` in them; what each has to show is at the top of `catalog.ts`. In one, an own page connects to `mlmpub`, the publisher of moqlivemock, without a relay, and reads its catalog with `followCatalog` of `@xmoq/msf`, a SUBSCRIBE and a Joining FETCH with offset 0: it has to have the catalog once, the same one that was captured from this publisher (`fixtures/msf-01/`), the page has to say that its Joining FETCH was served with one Object, and `mlmpub` that it served one. In the other two an own page publishes a catalog with `createCatalogPublisher` through the own relay: another page reads it and then the catalog that replaces it, and `mlmsub`, the subscriber of moqlivemock, reads it and writes it out. `mlmpub` takes a second port, its own plus 1000: two working copies that run at once need base ports further apart than that. The records: [reading](../docs/experiments/2026-10-11-a-page-reads-a-catalog.md), [publishing](../docs/experiments/2026-10-11-a-page-publishes-a-catalog.md).

Thirteen runs are about video, with `video:` in their names; what each has to show is at the top of `video.ts`, and for the one with the own Rust client at the top of `video-rust.ts`: there a page publishes, and the Rust client writes the frames as an H.264 byte stream (`--annex-b-out`), which `ffprobe` reads where the machine has it. In one, an own page connects to `mlmpub`, reads its catalog, and plays one of its H.264 tracks with `playVideoTrack` of `@xmoq/media`: the frames come out of the H.264 decoder of the browser, 1280x720 and 40 ms apart. Nothing is drawn.

Two runs are about the terminal viewer, with `watch:` in their names; what each has to show is at the top of `watch-rust.ts`.

Eleven runs are about a SUBSCRIBE that comes before its publisher, or after it ended the track and before it or another returns, with `rendezvous:` in their names; what each has to show is at the top of `rendezvous.ts`. With RENDEZVOUS_TIMEOUT the own relay holds the SUBSCRIBE of an own page, of the own Rust client and of the terminal viewer until a publisher comes, or refuses it with `TIMEOUT` when none does; without the parameter it refuses at once.

Two runs are about a subscriber that asks for a new Group, with `new group:` in their names; what each has to show is at the top of `new-group.ts`. The own Rust client subscribes with NEW_GROUP_REQUEST to a track that the own Rust client, or an own page, published with DYNAMIC_GROUPS; the own relay sends the publisher a REQUEST_UPDATE on the stream of its PUBLISH, and the publisher answers it, reports it and goes on. Neither publisher begins a Group for it. `moqtail` has no part in these runs: its relay and its test client at `relay@0.15.0` neither send nor act on the parameter (read in the source; its libraries encode it).

Thirty-four runs are against a peer that stays quiet, twenty-seven of them with the own clients, with `quiet peer:` in their names; what each has to show is at the top of `quiet-peer.ts`. The peer is `xmoq-quiet-peer` (`crates/quiet-peer`), a server of this repository that is built for these runs alone: it takes a session and sends no SETUP, or answers no request, and prints what reached it. An own page, the own Rust client and the terminal viewer close their session with `CONTROL_MESSAGE_TIMEOUT` when no SETUP comes; the page and the Rust client end a PUBLISH nobody answers with PUBLISH_DONE and a FIN, and give up a FETCH nobody answers. In two more the peer sends a datagram of an unknown type, and the own Rust client and the terminal viewer close the session with `PROTOCOL_VIOLATION`; in one the quiet peer sends the own relay one, and the relay does the same. In three more the peer allows no bidirectional stream, and the three clients fail a SUBSCRIBE they cannot send as not sent. In two more the peer allows no room for a subgroup stream: the page and the Rust client do not send that Subgroup, say so, and go on. In four more the peer sends GOAWAY on its first session (`--quiet goaway`) and refuses its requests with `GOING_AWAY`: the three clients close that session with `NO_ERROR`, and the page (`keepPlaying`, and `keepPublishing` in the fourth) and the terminal viewer connect to the URI it names. In three more the GOAWAY is on the stream of the client's first SUBSCRIBE (`--quiet goaway-request`), with the same outcome. In six more the peer sends the client a request with a Request ID §10.1 does not allow (`--quiet request-id-parity`, `request-id-again`), and the page, the Rust client and the terminal viewer close the session with `INVALID_REQUEST_ID`. Their waits are set to 1.5 seconds. In the last two the peer is the client of the own relay: one that sends no SETUP is closed with `CONTROL_MESSAGE_TIMEOUT`, and one that sends its SETUP and then holds nothing is closed with `NO_ERROR`, while a publisher that announced a namespace stays. With `--announce-per-second` the same program is a client that announces a namespace and withdraws it at that rate; one run of the matrix uses it (`--case relay-many-announcements`: the relay refuses what goes beyond five at once and one a second), and what it measured is in `docs/experiments/2026-10-13-a-session-that-announces-a-namespace-again-and-again.md`. With `--ask-per-second` it asks for another track each time, with `--announce-and-refuse yes` it is a publisher that has nothing, and with `--announce-and-stay-quiet yes` one that answers nothing; one run of the matrix uses the first and the last (`--case relay-one-session-asks-for-much`: the relay keeps one session to its share of a publisher, and another viewer's requests reach it), and and what they measured is in `docs/experiments/2026-10-13-a-session-that-asks-for-a-new-track-again-and-again.md`.

Three runs are about a request that is not answered, with `no answer:` in their names; what each has to show is at the top of `no-answer.ts`. A publisher behind the own relay holds its answer back, and an own page, the own Rust client and the terminal viewer give their SUBSCRIBE up after the time its answer gets, which the driver sets to 1.5 seconds so that the relay, which waits 5 for a publisher, does not answer first.

In four of the others, **an own page encodes frames it paints and publishes them** with `createVideoTrackEncoder` of `@xmoq/media` (H.264, 320x180, 25 frames a second, a key frame every second), and another own page plays them, through each relay and with each way of putting Objects onto streams (a Group on a stream, an Object on a stream). Every frame shows its number as 8 bars, black or white, which the viewer reads from the decoded picture: 50 frames or more, each with a later number than the one before it and that many times 40 ms after it, with wall-clock times, although the viewer comes in the middle of a Group. **Every Object the publisher wrote has to reach the viewer**: the publisher ends the track, and then what it wrote is counted against what the viewer gave its decoder or dropped. A viewer whose decoder is behind drops the rest of a Group (`decoder-behind`, `late`), which a slow machine does and the run lets through; no more numbers may be passed over than Objects were dropped (the publisher asks its encoder for a key frame when the relay subscribes). Through the own relay the viewer takes the codec from the catalog the publishing page puts out; **through the moqtail relay it is told the codec by the driver**, because that relay answers the Joining FETCH for the catalog for Group 0 and the viewer gets no catalog (the [implementation notes](../docs/moqt-implementation-notes.md), "The `moqtail` relay 0.15.0 answers a FETCH in five ways", row a). A page that reads a catalog through the moqtail relay had not been run before; the runs with `catalog:` in their names go through the own relay or none.

**`mlmsub` of moqlivemock takes the video of a page** (`--video to-mlmsub`, own relay): it reads the page's catalog, selects the track, and prints the Groups and Objects it gets: three Groups in a row a second apart, the Objects of the first 40 ms apart, with wall-clock Timestamps. **It cannot write that video out**: with `-videoout` it takes a payload to have a length before every NAL unit, where the page sends start codes, and stops reading the track at the first Object (the [record](../docs/experiments/2026-10-11-pages-send-and-play-video.md)); the run is without that flag.

**The relay goes and comes back** (`--video reconnect`, own relay): a page publishes with `keepPublishing` and another plays with `keepPlaying` and `createPlayout` of `@xmoq/media`; the driver stops the relay and starts it anew on the same port. Both pages are told that their session ended, fail to connect while the relay is gone, and go on with a second session, on which the viewer shows 25 frames or more, each after the one before it.

**moqtail's relay sends its sessions away** (`--video goaway-from-moqtail`, `--video goaway-from-moqtail-rust`, moqtail relay): moqtail's relay is started with `--redirect-uri` naming the own relay, which the driver starts on the run's port plus 1,000, and is sent SIGTERM once the clients play. It then sends every session a GOAWAY on the control stream (that URI, a Timeout of 10,000 ms, Request ID 0) and exits 10 seconds later. A page that publishes with `keepPublishing` and one that plays with `keepPlaying` both move to the own relay, and the viewer shows 25 frames there; with `--video goaway-from-moqtail-viewer-first` the publisher waits 3 s before it connects, and the viewer, refused at the own relay meanwhile, tries that relay again and not moqtail's; the own Rust client, which subscribes, prints the GOAWAY and leaves with 0. The terminal viewer is not in these runs: through moqtail's relay its Joining FETCH of the catalog asks the page for Group 0, where the page's Groups are numbered by the clock, and no catalog comes.

**The own relay sends its sessions away** (`--video goaway-from-own`, `--video goaway-from-own-rust`, `watch-rust.ts --watch goaway`, own relay): the same, with the own relay started with `--redirect-uri` naming a second own relay. Its GOAWAY has a Timeout of 10,000 ms (`--goaway-timeout-ms`) and the smallest Request ID of the client it has not seen (2 for own-rust, after its SUBSCRIBE 0); it prints `GOING_AWAY sessions=<n> ...`, refuses new connections, and the run holds it to end with 0 within its Timeout and 2 seconds more. The terminal viewer, with a page publishing the broadcast through each relay, moves to the second and plays the same broadcast there.

**The terminal viewer chats on MoQ tracks** (`watch-rust.ts --watch chat`, own relay): the gatherer (`xmoq-chat`) runs beside the relay and a page publishes the broadcast. Two terminal viewers of room `r` (`--chat-room r --say ...`): the first comes before the gatherer has published the room's `lines`, and its SUBSCRIBE is held for it (RENDEZVOUS_TIMEOUT); the second comes later and gets the first one's line by its Joining FETCH. Each shows `0 wa: a1` and `1 wb: b1`, once each. Through moqtail's relay as well (`--relay moqtail`), with the viewers naming the tracks of the broadcast, since the catalog does not come through it; whether the first viewer's SUBSCRIBE is refused before the gatherer has published the room depends on which comes first, and is not looked at.

**A viewer that comes late** (`--video late-viewer`, own relay): one page watches already; a second comes in the middle of a Group and plays with the Joining FETCH of `playVideoTrack`. The relay answers from the Group it keeps, and the second viewer's first frame is one that was made before it came (about 740 ms before, in the runs here), with nothing dropped as `no-key-frame`.

**A viewer that comes late and asks for a new Group** (`--video late-viewer-new-group`, own relay): the same, with a track that has dynamic Groups and a key frame every 10 seconds, and a second viewer that sends no Joining FETCH and has NEW_GROUP_REQUEST 0 in its SUBSCRIBE. The relay passes the request on as a REQUEST_UPDATE on the stream of its own SUBSCRIBE to the page, the page begins a Group, and the viewer's first frame comes within a second.

**Audio** (`--video audio`, own relay; `audio:` in its name): a page makes a 440 Hz tone, encodes it as Opus and publishes it with `createAudioTrackEncoder` of `@xmoq/media`, and another decodes it with `playAudioTrack`: 100 pieces of 20 ms, one after the other, with wall-clock times, and not silence (a sample of 0.5 of full scale). Nothing is sounded.

**A page whose key frames are all over what it sends** (`--video key-frames-too-large`, own relay, nobody viewing): with payloads of 100 bytes at most, the page is told once, at the third key chunk in a row that was not sent (`onKeyChunksNotSent`), and goes on encoding.

**The same four with a picture an encoder cannot make small** (`--picture noise`): 1280x720, the lower half random pixels that change with every frame, at 30,000,000 bits a second, and a viewer that takes Objects of 1 MiB at most. A Group is about 3.9 MB and the largest chunk about 639,000 bytes here; the same has to hold, with nothing left unsent as `too-large` or `behind`, and a Group of more than 2 MiB.

The runs in which an own page sends a FETCH have names that begin with `fetch:`. What each has to show is written, with where it comes from, at `EXPECTED` in `fetch.ts`, before the runs; the [record](../docs/experiments/2026-10-11-fetch-from-a-page.md) has what the runs showed. The page reads the fetch stream with `decodeFetchBody` of `@xmoq/moqt`. The runs in which the own Rust client sends the FETCH, a Standalone one or a Joining one for its own subscription, are in the same group; theirs is `EXPECTED` in `fetch-rust.ts`, and the [record](../docs/experiments/2026-10-11-fetch-from-the-rust-client-through-the-matrix.md) has what they showed.

| Runs with a FETCH (18) | Through | What is expected |
|---|---|---|
| The page asks for {0, 1} to {1, 2} of what moqtail's test client publishes | both relays | FETCH_OK with End Location {1, 2}, the four Objects of the range with the publisher's payload, a FIN |
| The same publisher, {0, 0} to {2, 0} with FILL_TIMEOUT 0 | own relay | FETCH_OK with End Location {2, 0} and one End of Unknown Range indicator at {2, 2^64 - 1}: the relay asks no publisher, and its cache is not used for a FETCH with FILL_TIMEOUT 0 |
| `moqtail-rust`, which answers no FETCH, sends Group 0, and a second publisher, `own-rust`, Group 1; the page asks for Group 0 and gives its FETCH up after a second | own relay | The relay holds Group 1 alone, so it asks the first publisher for Group 0. The FETCH ends as cancelled for the page, and the relay cancels the FETCH it had sent the publisher |
| `own-rust`, which refuses a FETCH, sends Groups 0 and 1; the page asks for Group 0 | own relay | The relay holds Group 1 alone and asks the publisher: REQUEST_ERROR `DOES_NOT_EXIST` (`0x10`), and the relay's log has the FETCH it sent |
| `own-rust` sends Group 0; the page asks for {0, 0} to {1, 0}, the latest Group | both relays | **The relay answers from what it holds**: FETCH_OK with End Location {0, 1} and the one Object. The own relay's log has the fetch stream it wrote (`WRITE_FETCH_STREAM … pieces=1`) and no FETCH to the publisher |
| A track nobody published | both relays | REQUEST_ERROR `DOES_NOT_EXIST` |
| **An own page is the publisher**: it only announced, serves the track when the relay asks, and answers a FETCH from the Objects it has sent (`serve` of `@xmoq/moqt`). It has sent Object 0 of Groups 0 to 2 to another subscriber before the page that fetches comes. The page asks for {0, 0} to {2, 1} | own relay | The relay asks the publisher. FETCH_OK with End Location {2, 1}, the three Objects, a FIN |
| The page that fetches subscribes and sends a Relative Joining FETCH, one Group back | own relay | The relay makes a Standalone FETCH for {1, 0} to {2, 1} of it, from the LARGEST_OBJECT of the SUBSCRIBE_OK it sent the page, and the publisher answers: FETCH_OK with End Location {2, 1}, Object 0 of Groups 1 and 2 |
| {0, 1} to the end of Group 0, where the publisher has nothing | own relay | FETCH_OK with End Location {0, 0} and the publisher's End of Range indicator, `NonExistent` at {0, 2^64 - 1} |
| {100, 0} to {102, 1}: Objects the publisher has and never sent, so that no relay holds them | both relays | Own relay: FETCH_OK with End Location {102, 1} and the three Objects. **The moqtail relay asks the page as well**, and says that the publisher accepted with End Location {102, 1}; the publishing page served its FETCH whole. What the page that fetches gets from that relay is not looked at beyond its FETCH_OK: the relay sends it a fetch stream under the Request ID of its own FETCH, and `@xmoq/moqt` closes the session for that |
| The publisher accepts and writes nothing; the page gives its FETCH up after a second | own relay | The page has a FETCH_OK and its FETCH ends as cancelled; the relay cancels the FETCH it had sent the publisher, after the publisher's FETCH_OK; the publishing page sees the FETCH it served end as cancelled by the peer |
| The publisher refuses with `DOES_NOT_EXIST` | own relay | REQUEST_ERROR `DOES_NOT_EXIST`, passed on |
| The publisher sends one more Group, 83 Objects of 100,000 bytes; the page asks for it, reads nothing of the fetch stream and gives up after a second | own relay | The relay answers itself, FETCH_OK with End Location {3, 83}, and begins to write the 83 Objects. **Its writing ends before it is whole**: `FETCH_WRITE_ENDED … stopped pieces=K code=0x1` with K below 83 (62 in the runs here), because the page stops the fetch stream when it gives up. A write that was whole fails the run |
| The publisher sends one more Group on two subgroup streams, with another Publisher Priority and another property on each, Objects 0, 2, 4 on one and 1, 3, 5 on the other, and not in the order of their Object IDs. A page subscribes after three of them and sends a Joining FETCH; later it asks for the whole Group | own relay | **The relay answers both from its cache**, the Objects in the order of their Object IDs, each with its Subgroup ID, the Publisher Priority of its Subgroup, its property and its payload: 0 to 2 with End Location {3, 3}, then all six with {3, 6}. Between the two, the page's subscription brings Objects 3 to 5 and no other of the Group. No FETCH goes to the publisher |

**A subscriber that gives a FETCH up on the request stream alone** is the own Rust client with `--fetch-cancel-request-only`, in `fetch-rust.ts` (`page-large-request-only`): it asks for a Group of 8 Objects of 900,000 bytes the relay holds, reads nothing, and resets and stops the request stream after a second, leaving the fetch stream as it is. The relay is still writing and stops by its own order: `ABORT_FETCH_WRITE … code=0x1`, then `FETCH_WRITE_ENDED … aborted pieces=K code=0x1` with K below 8 (1 in the runs here). `stopped` in place of `aborted`, as in the run with a page above, would say that the subscriber stopped the fetch stream too.

**The own relay keeps the latest Group of a track and answers a FETCH that lies in it itself.** Where a run is to show what a publisher's answer becomes, it asks for a Group that is not the latest, and checks in the relay's log that the relay sent the FETCH on (`ANSWERED_BY` in `fetch.ts`); where the relay is to answer, it checks that no FETCH was sent.

The runs in which a client leaves go through the own relay only, because the driver waits for lines only that relay prints. Their names begin with `leaving:`. What each has to show, and why, is in the [record](../docs/experiments/2026-10-10-disconnects-through-own-relay.md).

| Runs with a client that leaves, or with two publishers of the track (35) | What is expected |
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
| An `own-rust` publisher announces the namespace and holds back its answer to the relay's SUBSCRIBE; another, which is connected and holds back its PUBLISH, sends it then (1) | Both publishers are there before the subscribers, and the driver tells each when to go on (`--serve-on-line`, `--publish-on-line` of the own Rust client: it prints `ASKED request_id=N` and `READY_TO_PUBLISH`, and waits for the lines `serve` and `publish` on its stdin). The subscribers get no SUBSCRIBE_OK before the relay has dealt with the PUBLISH. The PUBLISH answers them, before the first publisher has answered the relay. The relay waits 5 seconds for that publisher; a run in which it gave up before the PUBLISH, or before the publisher was told to answer, fails and says that it shows nothing. They get Group 0 from the one that sent PUBLISH and, when the other has answered, Group 1 from that one. The track ends when both have left |
| An `own-rust` publisher sends a Group, says PUBLISH_DONE with the Stream Count of a publisher that cannot count, publishes the track anew on the same session once the relay has printed that it read the PUBLISH_DONE (`DRAIN_TIMER`), under the next Track Alias or the same, and sends another Group (2) | The PUBLISH is accepted and the subscribers stay: they get no PUBLISH_DONE then, and both Groups within one subscription. The relay resets the first PUBLISH stream with CANCELLED, at once under the same alias and after its 2 seconds of waiting under another. When the publisher leaves, the subscribers see PUBLISH_DONE with Stream Count 2 |
| Which publishers the relay asks for a track (5): two `own-rust` publishers that only announce the namespace, or an `own-browser` page and an `own-rust` one; a publisher with PUBLISH and an `own-rust` one that announces before the subscribers come, or after; an `own-browser` page that announces and refuses what it is asked | The relay sends its own SUBSCRIBE to every publisher that announced and is no source of the track, in the order they announced, once each, also when a second subscriber comes; never to the publisher that sent PUBLISH. A publisher that announces while the track has subscribers is asked right then. Each one that accepts becomes a source, and the subscribers get its Group. The page of the second run accepts the relay's SUBSCRIBE (`serve` of `@xmoq/moqt`) and sends its Group on that subscription. The page that refuses (it does not call `serve`, so the SUBSCRIBE gets NOT_SUPPORTED) is not asked for the next subscriber, and is asked again when it announces anew |
| A Subgroup comes to the relay a second time, after its first stream was delivered (4): from a second publisher with another Publisher Priority (`own-rust` and `own-rust`, `own-browser` and `own-rust`), from the same `own-browser` publisher with another, and from a second publisher with the same priority written the other way (left out by `own-rust`, 128 by `own-browser`) | Another priority: the relay stops the second stream with MALFORMED_TRACK (`0x12`), every own subscriber sees PUBLISH_DONE `0x12` and has the Group once, from the stream that came first, and the relay cancels the PUBLISH of the publisher whose stream came second with `0x12` and that of the other with CANCELLED. The same priority: the second stream is stopped with CANCELLED like any second stream of a Subgroup, and the track goes on |

Each of these runs ends by moving one more Object over the same track with a fresh publisher and two fresh subscribers, and by having that publisher leave. One subscriber stays and has to see PUBLISH_DONE: a relay that still held a source of the run would take the fresh publisher as one more source and pass the Object, but it could not end the track when that publisher leaves.

| Option of `matrix.ts` | Values (default first) |
|---|---|
| `--relay` | `moqtail` and `own`. Give it once to run through one relay only |
| `--base-port` | `24400`. The first of the UDP ports used, one per pairing; the run with `mlmpub` takes the port 1000 above its own as well. Below 32768, where Linux begins to hand out the source ports of outgoing sockets |
| `--only` | Empty. Runs the pairings whose name contains the text, such as `--only own-rust`, `--only "own relay"`, `--only "leaving: "` or `--only "fetch: "` |
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
| `--port` | `24330`. UDP port of the relay |
| `--subscribe-track` | `object`. Any other name makes the relay answer REQUEST_ERROR, and the run fails by design |
| `--await-publish-done` | off. After the transfer, makes the publisher leave and waits until the subscriber reports that PUBLISH_DONE ended its subscription. The subscriber has to be an own client |
| `--announce-only` | off. The publisher, which has to be `own-browser`, announces the namespace and sends no PUBLISH: it accepts the SUBSCRIBE the relay sends it for the track, and sends on that subscription |
| `--object-properties` | off. The Object carries the two Object Properties, and the subscriber has to report the same. Both clients have to be own clients, the ones that send properties with content and report the ones they get |
| `--verbose` | off. Sets `RUST_LOG=debug` for the moqtail relay and makes `own-rust` print the bytes of every stream |

One side has to be an own client: a run between two moqtail peers checks nothing of this repository.

An own publisher leaves by sending PUBLISH_DONE. A moqtail publisher has no way to and ends its session, after which the relay sends PUBLISH_DONE to the subscriber on its own.

One run with a client that leaves:

```sh
bun disconnect.ts --scenario subscriber-leaves --publisher own-browser --leaver own-rust:kill --stayer own-browser
```

`disconnect.ts` starts the own relay and the clients like `run.ts`, plays the scenario, and prints a `RESULT` line with how each own subscription ended and what was measured, and a `SUMMARY` line that `matrix.ts` shows in its table. It exits 0 only if everything the scenario expects was seen.

What a scenario counts in the relay's log, or expects not to be there, it counts before a line of the relay that it names, and never "so far": the lines of the relay and of a client reach the driver in no order, and a count taken when a client had reported something missed the relay's line that was on its way, three times in CI. `RelayLog` in `harness.ts` has no other way to count, so a scenario that does not name such a line does not pass the type check.

| Option of `disconnect.ts` | Values |
|---|---|
| `--scenario` | `publisher-killed`, `last-object-then-close`, `subscriber-leaves`, `slow-subscriber`, `same-group-twice`, `second-source`, `publish-while-asked`, `publish-anew`, `ask-all`, `subgroup-priorities` |
| `--publisher` | A client, `own-rust` by default. `same-group-twice`, `second-source` and `publish-while-asked` take it twice, the first publisher and the second (in `publish-while-asked` the first is the one that is asked and has to be `own-rust`); the other scenarios take one and refuse more |
| `--subscriber` | A client; give it once per subscriber. Used by every scenario but `subscriber-leaves` |
| `--leaver` | `subscriber-leaves` only, once per subscriber that leaves: `<client>:cancel` (the subscription ends, the session stays), `<client>:close` (the client closes its session in order) or `<client>:kill` (the process is killed, or the page closed) |
| `--stayer` | `subscriber-leaves` only: a client, once per subscriber that stays |
| `--with-publish-done` | `last-object-then-close` only: the publisher sends PUBLISH_DONE before it closes. `own-rust` always does and needs the flag; the moqtail clients cannot |
| `--same-alias` | `publish-anew` only: the new PUBLISH has the Track Alias of the first. Without it, the next one |
| `--ask` | `ask-all` only: `two-announcers`, `announcer-and-publisher`, `late-announcer` or `refusing-announcer`. The announcers are `own-rust` (the page, where it refuses); `--publisher` is the one that sends PUBLISH, in the last three |
| `--priorities` | `subgroup-priorities` only: `<first>,<second>`, the Publisher Priority of the first and of the second stream of the Subgroup, each 0 to 255 or `default` for a header that leaves it out. With one `--publisher`, which has to be `own-browser`, it sends the Group twice itself |
| `--slow` | `slow-subscriber` only: `stopped-own-rust` (default), an own-rust client whose process is stopped, or `unread-own-browser`, a page that subscribes and never reads |
| `--first-sends` | `same-group-twice` and `second-source`: how many Groups the first publisher sends before the second sends; 1 by default. In `same-group-twice`, above 1 the two publishers send different payloads |
| `--second-resends` | `second-source` only: the second publisher starts at Group 0 and sends every Group of the first again before it sends the next one |
| `--first` | `second-source` only, what the first publisher does before the second sends: `done` (default; PUBLISH_DONE), `kill`, `kill-with-open-stream` (own-browser: it sends one more Group on a stream it leaves open, and its page is closed) or `stays` |
| `--second-joins` | `second-source` only: `before` (default) or `after` the subscribers have their first Object |
| `--port` | `24340`. UDP port of the relay |
| `--verbose` | off. Makes `own-rust` print the bytes of every stream |

Not every client can leave in every way. `own-rust` and `moqtail-rust` close their session only by having the Objects they were started for; `moqtail-browser` cancels by its `disconnect()`, which leaves its session open; only `own-browser` can cancel and keep going. A client that is asked for what it cannot do fails the run with a message that says so.

| File | Content |
|---|---|
| `harness.ts` | The certificate, the relay, the browser, the Rust client processes, and the teardown of all of them |
| `clients.ts` | The four clients behind one shape for a publisher and one for a subscriber |
| `run.ts` | The driver that moves one Object |
| `disconnect.ts` | The driver for the runs in which a client leaves |
| `fetch.ts` | The driver for the runs in which an own page sends a FETCH; each case of `--fetch` has its publisher, and the cases that begin with `page-` an own page that answers the FETCH. `--report-only` prints what the page saw and compares it with nothing |
| `fetch-rust.ts` | The driver for the runs in which the own Rust client sends a FETCH (`--mode fetch`, or `--mode subscribe` with `--join`). In `page-join-at-once` the run does not choose which of the two streams the relay reads first; where the relay held the FETCH (`HOLD_REQUEST … FETCH`), the run wants the FETCH_OK on that stream |
| `catalog.ts` | The driver for the three runs with an MSF catalog (`--catalog from-mlmpub`, `own`, `to-mlmsub`). What has to be seen is at the top of the file |
| `video.ts` | The driver for the runs in which an own page plays a video track: of `mlmpub` (`--video from-mlmpub`), and of another own page that encodes it (`--video page-to-page --through own\|moqtail --mapping stream-per-group\|stream-per-object --picture bars\|noise`), and through moqtail's relay when it sends GOAWAY (`--video goaway-from-moqtail\|goaway-from-moqtail-rust`). What has to be seen is at the top of the file |
| `video-rust.ts` | The driver for the run in which the own Rust client takes the video track of an own page through the own relay and writes it with `--annex-b-out`. What has to be seen is at the top of the file |
| `watch-rust.ts` | The driver for the runs of the terminal viewer, `xmoq-watch` (`crates/watch`), with no terminal and no sound output (`--out /dev/null`, `--sound off`): it finds the video and the audio track in a catalog and plays both for 8 seconds, of a page that publishes the catalog and both tracks on one connection (`publishBroadcast`) through the own relay, and of moqlivemock's publisher with no relay. What has to be seen is at its top. The viewer is the one binary of these runs that is a release build (`TERMINAL_VIEWER_BINARY` in `harness.ts` has why) |
| `quiet-peer.ts` | The driver for the thirty-four runs with `xmoq-quiet-peer` (`--case no-setup\|no-publish-answer\|no-fetch-answer\|no-stream\|no-subgroup-stream\|goaway\|goaway-publisher\|goaway-request\|request-id-parity\|request-id-again\|datagram --client own-browser\|own-rust\|xmoq-watch`, and `--case relay-no-setup\|relay-idle-session\|relay-unknown-datagram --client quiet-peer`). What has to be seen is at the top of the file |
| `namespaces.ts` | Namespace discovery (`--relay own\|moqtail`): an own-rust client sends SUBSCRIBE_NAMESPACE for (`xmoq`) and is told with NAMESPACE and NAMESPACE_DONE of (`xmoq`, `phase0`) as an own-rust publisher announces it and leaves. What has to be seen is at the top of the file |
| `chat.ts` | The gatherer of the chat, `xmoq-chat` (`crates/chat`), through a relay (`--relay own\|moqtail`): own-rust clients are two participants of a room and its viewers, live and by FETCH; a line too long, lines too fast, and a FETCH cut to its limit; with `--restart yes` (the default) also a restart of the gatherer on the same store with `--room r`, a participant of another room, and the end of the room. Through moqtail's relay it runs with `--restart no`, and is expected to fail at the FETCH the gatherer cuts to its limit (why both, at the pairing in `matrix.ts`). What has to be seen is at the top of the file |
| `chat-pages.ts` | Own pages as participants of the chat (`joinChat` of `@xmoq/chat`, which the demo's pages use), with the gatherer `xmoq-chat`, through a relay (`--relay own\|moqtail`): two pages say lines on their tracks `said` and have the room's `lines` live; the page keeps back a line too long and one too many in a second; a page that comes after 20 lines has the newest 16 by a Joining FETCH, marked `history`, and the next one live; a page that stops withdraws its namespace, and the gatherer lets it go (`PARTICIPANT_GONE`) and ends its subscription to that page's `said`. Through moqtail's relay the first SUBSCRIBE for the room is refused, since that relay does not hold it until the gatherer publishes, and the page asks again. What has to be seen is at the top of the file |
| `rendezvous.ts` | The driver for the eleven runs in which a SUBSCRIBE comes before its publisher (`--case publisher-comes\|nobody-comes\|not-asked\|publisher-returns\|another-publisher-comes\|page-publisher-returns\|viewer-before-the-broadcast --subscriber own-browser\|own-rust`). What has to be seen is at the top of the file |
| `new-group.ts` | The driver for the two runs in which a subscriber asks for a new Group (`--publisher own-rust\|own-browser`). What has to be seen is at the top of the file |
| `no-answer.ts` | The driver for the three runs in which a SUBSCRIBE is not answered in time (`--subscriber own-browser\|own-rust\|xmoq-watch`). What has to be seen is at the top of the file |
| `fetch-publishers.ts` | The two publishers both FETCH drivers start: an own page that answers a FETCH, and `moqtail`'s test client |
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
