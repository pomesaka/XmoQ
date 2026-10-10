# Switch of the Rust WebTransport library to web-transport-quinn

- Date: 2026-10-10
- Commit SHA: the commit that switches the library together with this record. The runs were made on its working copy on top of `ef0d72b`
- Roadmap item: Phase 0b, the WebTransport library under the own Rust relay and client

## Objective

Move `crates/moqt-io`, the relay and the Rust client from `wtransport` 0.7.2 to `web-transport-quinn` 0.13.2, so that a browser learns the code and reason of a session the relay closes. The [comparison](2026-10-10-webtransport-library-comparison.md) is the basis of that choice. This record says what the switch changed, measured with the contract tests of the connection traits, the interop runs and a page in Chromium.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4 (23E214), arm64, loopback only |
| Library | `web-transport-quinn` 0.13.2 (the latest on crates.io, read 2026-10-10) with the feature `ring` and without the default `aws-lc-rs`; `web-transport-proto` 0.6.2, `web-transport-trait` 0.5.0, `kio` 0.5.9 |
| Below it | `quinn` 0.11.12, `quinn-proto` 0.11.19, `rustls` 0.23.45, `ring` 0.17.14, `tokio` 1.49.0 (root `Cargo.lock`) |
| Own code | `xmoq-moqt-io`, `xmoq-relay`, `xmoq-relay-core`, `xmoq-moqt-client`, all 0.0.0 |
| Peers | moqtail `relay` 0.15.0, the moqtail Rust client on `wtransport` 0.7.2, `moqtail` npm 0.13.0, `@xmoq/moqt` of this repository |
| Browser | Chromium 156.0.8078.4, headless, through `playwright` 1.64.0 |
| Specifications | `draft-ietf-moq-transport-18`; `draft-ietf-webtrans-http3-16` §4.4 |

Provisional settings, none of them project decisions: the QUIC limits of the relay are the ones of the [relay record](2026-10-10-own-relay-vs-moqtail-peers.md) (64 unidirectional and 264 bidirectional streams, 256 KiB a stream, 4 MiB a connection each way), now set on a quinn transport configuration built by hand, because the library's server builder takes none. The client keeps quinn's defaults and sends no keep-alive; the relay's keep-alive (3 s) keeps the connection up.

## Commands

```sh
mise run check
mise run interop
cargo build -p xmoq-relay
node spikes/wt-library-comparison/relay-close.mjs
cargo tree --workspace -e normal,build --prefix none
```

`crates/moqt-io/src/contract.rs` holds the contract of the connection traits as 18 cases that run against the in-memory connection and against a client and a server of the library over 127.0.0.1. They are part of `mise run check`.

## Observed results

**Passed, with one sentence of the contract that had to be weakened.**

### The contract of the connection traits

17 of the 18 cases passed on the new library, four of them new in this change: the code and reason of a close reach the peer (both directions), a reason longer than the 1024 bytes of WebTransport is cut at a character boundary (the library sends it uncut), and a reset that has arrived takes the bytes the receiver had not read. The other 13 passed as they were written for the old library. The case on the stream limit only needed its number: a `web-transport-quinn` client, like a `wtransport` one, opens one HTTP/3 stream of the three the limit leaves room for, so opening waits at exactly 10 streams for a limit of 8.

One case failed and cannot be met:

> Once the connection is gone, the streams that had arrived before are still accepted, and then the call fails.

With this library, a stream that arrived and was not accepted yet is lost when the connection closes. quinn itself still hands such a stream over. The library reads the WebTransport stream header before it gives a stream to its caller, and its accept loop returns quinn's connection error before it looks at the streams whose header is being read or is already read (`poll_accept_uni`, `src/session.rs` lines 793 to 846: the error return is at 808 to 810, the list of pending streams is polled at 819; `poll_accept_bi`, lines 873 to 918, is built the same way). `wtransport` accepted and parsed streams in a task of its own, ahead of the caller.

Bytes and the FIN of a stream that was already accepted are still read after the connection is gone, as before.

The sentence now reads: a stream that was not accepted when the connection went may be lost, also when it had arrived. The in-memory connection always loses it, so that code that relies on it fails in tests.

### What was done about the lost stream

A publisher that sends its last Object on a new stream and closes cannot rely on "the peer's transport acknowledged it" any more. The answer is one level up and uses what PUBLISH_DONE already carries:

