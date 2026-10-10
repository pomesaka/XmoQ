# Own Rust relay against moqtail peers and the own Rust client (MOQT draft 18)

- Date: 2026-10-10
- Commit SHA: the commit that adds the relay together with this record. The runs were made on its working copy on top of `0f09de4`
- Roadmap item: Phase 0b, own Rust relay against the moqtail browser client and the own Rust client

## Objective

Replace the moqtail relay with XmoQ's own and leave the clients as they are: moqtail-ts in a real Chromium, the Rust client on the `moqtail` crate, and XmoQ's own Rust client. One Object has to pass through the relay with its payload unchanged, to one subscriber and to two. Beyond the plain transfer, three situations are provoked on purpose: the publisher goes away first, a subscriber goes away first, and a subscriber asks for a track nobody published.

Before the relay was written, one question was answered separately: what a browser sees when a `wtransport` server closes a session or aborts a stream with an error code.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Own relay | `xmoq-relay` 0.0.0 (`crates/relay`), on `xmoq-relay-core`, `xmoq-moqt-io` and `xmoq-moqt` 0.0.0; `wtransport` 0.7.2, `tokio` 1.49.0 (root `Cargo.lock`: `quinn` 0.11.12) |
| Own client | `xmoq-moqt-client` 0.0.0, on the same `xmoq-moqt-io` |
| moqtail peers | `moqtail` npm 0.13.0 in the page; the spike client on the `moqtail` crate 0.15.0; for the regression runs, the moqtail `relay` 0.15.0 (`moqtail/0.15.0+dd8623d`) |
| Browser | Chromium 156.0.8078.4, new headless, through `playwright` 1.64.0 |
| Toolchain | `cargo` 1.99.0, `node` 23.10.0 |
| Specifications | `draft-ietf-moq-transport-18`; `draft-ietf-webtrans-http3-16` §4.4 for the stream error codes, fetched 2026-10-10 |

Provisional settings, none of them project decisions:

