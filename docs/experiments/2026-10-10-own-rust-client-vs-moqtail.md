# Own Rust client against the moqtail relay and browser (MOQT draft 18)

- Date: 2026-10-10
- Commit SHA: the commit that adds this record. The runs were made on its working copy on top of `20d3272`
- Roadmap item: Phase 0b, Rust wire codec and session with byte fixtures, and the own Rust client against the moqtail relay and browser client

## Objective

Check that a MoQ Transport implementation written from the text of `draft-ietf-moq-transport-18` interoperates with an independent one. `crates/moqt` (`xmoq-moqt`) holds the wire coding and session state and does no I/O; `crates/moqt-client` (`xmoq-moqt-client`) puts it on `wtransport`. The client subscribes and publishes through the moqtail relay, with moqtail-ts in a real Chromium on the other side, and the received payload has to equal the sent one.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Own code | `xmoq-moqt` 0.0.0 with no dependencies; `xmoq-moqt-client` 0.0.0 on `wtransport` 0.7.2, `tokio` 1.49.0, `anyhow` 1.0.100 (root `Cargo.lock`: `quinn` 0.11.12, `rustls` 0.23.45) |
| Relay | moqtail `relay` 0.15.0 (`moqtail/0.15.0+dd8623d`), built from tag `relay@0.15.0` |
| Browser | Chromium 156.0.8078.4, new headless, through `playwright` 1.64.0, running `moqtail` npm 0.13.0 |
| Other Rust clients | the Phase 0 spike client on the `moqtail` crate 0.15.0; moqtail's own test client `client` 0.15.0, for fixtures only |
| Toolchain | `cargo` 1.99.0 for the workspace, `node` 23.10.0 |
| Specification | `https://www.ietf.org/archive/id/draft-ietf-moq-transport-18.txt`, fetched 2026-10-10 |
| Wire | WebTransport subprotocol `moqt-18` on every session |

Provisional settings, none of them project decisions: no authentication; a self-signed certificate valid for 13 days, pinned by hash; namespace `xmoq/phase0`, track `object`; in the client, a 1 MiB cap on the undecoded bytes of one subgroup stream, at most 64 unidirectional streams read at once, 16 decoded Objects queued, at most 64 Key-Value-Pairs in one list (Setup Options, Track Properties, the Properties of an Object) and 64 Message Parameters in one message, and a wait of 2 seconds for the last Objects after PUBLISH_DONE.

## Commands

From the repository root. The relay binary, the spike client and the browser dependencies come from the steps in [`spikes/moqtail/README.md`](../../spikes/moqtail/README.md); moqtail's test client, used only to capture fixtures, comes from [`fixtures/moqt-18/README.md`](../../fixtures/moqt-18/README.md).

```sh
mise run check
cargo build --workspace --locked

node spikes/moqtail/browser/run.mjs --direction browser-to-rust --rust-subscriber own                       # run 1
node spikes/moqtail/browser/run.mjs --direction rust-to-browser --rust-publisher own                        # run 2
node spikes/moqtail/browser/run.mjs --direction rust-to-rust --rust-publisher moqtail --rust-subscriber own # run 3
node spikes/moqtail/browser/run.mjs --direction rust-to-rust --rust-publisher own --rust-subscriber moqtail # run 4
node spikes/moqtail/browser/run.mjs --direction rust-to-rust --rust-publisher own --rust-subscriber own     # run 5
node spikes/moqtail/browser/run.mjs --direction browser-to-rust                                             # run 6
node spikes/moqtail/browser/run.mjs --direction rust-to-rust --rust-publisher own --rust-subscriber own --await-publish-done # run 7
```

A different `--port` (44351–44357) was passed per run. After the last code change (a limit on Message Parameters, and Properties checked before the payload arrives) the check and the seven runs were repeated on ports 44361–44367 with the same outcomes; the excerpts below are from the round before. The commands that captured the fixtures are in the fixtures README.

## Observed results

**Passed in both directions the roadmap item names**, browser to Rust and Rust to browser.

`mise run check` ended with `OK: Rust checks (fmt, clippy, test)`, `OK: Layer checks` (`xmoq-moqt` depends on no I/O crate) and `OK: Workflow and script checks (actionlint, shellcheck)`; the TypeScript checks were skipped because this working copy has no `packages/`. The tests reported `78 passed; 0 failed` for `xmoq-moqt` and `3 passed; 0 failed` for `xmoq-moqt-client`. Clippy ran under the workspace lints (`pedantic`, `as_conversions`, `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`, `missing_docs` and others), and the code has no `allow` attribute.