- The relay, after PUBLISH_DONE, ends its direction of the request stream with a FIN once as many subgroup streams as the Stream Count named have ended there. When its wait for them (2 s) runs out it resets that direction instead.
- The Rust client, after PUBLISH_DONE and FIN, waits up to 3 s for the peer to end the stream and prints what it saw: `PUBLISH_DONE_PEER_ENDED how=FIN`, a reset with its code, or `nothing in time`. Then it closes the session. The line says nothing about delivery: a FIN means "every counted stream has ended here" only from an XmoQ relay, and the client cannot tell what kind of peer it has.

Draft 18 was read for this (§10.11, §5.1, §5.1.1, §3.3.2): it gives the subscriber's direction no meaning after PUBLISH_DONE and sets no time for closing it, so the FIN is XmoQ's convention and a peer that knows nothing of it sees an ordinary end of the stream. The TypeScript client does not wait yet.

Tests on the in-memory connection: a publisher that waits for the relay's FIN loses nothing; one that closes at once loses its last Object, every time; the relay resets the stream when the counted streams do not come.

In the interop runs the client saw a FIN in all 6 runs with `own-rust` as the publisher that ends its track, 3 through each relay (`PUBLISH_DONE_PEER_ENDED how=FIN`; the runs were made while the line still read `PUBLISH_DONE_CONFIRMED peer=FIN`, and repeated with the new wording). So the moqtail relay 0.15.0 finishes the stream after PUBLISH_DONE as well. What it means by that was not looked at, and for the 3 runs through it the FIN confirms nothing.

### Interop

`mise run check` ended with `OK` for all four parts. Tests: 99 in `xmoq-moqt`, 32 in `xmoq-relay-core`, 64 in `xmoq-moqt-io` (36 of them the contract cases), 18 in `xmoq-relay`, 3 in `xmoq-moqt-client`; none failed.

`mise run interop`: `32 of 32 pairings ended as expected`, through the moqtail relay and through the own relay, without a change to `interop/`.

How codes looked between the two libraries, from the logs of that run:

| Direction | Seen |
|---|---|
| The own client stops or resets a request stream with CANCELLED (`0x1`), towards the moqtail relay | The relay logs `Error reading from stream: Reset(91141958510812)`, which is `0x52e4a40fa8db + 1`, the mapped value. It goes on serving; the run ends as expected |
| A moqtail client closes its session, towards the own relay | `ClosedByPeer { code: 0, reason: "done" }` (moqtail Rust client) and `ClosedByPeer { code: 0, reason: "" }` (moqtail-ts in Chromium, and the own clients). The moqtail Rust client sends no capsule; its QUIC application close is read as the session close. A close with a code from `0x100` on, where the codes of HTTP/3 and QPACK are, is reported as a lost connection instead (unit test, not provoked with a peer) |
| A moqtail peer resets or stops a stream with a code, towards own code | Not seen in these runs. By the library's source it arrives as `InvalidReset` or `InvalidStopped`, which the connection reports as a reset or stop without a code |

A regression found on the way: with the first version of the switch the relay closed sessions that were ending anyway with PROTOCOL_VIOLATION, "the peer stopped the relay's control stream". An endpoint that lets go of its receiving halves has quinn send STOP_SENDING for them, and closing a session now takes a round trip (the capsule) where it used to be immediate, so the stop arrived first. The own client now keeps its readers until the session is closed, and the relay takes a stop of its control stream for a peer leaving when the connection ends within 200 ms. After that no run logged a session closed by the relay.

### What a browser sees

`relay-close.mjs`, 5 rounds each, Chromium 156.0.8078.4 against the own relay:

```text
[relay] CLOSE_SESSION session=0 code=0x3 reason="unknown message type 0x3f"
[relay] STOP_DATA session=5 stream=0 code=0x1
[driver] RESULT {"protocol":"moqt-18","violation":[{"count":5,"outcome":{"resolved":{"closeCode":3,"reason":"unknown message type 0x3f"}}}],"early":[{"count":5,"outcome":{"rejected":{"name":"WebTransportError","message":"Received STOP_SENDING.","source":"stream","streamErrorCode":1}}}]}
```

| What the relay did | What the page read |
|---|---|
| Closed the session with PROTOCOL_VIOLATION (`0x3`) and the reason `unknown message type 0x3f`, after the page opened a request stream with a message type that does not exist | `WebTransport.closed` resolved with `closeCode: 3` and that reason, 5 of 5 |
| Stopped a subgroup stream whose Track Alias no PUBLISH explained, with CANCELLED (`0x1`), after 2 s | The page's write failed with `streamErrorCode: 1`, 5 of 5 |

With `wtransport` the first row was `Connection lost.` without a code.

### Dependencies