- No authentication or authorization: anyone who reaches the port can publish and subscribe.
- A self-signed certificate valid for 13 days, pinned by hash.
- The relay keeps no Objects. A subscriber gets what arrives after it subscribed.
- Limits and deadlines in the relay and the shared stream code. The values after the review are in the table of limits in [the implementation notes](../moqt-implementation-notes.md#relay). The runs of the first pass, below, were made with a send queue per subgroup stream (4 MiB or 4096 Objects) and 64 request streams read at once.

## Commands

From the repository root. The moqtail relay, the spike client, the certificate and the browser dependencies come from the steps in [`spikes/moqtail/README.md`](../../spikes/moqtail/README.md).

```sh
mise run check
cargo build --workspace --locked

# What a browser sees of error codes from a wtransport server.
cargo build --manifest-path spikes/wt-error-codes/Cargo.toml
node spikes/wt-error-codes/probe.mjs          # codes passed as they are
node spikes/wt-error-codes/probe.mjs --map    # stream codes remapped by hand

# Own relay. P = publisher, S = subscriber.
R="node spikes/moqtail/browser/run.mjs --relay own"
$R --direction browser-to-rust --rust-subscriber own                                         #  1  P moqtail-ts, S own
$R --direction rust-to-browser --rust-publisher own                                          #  2  P own, S moqtail-ts
$R --direction browser-to-browser                                                            #  3  P and S moqtail-ts
$R --direction rust-to-rust --rust-publisher moqtail --rust-subscriber own                   #  4
$R --direction rust-to-rust --rust-publisher own --rust-subscriber moqtail                   #  5
$R --direction rust-to-rust --rust-publisher moqtail --rust-subscriber moqtail               #  6
$R --direction rust-to-rust --rust-publisher own --rust-subscriber own                       #  7
$R --direction browser-to-rust --rust-subscriber moqtail                                     #  8
$R --direction rust-to-browser --rust-publisher moqtail                                      #  9
$R --direction browser-to-browser --subscribers 2                                            # 10
$R --direction browser-to-rust --rust-subscriber own --subscribers 2                         # 11
$R --direction rust-to-rust --rust-publisher own --rust-subscriber own --subscribers 2 --await-publish-done  # 12

node spikes/moqtail/browser/relay-scenarios.mjs
```

A different `--port` was passed per run. Runs 13 to 19 repeat the seven runs of the [own Rust client record](2026-10-10-own-rust-client-vs-moqtail.md) against the moqtail relay, without `--relay own`, because the client was moved onto the shared stream code.

## Observed results

**Passed: every combination moved the Object through the own relay with its payload unchanged, and the three provoked situations ended as intended.**

`mise run check` ended with `OK` for the Rust checks (fmt, clippy, test), the layer checks (`OK: I/O-free check for xmoq-moqt`, `OK: I/O-free check for xmoq-relay-core`), the TypeScript checks and the workflow checks. Tests: 97 in `xmoq-moqt`, 18 in `xmoq-relay-core`, 23 in `xmoq-moqt-io`, 4 in `xmoq-relay`, 3 in `xmoq-moqt-client`; none failed. The code has one `expect(clippy::too_many_arguments)` attribute, with its reason, in `crates/relay-core/src/lib.rs`, and no `allow`.

### Transfers through the own relay

| # | Publisher | Subscriber(s) | Outcome |
|---|---|---|---|
| 1 | moqtail-ts in Chromium | own Rust client | payload match |
| 2 | own Rust client | moqtail-ts in Chromium | payload match |
| 3 | moqtail-ts in Chromium | moqtail-ts in Chromium | payload match |
| 4 | moqtail crate (spike client) | own Rust client | payload match |
| 5 | own Rust client | moqtail crate (spike client) | payload match |
| 6 | moqtail crate (spike client) | moqtail crate (spike client) | payload match |
| 7 | own Rust client | own Rust client | payload match |
| 8 | moqtail-ts in Chromium | moqtail crate (spike client) | payload match |
| 9 | moqtail crate (spike client) | moqtail-ts in Chromium | payload match |
| 10 | moqtail-ts in Chromium | 2 × moqtail-ts in Chromium | both match |
| 11 | moqtail-ts in Chromium | 2 × own Rust client | both match |
| 12 | own Rust client | 2 × own Rust client | both match; both then receive PUBLISH_DONE |

Runs 13 to 19, against the moqtail relay, all matched as before.

Run 10, the relay's lines. Session 0 is the publisher; each subscriber gets a stream of its own under its own Track Alias 0:

```text
[relay] LISTENING xmoq-relay/0.0.0 port=44510
[relay] SESSION_OPENED session=0
[relay] SEND session=0 control SETUP
[relay] SEND session=0 stream=1 REQUEST_OK
[relay] SEND session=0 stream=2 REQUEST_OK
[relay] SESSION_OPENED session=1
[relay] SEND session=1 control SETUP
[relay] SEND session=1 stream=1 SUBSCRIBE_OK track_alias=0
[relay] SESSION_OPENED session=2
[relay] SEND session=2 control SETUP
[relay] SEND session=2 stream=1 SUBSCRIBE_OK track_alias=0
[relay] OPEN_DATA session=1 out=0 track_alias=0 group=0
[relay] OPEN_DATA session=2 out=1 track_alias=0 group=0
[relay] FORWARD out=0 object=0 payload_len=53
[relay] FORWARD out=1 object=0 payload_len=53
[driver] RESULT {"direction":"browser-to-browser","match":true, ... ,"relay":"own","subscribers":2, ...}
```

The two `REQUEST_OK` answer moqtail-ts's PUBLISH_NAMESPACE and PUBLISH. moqtail-ts sent nothing the relay did not implement: no run produced a `REQUEST_ERROR`, a refused request or a closed session.

### Provoked situations

`relay-scenarios.mjs` uses the own client on both sides.

```text
[driver] SCENARIO publisher disconnects first
[relay] SEND session=1 stream=1 PUBLISH_DONE status_code=0x2 stream_count=1
[sub] PUBLISH_DONE status_code=0x2 stream_count=1 reason=""
[sub] SUBSCRIPTION_ENDED why=PUBLISH_DONE, all streams closed received=1 wanted=2
[driver] RESULT {"scenario":"publisher disconnects first", ... ,"subscriberExit":0}
[driver] SCENARIO subscriber disconnects first
[sub-stays] OBJECT group=1 object=0 status=Normal payload_len=19 payload_hex=786d6f712d72656c61792d7363656e6172696f
[driver] RESULT {"scenario":"subscriber disconnects first","leaverExit":0,"stayerExit":0,"publisherExit":0}
[driver] SCENARIO subscriber is killed
[sub-stays] OBJECT group=1 object=0 status=Normal payload_len=19 payload_hex=786d6f712d72656c61792d7363656e6172696f
[driver] RESULT {"scenario":"subscriber is killed","stayerExit":0,"publisherExit":0}
[driver] SCENARIO subscribe to a track nobody published
[relay] SEND session=8 stream=1 REQUEST_ERROR error_code=0x10
[sub] Error: SUBSCRIBE rejected: error_code=0x10 retry_interval=0 reason=""
[driver] RESULT {"scenario":"subscribe to a track nobody published", ... ,"subscriberExit":1}
```

- **Publisher first.** The publisher is killed with SIGKILL, so nothing tells the relay. It notices when the connection has been idle for 9 seconds, ends the track and sends the subscriber PUBLISH_DONE with TRACK_ENDED (`0x2`). With the default idle timeout of `quinn` (30 seconds) the same run timed out in the driver after 20 seconds; that is why the timeout is set.
- **Subscriber first.** One of two subscribers leaves after the first Object, once by closing its session and once by SIGKILL. The publisher sends a second Object, and the remaining subscriber receives it (`group=1`). The publisher ends normally.
- **Unknown track.** The SUBSCRIBE is answered with REQUEST_ERROR DOES_NOT_EXIST (`0x10`) and the session goes on until the client leaves.

In run 12 the publisher ends in order instead, with PUBLISH_DONE, and both subscribers are told.

### What a browser sees of error codes from `wtransport` 0.7.2

Measured with `spikes/wt-error-codes` before the relay was written. The server closed a session with code `0x3` and a reason, reset a stream with `0x12`, and sent STOP_SENDING with `0x5`; the page read `WebTransport.closed` and the `streamErrorCode` of the stream errors.

| Server does | Codes passed as they are | Stream codes remapped by hand |
|---|---|---|
| `close(0x3, "protocol violation")` | `closed` rejects with `WebTransportError` "Connection lost."; no `closeCode`, no `reason` | the same: the remapping does not apply to sessions |
| resets a stream with `0x12` | `streamErrorCode: 0` | `streamErrorCode: 18` |
| STOP_SENDING with `0x5` | `NetworkError`, no code | `streamErrorCode: 5` in 3 of 5 runs, the `NetworkError` without a code in the other 2 |
| (page aborts a stream with `0x12`) | the server reads `Reset(91141958510829)`, which is `0x52e4a40fa8db + 0x12` | the same; mapped back to `0x12` |
| (page closes with `closeCode: 3`, `reason: "bye"`) | `closed()` says `LocallyClosed`; the next call on the session returns `ApplicationClosed { code: 3, reason: "bye" }` | the same |

Consequences taken in the code, all in `crates/moqt-io/src/wt.rs` and `codes.rs`:

- Stream error codes are remapped on the way out and back on the way in, as `draft-ietf-webtrans-http3-16` §4.4 requires of a WebTransport implementation. `wtransport` 0.7.2 does not do it, and neither does the moqtail relay (`libs/moqtail-rs/src/transport/connection.rs` at tag `relay@0.15.0`, read 2026-10-10).
- A received code outside the reserved range, or on one of its reserved codepoints, is reported as a reset without a code. §4.4 says such a code SHOULD be delivered "as a stream reset with no application error code". `draft-ietf-moq-transport-18` §14 has the receiver treat an unknown code like INTERNAL_ERROR.
- **A browser does not get the Session Termination Error Code or the reason from this relay.** `wtransport` 0.7.2 cannot send the `WT_CLOSE_SESSION` capsule; it only closes the QUIC connection. A peer that reads the QUIC close gets the code: the in-memory tests assert it, and another `wtransport` endpoint would.
- The STOP_SENDING code did not reach the page in 2 of 5 runs. The cause was not found. The [library comparison](2026-10-10-webtransport-library-comparison.md) measured the same effect with another library, at a lower rate.

Whether XmoQ keeps `wtransport` is with the project; see the library comparison. The transport library is used in one file, `wt.rs`.

### How the code is split

| Crate | Does | I/O |
|---|---|---|
| `xmoq-moqt` | Wire coding and session state. New: the responder side (requests the peer opens, its Request IDs), and a handle per session that another session refuses | none (checked by `check:layers`) |
| `xmoq-relay-core` | Which Object goes to which subscriber, who is cut off, what a leaving peer takes with it. One `handle(Event) -> Vec<Action>` for all sessions | none (checked by `check:layers`) |
| `xmoq-moqt-io` | Reads and writes MoQ streams on a `Connection` trait: limits, deadlines, stops, the per-stream send queue. Two connections: `wtransport`, and an in-memory pair for tests | `tokio`, `wtransport` |
| `xmoq-relay` | Accepts sessions and connects the two crates above | through `xmoq-moqt-io` |
| `xmoq-moqt-client` | The test client, now on `xmoq-moqt-io` | through `xmoq-moqt-io` |

The stream handling that was untested in the client is now tested on the in-memory connection: a full queue holds the sender back, at most the configured number of streams is read at once, a stream is stopped on request and its slot freed, what was finished before a disconnect is still delivered, a stream that never says what it is is stopped after the deadline. The relay as a whole runs on the same connection in four tests, among them a subscriber that stops reading and is cut off with TOO_FAR_BEHIND while another one receives everything.

### Differences between the draft text and moqtail

- The moqtail relay passes stream error codes to `wtransport` without remapping them (above).
- The moqtail relay answers PUBLISH with FORWARD and GROUP_ORDER; the own relay sends FORWARD only. Both are within §10.2.
- The moqtail relay sends PUBLISH_DONE with the reason "Publisher disconnected" when a publisher ends in order; the own relay sends an empty reason. The draft does not say what a relay puts there.

### Judgments the draft leaves open

These were made in the code in the first pass. After the review they are in [the implementation notes](../moqt-implementation-notes.md#relay), which is where they are kept up to date; two of them changed (below).

- A second PUBLISH for a track that already has a publisher is rejected with UNINTERESTED (`0x20`), the code §10.6.2 gives a subscriber for declining a PUBLISH. §9.3 lets a relay refuse a publisher it cannot take and names no code.
- The relay answers PUBLISH with Forward State 1 from the start (§9.5 allows 0 until a subscriber comes).
- Whatever ends the upstream subscription, the subscribers get TRACK_ENDED, except for a malformed track (MALFORMED_TRACK) and a refused REQUEST_UPDATE (UPDATE_FAILED).
- A REQUEST_UPDATE for a request that is not answered yet ends the request: its REQUEST_ERROR is the only response the stream gets, since responses carry no Request ID.
- A SUBSCRIBE or PUBLISH that would double a subscription in the same role is answered with DUPLICATE_SUBSCRIPTION (§5.1); a PUBLISH with a Mandatory Track Property with UNSUPPORTED_EXTENSION (§2.5.1); a request beyond 256 open ones with EXCESSIVE_LOAD.
- More data streams than PUBLISH_DONE counted closed the session with PROTOCOL_VIOLATION. Changed after the review: the count is not checked, as in the TypeScript implementation.
- The send queue of a subgroup stream is bounded by payload bytes as well as by a count. A count alone took a burst of small Objects for a slow subscriber: one read of a publisher's stream can decode hundreds at once. Changed after the review: the bound is per subscriber session, over all its streams.

### After the first review

The review found ways to make the relay hold memory or stall, and rules of the draft it did not meet. What changed, and what was run again on 2026-10-10:

- A subscriber is cut off by what waits for its session as a whole: bytes, Objects and streams that are not written out, finished ones included. Before, the bound was per stream, and a subscriber that read nothing of many short Subgroups was never cut off.
- Undecoded bytes are bounded per session (4 MiB) as well as per stream. Streams held for an unknown Track Alias are bounded in bytes, released after 2 seconds, and released when their PUBLISH is refused.
- A request stream has its own writer, which notices STOP_SENDING (§3.3.2) and never makes the session's other writes wait. A control stream that cannot be written, or that the peer stopped, closes the session.
- The process ends when the relay's one deciding task ends. Logging goes through a bounded queue and no longer prints a line per forwarded Object. At most 64 handshakes run at once.
- A subscription the relay ended with PUBLISH_DONE is forgotten when its stream closes (it used to count against the 256 open requests for good). LARGEST_OBJECT of a PUBLISH is kept. FORWARD 0 in a SUBSCRIBE is honoured. Stream Count is the number of streams opened, or 2^62 - 1.
- The traits of `xmoq-moqt-io` state what a connection has to do, `finish` no longer waits for the peer, and `wtransport` is behind a cargo feature of that crate.

```sh
mise run check
mise run interop
node spikes/moqtail/browser/relay-scenarios.mjs
```

`mise run check` ended with `OK` for all four parts. Tests: 99 in `xmoq-moqt`, 29 in `xmoq-relay-core`, 28 in `xmoq-moqt-io`, 8 in `xmoq-relay`, 3 in `xmoq-moqt-client`; none failed. The `expect(clippy::too_many_arguments)` is gone; the Rust code of the relay has no `allow` or `expect` attribute.

`mise run interop` now runs every pairing through the moqtail relay and through the own relay ([`interop/`](../../interop/README.md)): `32 of 32 pairings ended as expected`. Through the own relay that is the 12 transfers, the two runs with three subscribers and the two refused subscriptions; `own-browser`, the TypeScript client, met the own relay here for the first time, as publisher and as subscriber, and passed without a change on either side. `relay-scenarios.mjs` printed the same four `RESULT` lines as above.

Tested on the in-memory connection only, not against a real peer: the cut-off of a subscriber that reads none of 30 short Groups while another one gets all of them, STOP_SENDING on a SUBSCRIBE stream and on the control stream, the session limit, and the limits on early streams.

### After the second review

The second review asked whether the in-memory connection, on which the relay is tested, behaves like a real one. `crates/moqt-io/src/contract.rs` now holds the contract of the connection traits as 14 cases that run against both the in-memory pair and a `wtransport` 0.7.2 client and server over 127.0.0.1.

**The first run failed for the in-memory connection, and the real one was right.** The contract said that nothing is read once a connection is gone. With `wtransport` on quinn 0.11.12, bytes and a FIN that had arrived before the close are still read, and a stream that had arrived is still accepted; only then do the calls fail. The contract and the in-memory connection were changed to that. A sender learns that its bytes arrived from the new `finish_and_wait`, which completes when the peer acknowledged everything; `finish` alone followed by `close` can lose them.

What follows from it, each with a test:

- The own client waits with `finish_and_wait` for its last subgroup stream and for PUBLISH_DONE before it closes the session (up to 5 seconds each).
- The relay reads what had arrived on a connection that is gone (for up to 1 second) before it treats the session as closed, so a publisher's last Object and PUBLISH_DONE are not dropped at the relay. Subgroup streams a publisher had finished are delivered whole when the publisher goes; before, they were reset when they were not written out yet.
- A writer of a subgroup stream ends when the connection is lost. It used to stay, with its share of the subscriber's budget, until someone reset it. A new test found this.
- A request the relay ended with PUBLISH_DONE is let go after 2 seconds with STOP_SENDING when the subscriber leaves its direction of the stream open. Whether a moqtail client closes that direction was not observed: no run here sends PUBLISH_DONE to a moqtail subscriber and waits.

One more finding of those tests: with `wtransport`, streams are not always accepted in the order they were opened. A test that assumed the order failed in 3 of 12 runs, and in none of 20 after the assumption was removed.

The relay now sets the QUIC limits of its connections (`quic_limits` in `crates/relay/src/main.rs`): 64 unidirectional and 264 bidirectional streams a peer may have open, 256 KiB unread a stream and 4 MiB over the connection, 4 MiB written and unacknowledged. Before, quinn's defaults applied (100 streams of each kind, 1.25 MB a stream with no bound across streams, 10 MB unacknowledged), which let one peer hold about 250 MB and kept the relay's `EXCESSIVE_LOAD` from ever being sent, because QUIC stopped the peer at 100 request streams. With the limits, one session can hold about 22 MiB (the table is in `HubConfig`). The own client keeps quinn's defaults.

```sh
mise run check
mise run interop
node spikes/moqtail/browser/relay-scenarios.mjs
```

`mise run check` ended with `OK` for all four parts. Tests: 99 in `xmoq-moqt`, 32 in `xmoq-relay-core`, 57 in `xmoq-moqt-io` (28 of them the contract cases, 14 for each connection), 16 in `xmoq-relay`, 3 in `xmoq-moqt-client`; none failed. `mise run interop`: `32 of 32 pairings ended as expected`, with the QUIC limits in place, so the moqtail clients and Chromium work within them. `relay-scenarios.mjs` printed its four `RESULT` lines as before.

Not covered by a test: the publisher path of the own client itself (`finish_and_wait` before the close). The client is written against the `wtransport` connection only and reads its triggers from stdin; what is tested is the primitive, on both connections, and the same sequence against the relay on the in-memory connection.

## Limitations and blockers

No blocker for this step.

Requirements of the draft the relay does not meet, each marked with a `FIXME:`:

- **Routing by namespace (§9.5).** A PUBLISH_NAMESPACE is accepted and not used. A SUBSCRIBE for a track that no PUBLISH offered is rejected, where the draft has the relay send a SUBSCRIBE to every publisher of a matching namespace. Every peer in these runs sends PUBLISH for its track.
- **Several publishers of one track (§9.3).** A second one is rejected.
- **Subscription Filters (§5.1.2).** Every subscriber gets what arrives after it subscribed, whatever its filter says.
- **Session close towards a browser.** No `WT_CLOSE_SESSION` (above).
- **Datagrams (§11), GOAWAY, FETCH, TRACK_STATUS, namespace subscriptions.** FETCH, TRACK_STATUS, SUBSCRIBE_NAMESPACE and SUBSCRIBE_TRACKS are answered with REQUEST_ERROR NOT_SUPPORTED and the session stays open. GOAWAY still closes the session with INTERNAL_ERROR. Datagrams are not read.
- **REQUEST_UPDATE** is refused, never applied.
- **Authorization** (§9.4, §9.5 MUST) is not done: there is no authentication yet.
- In the client, the Request IDs of requests a peer opens are not checked (the client answers them all with NOT_SUPPORTED). In the first pass it also noticed a STOP_SENDING on its PUBLISH stream only when it wrote PUBLISH_DONE; after the review it notices it at once.
- NEW_GROUP_REQUEST is not passed on (§10.2.13), delivery timeouts are not enforced (§8), a SUBSCRIBE with RENDEZVOUS_TIMEOUT is not held (§10.2.6), and the Retry Interval of a REQUEST_ERROR is always 0.

Limits of what was run:

- One Object per transfer, on one subgroup stream; two in the scenarios. No sustained traffic, no large Objects.
- A slow subscriber, a malformed track, a refused REQUEST_UPDATE, an early stream, a reset stream, a drain timeout and the limits were exercised in tests on the in-memory connection, not against a real peer.
- A subscriber that joins inside a subgroup gets a header rewritten for it (explicit Subgroup ID, FIRST_OBJECT cleared). This is covered by a test of `xmoq-relay-core` only.
- Each subscriber gets its own copy of an Object's payload, and the relay holds a whole Object in memory before forwarding it.
- Chromium only, headless, one machine, loopback. No packet loss, no real network.

## Open questions raised

1. **Does XmoQ keep `wtransport`?** With it, a browser cannot learn why the relay closed a session. See the [library comparison](2026-10-10-webtransport-library-comparison.md).
2. **Which of the limits above does XmoQ commit to**, and with what values? All are provisional; the ones for the send queue and the idle timeout decide how soon a slow or vanished peer is dropped.
3. **Does the first relay route by namespace** (§9.5), or is PUBLISH per track enough for XmoQ's own publishers?
4. **Should a relay pass on the upstream PUBLISH_DONE status and reason**, or always say TRACK_ENDED?
5. **What does a subscriber get after TOO_FAR_BEHIND?** The subscription is ended; whether the client resubscribes, and from where, is not designed.