| # | Publisher | Subscriber | Outcome |
|---|---|---|---|
| 1 | moqtail-ts in Chromium | own Rust client | payload match |
| 2 | own Rust client | moqtail-ts in Chromium | payload match |
| 3 | moqtail crate (spike client) | own Rust client | payload match |
| 4 | own Rust client | moqtail crate (spike client) | payload match |
| 5 | own Rust client | own Rust client | payload match |
| 6 | moqtail-ts in Chromium | moqtail crate (spike client) | payload match; checks that the driver changes did not break the earlier setup |
| 7 | own Rust client | own Rust client, asking for 2 Objects | payload match; then PUBLISH_DONE received and the subscription ended |

Every session went through the moqtail relay. The excerpts below shorten long lines with `...` and leave out the relay's lines, except the ones that show a PUBLISH_DONE arriving at or leaving the relay. The driver merges the output of several processes, so lines of different processes are not always in the order things happened; the run 7 excerpt puts the relay's lines where they belong.

Run 1:

```text
[browser-pub] connected: protocol=moqt-18
[browser-pub] track published: alias=868305376117715346
[rust-sub] CONNECTED url=https://127.0.0.1:44351
[rust-sub] SETUP_OK server_setup=Setup { setup_options: [KeyValuePair { ty: 7, value: Bytes([109, 111, 113, 116, 97, 105, 108, 47, ...]) }] }
[rust-sub] SUBSCRIBED track_alias=0
[browser-pub] object enqueued: group=0 object=0 payload_len=53
[rust-sub] OBJECT group=0 object=0 status=Normal payload_len=53 payload_hex=786d6f712d70686173653020323032362d31302d31305430323a34343a35392e3538335a2065396535333938323132363135356161
[rust-sub] STREAM_STOPPED uni#1 the subscription ended
[driver] RESULT {"direction":"browser-to-rust","match":true,"sentText":"xmoq-phase0 2026-10-10T02:44:59.583Z e9e53982126155aa", ... ,"rustSubscriber":"own","browser":"chromium","browserVersion":"156.0.8078.4", ...}
```

Run 2:

```text
[rust-pub] NAMESPACE_PUBLISHED
[rust-pub] PUBLISHED track_alias=1 forward=true
[browser-sub] connected: protocol=moqt-18
[browser-sub] subscribed: requestId=0
[rust-pub] SENT group=0 object=0 payload_len=53
[relay] ... control message received from conn 5100365840 — PublishDone (6 bytes): 0b 00 03 02 01 00
[relay] ... Received PublishDone message for request ID: 2 with status: TrackEnded
[driver] RESULT {"direction":"rust-to-browser","match":true,"sentText":"xmoq-phase0 2026-10-10T02:45:03.783Z b003f0357cf43c04", ... ,"rustPublisher":"own","browser":"chromium","browserVersion":"156.0.8078.4", ...}
```

Runs 3 to 6, the `RESULT` line of each:

```text
[driver] RESULT {"direction":"rust-to-rust","match":true,"sentText":"xmoq-phase0 2026-10-10T02:45:07.530Z fde890c7bb728d46", ... ,"rustPublisher":"moqtail","rustSubscriber":"own", ...}
[driver] RESULT {"direction":"rust-to-rust","match":true,"sentText":"xmoq-phase0 2026-10-10T02:45:10.854Z 0d2dd34462dfe706", ... ,"rustPublisher":"own","rustSubscriber":"moqtail", ...}
[driver] RESULT {"direction":"rust-to-rust","match":true,"sentText":"xmoq-phase0 2026-10-10T02:45:14.149Z 02ef10110e0a0d01", ... ,"rustPublisher":"own","rustSubscriber":"own", ...}
[driver] RESULT {"direction":"browser-to-rust","match":true,"sentText":"xmoq-phase0 2026-10-10T02:45:17.464Z c7b5e25611233391", ... ,"rustSubscriber":"moqtail","browser":"chromium","browserVersion":"156.0.8078.4", ...}
```

Run 7. The subscriber asked for two Objects, so it was still subscribed when the driver ended the publisher:

```text
[rust-pub] SENT group=0 object=0 payload_len=53
[rust-sub] OBJECT group=0 object=0 status=Normal payload_len=53 payload_hex=786d6f71...
[relay] ... control message received from conn 5083551248 — PublishDone (6 bytes): 0b 00 03 02 01 00
[relay] ... Sent PublishDone to subscriber=5092014608 relay_track_id=0 for request_id=0 stream_count=1
[relay] ... control message sending to conn 5092014608 — PublishDone (28 bytes): 0b 00 19 02 01 16 50 75 62 6c 69 73 68 65 72 20 64 69 73 63 6f 6e 6e 65 63 74 65 64
[rust-sub] PUBLISH_DONE status_code=0x2 stream_count=1 reason="Publisher disconnected"
[rust-sub] SUBSCRIPTION_ENDED why=PUBLISH_DONE, all streams closed received=1 wanted=2
[driver] RESULT {"direction":"rust-to-rust","match":true, ... ,"subscriptionEnd":"why=PUBLISH_DONE, all streams closed received=1 wanted=2"}
```