| | Before (`wtransport`) | After |
|---|---|---|
| Packages in the root `Cargo.lock` | 153 | 195 |
| Crates built for this host (`cargo tree --workspace`, normal and build edges, unique) | not measured | 114 |
| `wtransport`, `wtransport-proto` | present | gone |
| `aws-lc-rs`, `aws-lc-sys` | absent | absent |
| Crypto backend | `ring` | `ring` |

The lock file grew because it lists every platform and optional path: `rustls-platform-verifier` and its Android and Windows parts, `loom`, `tracing-subscriber`, the `futures` crates, `kio`, `sfv`, `http`. `httlib-huffman`, `octets`, `pem`, `sha2` and their parts left with `wtransport`. `rcgen` 0.14.10 is a test-only dependency of `xmoq-moqt-io`, for the certificate of the loopback server.

`crates/moqt-io/src/codes.rs`, XmoQ's own mapping of stream codes, is deleted; the library maps.

## Limitations and blockers

No blocker.

- **A stream that arrived and was not accepted can be lost at a close** (above). The relay keeps an accept call waiting for each kind of stream, up to its limit on concurrent readers, so the window is small; it is not closed. What covers it is the FIN after PUBLISH_DONE, in Rust only.
- A publisher that does not send PUBLISH_DONE and closes, which is what the moqtail clients do, gets what the relay had accepted by then.
- The TypeScript client does not wait for the relay's FIN before closing.
- The publisher path of the Rust client is covered by the interop runs only; the tests run the same sequence against the relay on the in-memory connection.
- The Rust client has no QUIC limits and no keep-alive of its own. Setting them together with a pinned certificate hash needs a certificate verifier written by hand (`ServerFingerprints` is private), which was not done.
- A reset or stop from a moqtail peer towards own code was not provoked.
- A stream code on one of the reserved codepoints inside the WebTransport range is decoded by the library as if it were a code (`error_from_http3` checks the range only). XmoQ's own mapping, deleted in this change, reported it as a reset without a code. Listed in the known gaps.
- A STOP_SENDING whose code does not arrive, and a stream the page resets right after its first write that never surfaces (both in the comparison), were not looked at again. The code treats a missing code as none.
- Chromium only, headless, one machine, loopback.
- The browser check is a script under `spikes/`. It is not part of `mise run interop` yet.

## Open questions raised

1. **Is the FIN after PUBLISH_DONE worth proposing upstream of XmoQ**, in the MoQ working group or to moqtail, or does it stay a private convention?
2. **Should the relay answer a publisher that leaves without PUBLISH_DONE** in some way, or is that case simply "what arrived, arrived"?

## Draft of a report to web-transport-quinn

Not sent. Sending it is with the project owner.

> **Title:** `accept_uni` / `accept_bi` drop streams that arrived before the connection closed
>
> **Version:** web-transport-quinn 0.13.2 (quinn 0.11.12)
>
> **What happens:** A peer opens a stream, writes to it, finishes it, waits until quinn reports everything acknowledged, and then closes the session. If the local side had not called `accept_uni` before the close arrived, the next `accept_uni` returns the session error and the stream is never returned, although its bytes and FIN are in quinn's buffers. `quinn::Connection::accept_uni` on its own still returns such a stream: quinn checks for incoming streams before it checks the connection error.
>
> **Where:** `SessionAccept::poll_accept_uni` in `src/session.rs` (lines 793 to 846 in 0.13.2). The loop polls `self.accept_uni` first and returns `Poll::Ready(Err(..))` as soon as quinn reports the connection error (lines 808 to 810). `self.pending_uni`, which holds streams whose header is still being read or was read and not yet handed out, is polled only after that (line 819), so it is never reached once the connection is closed. A stream that quinn returned in an earlier iteration of the same call is pushed to `pending_uni` and then lost the same way. `poll_accept_bi` (lines 873 to 918) has the same shape.
>
> **To reproduce:**
>
> 1. Connect a client and a server session.
> 2. Client: `open_uni`, `write_all(b"abc")`, `finish()`, then wait for the stream to be acknowledged (`stopped()` returning `Ok(None)`).
> 3. Client: `close(0, b"done")`.
> 4. Server: wait for `closed()`, then call `accept_uni()`.
>
> Observed: `accept_uni()` fails with the session error. Expected: it returns the stream, which reads `abc` and then ends, and the call after it fails.
>
> **Why it matters:** an endpoint that sends its last data on a new stream and then closes cannot know whether the peer's application will see that stream, even when the transport has acknowledged it.
>
> **Possible change:** when quinn's accept reports an error, keep the error, go on draining `pending_uni` / `pending_bi`, and return the error once they are empty.
