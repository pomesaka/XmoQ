# Stream resets and STOP_SENDING from `@xmoq/moqt` in Chromium

- Date: 2026-10-10
- Commit SHA: the commit that changes `createStreamError` in `packages/moqt/src/webtransport/connect.ts` together with this record. The runs were made on its working copy on top of `58cd373`
- Roadmap item: Phase 0b, own TypeScript browser client. Follows the defect found in [the disconnect record](2026-10-10-disconnects-through-own-relay.md): `Subscription.cancel()` threw in a real browser

## Objective

Make `@xmoq/moqt` reset a stream and stop a sender with a code in a real Chromium, and find out whether other calls of browser APIs in `packages/moqt/src/webtransport/` are wrong in a way its tests cannot show, because they play the transport themselves.

### What the specification asks for

W3C WebTransport, Candidate Recommendation Snapshot of 30 July 2026 (`https://www.w3.org/TR/webtransport/`), fetched on 2026-10-10. The Editor's Draft of 6 October 2026 (`https://w3c.github.io/webtransport/`) has the same text for the first three rows.

| # | Text | Where |
|---|---|---|
| 1 | `constructor(optional DOMString message = "", optional WebTransportErrorOptions options = {});` with `dictionary WebTransportErrorOptions { WebTransportErrorSource source = "stream"; [Clamp] unsigned long? streamErrorCode = null; };` | §12, IDL |
| 2 | To abort a `WebTransportSendStream` with reason: "Let code be 0." "If reason is a WebTransportError and reason.[[StreamErrorCode]] is not null, then set code to reason.[[StreamErrorCode]]." "Abort sending on stream.[[InternalStream]] with code" | §7.4 |
| 3 | To cancel a `WebTransportReceiveStream` with reason: the same two steps for the code | §9.3 |
| 4 | When the server stops a send stream, or resets a receive stream: "Let error be a newly created WebTransportError whose source is "stream" and streamErrorCode is code", and the stream is errored with it | §7.5, §9.4 |
| 5 | When the session ends, every stream is errored with the error of the cleanup; `close()` cleans up "with AbortError and closeInfo" | "cleanup a WebTransport", §6.4 |
| 6 | `dictionary WebTransportCloseInfo { unsigned long closeCode = 0; USVString reason = ""; };` | §6.10 |
| 7 | `WebTransportOptions` has `sequence<DOMString> protocols = [];` and `serverCertificateHashes`; the `protocol` attribute "returns the application-level protocol selected by the server, if any. Otherwise, an empty string." | §6.2, §6.3 |

So a code reaches the peer only when `abort()` or `cancel()` is given a `WebTransportError` that has it; anything else is sent as 0.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Browser | Chromium 156.0.8078.4, new headless, launched by `playwright-core` 1.64.0 with `channel: "chromium"` |
| Relay and own-rust | `xmoq-relay` and `xmoq-moqt-client` 0.0.0 at `58cd373`, `web-transport-quinn` 0.13.2, debug builds |
| Toolchain | `bun` 1.3.12, TypeScript 6.0.3 |

## Commands

From the repository root, after `cargo build --locked -p xmoq-moqt-client -p xmoq-relay`:

```sh
bun spikes/wt-error-form/constructor.ts
bun spikes/wt-error-form/codes.ts
mise run check
mise run interop
```

`constructor.ts` builds `WebTransportError` in a page in several forms. `codes.ts` publishes a track from a page on `@xmoq/moqt`, resets a subgroup stream with `0x12` while an own-rust client subscribes through the own relay, and closes the session with code 5.

## Observed results

### The constructor: Chromium differs from the specification

| Call in Chromium 156.0.8078.4 | Result |
|---|---|
| `new WebTransportError("m", { streamErrorCode: 1 })`, the form of the specification | `TypeError: Failed to construct 'WebTransportError': The provided value is not of type 'WebTransportErrorInit'.` |
| `new WebTransportError({ message: "m", streamErrorCode: 1 })` | an error with `message` "m", `source` "stream", `streamErrorCode` 1 |
| `new WebTransportError("m")` | the same TypeError |
| `new WebTransportError()` | an error with `streamErrorCode` null |
| `new WebTransportError({ streamErrorCode: c })` for `c` of 0, 255 | `streamErrorCode` 0, 255 |
| the same for `c` of 256, 65535, 65536, 4294967295, 4294967296 | `streamErrorCode` 255 every time |
| the same for `c` of -1, 1.5 | `streamErrorCode` 0, 1 |
| `new WebTransportError({ source: "session" })` | `source` "stream": the dictionary has no `source` |

`@xmoq/moqt` used the first form for every reset and every STOP_SENDING. It now tries the form of the specification and, on a TypeError or an error without the code, the dictionary. The order matters: a browser that follows the specification reads a dictionary given first as the message and builds an error without a code, which would be sent as 0.

**Chromium keeps only 8 bits of the code.** The specification has `[Clamp] unsigned long?`, 32 bits (row 1); Chromium clamps to 255. A reset with a larger code would reach the peer as 255. `createStreamError` compares the code of what it built and throws when it differs, and `SubgroupWriter.reset(code)`, the one place where an application gives a code, now rejects with a `RangeError` for anything but a whole number from 0 to 255, before the stream is touched. The codes the package sends by itself are `0x0`, `0x1` and `0x12`, and every Stream Reset Error Code of draft 18 (§3.3.3) is `0x12` or less. The Rust implementation sends any 32-bit code, so the two differ in what an application can send; a browser client cannot send more.