In run 1 the subscriber left after its one Object while moqtail-ts had not closed the subgroup stream, so the client sent STOP_SENDING on it (§10.11); that is the `STREAM_STOPPED` line.

In run 7 the subscriber read the PUBLISH_DONE on its request stream. The one data stream the message counted had already ended, so the Track Alias was retired by the stream count and the 2-second wait did not run. The relay did not forward the publisher's message: it sent its own, with the reason `Publisher disconnected`.

### Byte-level agreement

`fixtures/moqt-18/` holds 17 captures from moqtail: 13 control messages and 4 subgroup streams. For each of them the tests assert both that `xmoq-moqt` decodes the bytes into the expected value and that it encodes that value back into the same bytes. All 17 agree in both directions. The captures are bytes from the wire, not a re-encoding by the relay; the fixtures README says how that was checked. It also lists the encodings no fixture covers.

### Differences between the draft text and moqtail

- **FIRST_OBJECT on a stream that does not start with the subgroup's first Object.** A subscriber that joined inside a Group received a subgroup stream whose header type was `0x5D`, which has FIRST_OBJECT set, and whose first Object had Object ID 4 while the publisher had sent IDs 0 and 2 before it (`subgroup-stream-joined-mid-group-from-moqtail-rs`). §11.4.2 defines the bit as "the first object in this subgroup stream is the first object published in the subgroup by the original publisher". Why the relay sets it was not looked up in its source. `xmoq-moqt` decodes the bit and does not act on it.
- moqtail's clients send SETUP without MOQT_IMPLEMENTATION, which §10.3.1.5 says an endpoint SHOULD send. The relay sends it. The own client sends `xmoq-moqt-client/0.0.0`.
- moqtail-ts leaves its subgroup stream open after the one Object, with END_OF_GROUP set in the header. §11.4.3 requires a FIN once every Object of the Subgroup is on the stream; it sets no deadline, and a receiver cannot tell whether the publisher meant to add Objects, so this was not counted as a violation.
- The relay re-serializes subgroup streams and gives each subscriber a Track Alias of its own. A Track Alias is chosen by the publisher of each session (§10.8, §11.1), so the alias has to change at a relay. No rule about re-serializing a stream was looked for.

### Places where the draft itself is unclear

- **PUBLISH_OK.** Table 5 gives PUBLISH_OK the type `0x1E`, while §10.5 says a PUBLISH is answered with REQUEST_OK (`0x7`) and uses "PUBLISH_OK" as a name for that message. `xmoq-moqt` follows §10.5, as moqtail does, and reports `0x1E` as unsupported.
- **FILL_TIMEOUT and RENDEZVOUS_TIMEOUT** (§10.2.5, §10.2.6) do not state their value encoding. `xmoq-moqt` reads RENDEZVOUS_TIMEOUT as a varint, which is also what the `moqtail` crate writes, and does not decode FILL_TIMEOUT, which belongs to FETCH.
- **Second example of §11.6.** Its lengths do not add up under the draft's own encoding (it gives `Properties Length = 33` and `Length = 21` for the 14-byte value `traceID:123456`). It is not used as a test vector; the first example is.
- **Unknown message and stream types** (§10, §3.4) close the session without naming a code. `xmoq-moqt` maps them to PROTOCOL_VIOLATION.
- **An Object after End of Group or End of Track on the same stream.** §2.4.2 lists it among the conditions that make a track malformed; §11.4.3 calls it a protocol violation in passing. `xmoq-moqt` follows §2.4.2: the subscription is cancelled and the session stays open.
- **A stream that ends inside a control message, a stream type or a Subgroup Header** has no rule. §11.4 covers only the middle of a serialized Object. `xmoq-moqt` treats all of them as PROTOCOL_VIOLATION.

## Limitations and blockers

No blocker for this step.

Left out on purpose:

- Everything a relay needs to answer requests: accepting a peer's SUBSCRIBE, PUBLISH or PUBLISH_NAMESPACE, and checking the peer's Request IDs (§10.1, INVALID_REQUEST_ID). The check that a bidirectional stream begins with a request message (§3.3) belongs to the same side. The client never accepts a bidirectional stream. `FIXME:` comments mark both places.
- FETCH, GOAWAY, TRACK_STATUS, NAMESPACE, SUBSCRIBE_NAMESPACE, SUBSCRIBE_TRACKS and PUBLISH_BLOCKED. Their type codes are recognised and reported as unsupported, which closes the session with INTERNAL_ERROR. For GOAWAY that means a legitimate message from a relay ends the session; marked with a `FIXME:`.
- Datagrams. Nothing reads them: an OBJECT_DATAGRAM is lost, and an unknown datagram type does not close the session, which §11 requires. Marked with a `FIXME:`.
- Native QUIC. The client speaks WebTransport only. Answering a peer's FETCH, TRACK_STATUS, SUBSCRIBE_NAMESPACE or SUBSCRIBE_TRACKS with REQUEST_ERROR NOT_SUPPORTED, as XmoQ decided, needs the responder side and is marked with a `FIXME:`.
- The content of AUTHORIZATION_TOKEN (§10.2.2): the Token structure is carried as bytes, with no alias cache, so none of the MUSTs of that section is enforced. Marked with a `FIXME:`.
- Malformed-track detection across streams (§2.4.2). Within one stream these are caught: a Mandatory Track Property on an Object, an Object after End of Group or End of Track, and a Prior Group ID Gap or Prior Object ID Gap that appears twice on an Object or is larger than the ID it counts back from (§12.8, §12.9).
- A Stream Count that the peer exceeds (§10.11 lets a subscriber close the session; `xmoq-moqt` does not).
- Applying a REQUEST_UPDATE. Every update is refused.
- Timeouts, priorities and delivery order. The wait after PUBLISH_DONE (§10.11) is left to the caller, which tells the session when it gives up.
- A REDIRECT in answer to PUBLISH: §10.6.2 does not list PUBLISH among the requests it can answer, and gives no rule for receiving one. It is passed to the caller like any other rejection.

Handled, not left out: padding streams (§11.5.1), the two MUSTs for a Redirect (§10.6.1), a unidirectional stream that is reset before its type arrives (§11.4.1; it changes nothing for the session) and a peer that closes its control stream (§3.3, PROTOCOL_VIOLATION).

Limits of what was run:

- One Object per run, on one subgroup stream. Several Objects and Groups were received once, while capturing fixtures. Late join was seen only there; reconnect, cancellation and slow subscribers were not tried.
- The decoder holds a whole Object in memory before returning it, so the buffer cap is also the Object size limit. An Object over the cap stops its stream and leaves the session open; that path was not triggered.
- After PUBLISH_DONE the Track Alias was retired by the stream count (run 7). The other way out, the caller's timeout, ran only in unit tests, and so did the case where the peer resets the subscribe stream.
- Nothing provoked a malformed track, a reset or padding stream, or a closed control stream against the relay. The paths for them are covered by unit tests of the decoder and of the classification of errors in the library (`Error::scope`), not by a run.
- The publishing side reads its PUBLISH stream and would answer a REQUEST_UPDATE with REQUEST_ERROR NOT_SUPPORTED followed by PUBLISH_DONE UPDATE_FAILED (§10.9, §10.9.1). The relay sent none, so this ran only in unit tests of the session. The same holds for messages that cross a PUBLISH_DONE this endpoint sent (§5.1), which are ignored.
- The own client was not the target of a request from the relay. If the relay had opened a request stream towards it, the client would not have answered.
- REQUEST_ERROR was received once (DOES_NOT_EXIST, the source of one fixture). A rejected PUBLISH or PUBLISH_NAMESPACE and a Redirect were not provoked.
- Closing the session with a termination code was not triggered against the relay. Whether `wtransport` 0.7.2 carries the code in CLOSE_WEBTRANSPORT_SESSION was not checked.
- Chromium only, headless, one machine, loopback.

## Open questions raised

1. **How would a relay take Objects that do not fit in memory?** The current decoder returns whole Objects. A relay forwarding video needs either a limit XmoQ commits to or an interface that passes payload bytes through as they arrive.
2. **What is the Object size limit**, and what happens to a subscription that exceeds it? Stopping only the stream is a provisional choice.
3. **Which messages would the first own relay need** beyond today's set? Serving moqtail-ts as it behaves now needs the responder side of SUBSCRIBE, PUBLISH and PUBLISH_NAMESPACE, and PUBLISH_DONE towards subscribers. Whether GOAWAY, REQUEST_UPDATE, TRACK_STATUS and the namespace subscriptions are in the first cut is undecided.
4. **Would the own relay cache**, and for how long? The moqtail relay does; the draft allows a relay without a cache.
5. **How are authorization tokens handled?** Authentication is undecided for the project, and the relay is where tokens would be checked.
6. **Which timeouts would the relay enforce**, and with what values? The draft leaves all of them to the implementation.
7. **Is native QUIC in scope**, or WebTransport only? AUTHORITY and PATH handling exists in the session code but was only unit-tested.
8. **Does an own relay set FIRST_OBJECT correctly for a subscriber that joins inside a subgroup**, where moqtail does not, and does anything in XmoQ depend on the bit?