### In Chromium, after the change

| What the page did | What the other side saw |
|---|---|
| `Subscription.cancel()` on a subscription through the own relay | The relay ended the request and opened no stream for the Groups sent after. `Subscription.ended` was `{"type":"cancelled"}`, and the page had taken Group 0 only. The interop run `own-browser cancels its subscription` passes; it was listed as failing |
| `SubgroupWriter.reset(0x12)` on an open subgroup stream | The relay printed `RESET_DATA out=0 code=0x12`, and the own-rust subscriber `STREAM_ABORTED uni#1 code=0x12` |
| `MoqtConnection.close(5, "bye from the page")` | The relay printed `CONNECTION_ENDED session=0 ClosedByPeer { code: 5, reason: "bye from the page" }` |

**That a STOP_SENDING carries its code to the peer was not observed.** Neither the relay nor own-rust prints the code of a STOP_SENDING it receives. The error given to `cancel()` is built by the same function as the one given to `abort()`, and by row 3 the browser reads the code from it in the same way.

### The other calls of browser APIs

Every place where `packages/moqt/src/webtransport/` touches the browser, and how it is known to be right. "Comparison" is [the library comparison](2026-10-10-webtransport-library-comparison.md), whose page read these values in the same Chromium.

| Call | In | Known from |
|---|---|---|
| `new WebTransport(url, { protocols, serverCertificateHashes })` | `connect.ts` | Chromium: every interop run connects with both. Row 7 |
| `webTransport.ready`, and `closed` observed so that a failed connect is not unhandled | `connect.ts` | Chromium, noted in the code |
| `webTransport.protocol`, read with `Reflect.get` | `connect.ts` | Chromium: "moqt-18" in the interop runs. Row 7. A browser without it is refused by `protocolRefusal` |
| `webTransport.close({ closeCode, reason })` | `connect.ts`, `connection.ts` | Chromium: code 5 and its reason reached the relay (above). Row 6 |
| `closed` resolving with `{ closeCode, reason }` when the peer closes | `connection.ts` | Chromium, in the comparison: `{"closeCode":3,"reason":"protocol violation"}` from a `web-transport-quinn` server. Not run again here |
| `closed` rejecting when the connection is lost | `connection.ts` | Chromium, in the comparison: a `WebTransportError` with `source` "session" |
| `createUnidirectionalStream()`, `createBidirectionalStream()`, without options | `stream-io.ts` | Chromium: every interop run |
| `incomingUnidirectionalStreams`, `incomingBidirectionalStreams`, read with `getReader()` | `stream-io.ts` | Chromium: every interop run |
| `writer.write(Uint8Array)`, `writer.close()` | `stream-io.ts` | Chromium: every interop run |
| `reader.read()` giving `Uint8Array` chunks | `stream-io.ts` | Chromium: every interop run. A chunk of another type is a fatal error of the session |
| `writer.abort(error)` with the error of `createStreamError` | `stream-io.ts` | Chromium: `0x12` reached the subscriber (above). Row 2 |
| `reader.cancel(error)` with the same error | `stream-io.ts` | Specification only (row 3). The call no longer throws in Chromium; the code on the wire was not observed |
| `writer.closed` rejecting with a `WebTransportError` when the peer sends STOP_SENDING, read through `source` and `streamErrorCode` | `stream-io.ts`, `connect.ts` | Chromium, in the comparison: `streamErrorCode` 5 in 120 of 120. Row 4 |
| `reader.read()` rejecting with a `WebTransportError` when the peer resets | `stream-io.ts`, `connect.ts` | Chromium, in the comparison: `streamErrorCode` 18. Row 4 |
| `error.source === "stream"` to tell a stream's end from the session's | `connect.ts` | Rows 4 and 5. In Chromium the two values were seen in the comparison; that every stream of an ended session fails with "session" was not checked one by one |
| `new WebTransportError(...)` | `connect.ts` | **Was wrong in Chromium**; see above |

No second call of the same kind was found: the constructor is the only place where the package builds a browser object from arguments other than `new WebTransport`, whose options the interop runs exercise.

### The tests

The transport the package's tests play built its stream errors with a class of its own, which took any arguments. It now goes through the adapter the browser uses (`adaptWebTransport`), with an error class that is as strict as Chromium's: one dictionary, and a TypeError for a string. It clamps the code to 255 and gives a page no way to set `source`, as Chromium does. With the old call restored, 12 of the 301 tests fail; with the change all pass. A second class in the tests takes the form of the specification, so both forms are built, and one test fails when the dictionary is tried first.

`mise run check` passed with 301 TypeScript tests. `mise run interop` passed with 45 of 45 pairings.

## Limitations and blockers

- **Chromium only.** Firefox and Safari were not run. A browser whose constructor follows the specification is covered by a test class, not by a browser.
- The code of a STOP_SENDING on the wire was not observed (above).
- The rows marked "comparison" were measured against the spike server of that record, not against the own relay, and not again after this change.
- `spikes/wt-error-form/` is outside lint and review.

## Open questions raised

- Should the interop page offer a reset of a subgroup stream, so that the matrix checks the code of a reset in every run and not only this spike?
- Is there a peer that can report the code of a STOP_SENDING it receives? The Rust client could print it.
