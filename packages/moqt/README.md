# @xmoq/moqt

MoQ Transport for the browser, written from the text of `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`).

Status: partly implemented. The client end of a session and the messages below are implemented and were run against the `moqtail` relay only. The own relay does not exist yet, so the roadmap item for this client is still open.

## Scope

Implemented: SETUP, PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE, SUBSCRIBE_OK, REQUEST_OK, REQUEST_ERROR, PUBLISH_DONE, SUBGROUP_HEADER and subgroup Objects.

Sent, and not served: FETCH (§10.12), Standalone and Joining. `MoqtConnection.fetch` sends one and gives its FETCH_OK and the bytes of its fetch stream; `decodeFetchBody` makes Objects and End of Range indicators of them (`src/core/fetch.ts` has the codec). **The shape of this part of the API is provisional.** A FETCH of the peer is decoded whole and then refused, so that one the draft has close the session does: with INVALID_RANGE where a Standalone range ends before it starts, with INVALID_JOINING_REQUEST_ID for a Joining FETCH that names no SUBSCRIBE of the peer that is waiting or accepted, and with NOT_SUPPORTED otherwise. The Rust implementation reads the same captures (`fixtures/moqt-18/`) and the same hand-written streams (`fixtures/moqt-18-by-hand/`). Run in Chromium against the `moqtail` relay and the own one: [record](../../docs/experiments/2026-10-11-fetch-from-a-page.md).

Served: a SUBSCRIBE of the peer, which a relay sends the publisher of a namespace for a track it has no source of. `MoqtConnection.serve({ subscribe })` hands each one to the application, which answers with `accept()` or `reject()`; before `serve` is called every one is refused with NOT_SUPPORTED. `accept()` gives a `PublishedTrack`, the same as `publish()` does, so the code that sends Objects does not care which of the two made the subscription. `publish()` rejects with a `RangeError` for a Track this endpoint already publishes to the peer, by a PUBLISH of its own or by a SUBSCRIBE of the peer that waits or was accepted; before this, a second `publish()` for one Track was sent. The library keeps no Object: what was sent before is the application's to know, and it says how far it is with `parameters.largestObject` of the answer. The session refuses by itself a SUBSCRIBE for a Track it already publishes to the peer (DUPLICATE_SUBSCRIPTION) and one more request than `limits.maxPeerRequests` (64, EXCESSIVE_LOAD); the first is in a table both implementations read (`fixtures/peer-subscribe/`). **The shape of this part of the API is provisional.** Run in Chromium against both relays: [record](../../docs/experiments/2026-10-11-a-page-answers-the-relays-subscribe.md).

Not implemented: the server role, the other requests opened by the peer (refused), REQUEST_UPDATE (refused), GOAWAY, serving a FETCH, namespace subscription, datagrams, the content of authorization tokens and delivery timeouts. [The experiment record](../../docs/experiments/2026-10-10-own-ts-client-vs-moqtail.md) has the full list and what the runs exercised.

## Entries

| Import | Content |
|---|---|
| `@xmoq/moqt` | `connect`, and the types and errors of a connection, a published Track and a subscription |
| `@xmoq/moqt/diagnostics` | The types of the logging hooks in `ConnectOptions.diagnostics`. They show the session's internal events, which can change with the implementation |

How a subscription, a published Track, a FETCH or the session ended is a value with a `type` (`Subscription.ended`, `PublishedTrack.ended`, `Fetch.ended`, `MoqtConnection.closed`), and every rejection is one of the error classes listed at `RequestFailedError` in `src/webtransport/endings.ts`.

`MoqtConnection.fetch()` resolves with a `Fetch` once the FETCH is sent, before any answer: the FETCH_OK and the fetch stream come on two streams, in either order, so they are two things. `response` resolves with the FETCH_OK. `body` is the fetch stream after its header, as bytes, read from the transport only as fast as it is read here; it closes when the publisher finished the stream and fails otherwise, with an error whose class says who ended it (the table is at `Fetch.body` in `src/webtransport/connection.ts`). `decodeFetchBody(connection, fetch)` takes `body` over and gives the Objects. The bytes do not carry the Group Order by which their Group IDs count; the `Fetch` remembers the one it asked for (`groupOrder`), and the caller cannot give another. A FETCH is over when it was rejected or cancelled, or when both `response` has resolved and `body` has ended. **A FETCH can get no answer at all, and nothing here ends it then**: give the call a `signal`, or call `cancel()`. It was seen with the `moqtail` relay, which passes a FETCH on to the publisher of the track, when that publisher was a client that answers none ([record](../../docs/experiments/2026-10-11-fetch-from-a-page.md)). Cancelling, by either or by cancelling `body`, stops the fetch stream at once.

A stream the peer opens has `firstBytesTimeoutMs` (10 seconds) to say what it is: a unidirectional stream its type, with its whole Subgroup Header or FETCH_HEADER, and a request its first message, whole. One that has not is stopped, and the session stays open; only `ConnectOptions.diagnostics` reports it. With 0 the time is not switched off: a stream is then stopped unless its first read says what it is. The Rust implementation has the same time, points and codes ([record](../../docs/experiments/2026-10-11-first-bytes-timeout-in-chromium.md), also on how late a page in the background runs the timer).

`PublishedTrack.done()` sends PUBLISH_DONE and then waits for the subscriber to end its direction of the request stream, for at most `peerStreamEndTimeoutMs` (3 seconds) from the moment PUBLISH_DONE and the FIN were written. It resolves with what it saw, a `PeerStreamEnd`: `fin`, `reset` with its code, `nothing-in-time`, or `session-closed`. None of them is an error. Close the session after `done()` has settled: a session closed right after the last Object can lose it, and `close()` called without waiting for `done()` can end the session before PUBLISH_DONE has gone out. `fin` says that the subscriber finished its direction, not that everything arrived; it can be read so only when the peer is an XmoQ relay and did not finish before PUBLISH_DONE ([implementation notes](../../docs/moqt-implementation-notes.md), "A publisher that ended its PUBLISH with PUBLISH_DONE"). With `peerStreamEndTimeoutMs: 0`, `done()` does not wait and stops the subscriber's direction at once. A second `done()` rejects with `PublishEndedError`.

## Layout

| Path | Content | May import |
|---|---|---|
| `src/core/` | Wire codec and session state: bytes in, messages and outputs out | only `src/core/` |
| `src/webtransport/` | The connection: `stream-io.ts` moves bytes between transport streams and a session, `connection.ts` pairs requests with responses and builds what the application holds. `connect.ts` is the only file that touches the browser's `WebTransport` | `src/core/` |
| `test/core/` | Unit tests of the codec and the session. `moqtail-fixtures.test.ts` pins bytes captured from moqtail | `src/` |
| `test/webtransport/` | Tests of the connection against a transport played by the test | `src/` |

`src/core/` has no I/O, so that the same code can sit behind a different transport and be tested with bytes alone. Two checks enforce it. `tsconfig.core.json` type-checks `src/core/` with no DOM library and no `@types` package, so a stream, a timer, `fetch` or `WebTransport` there does not compile; the two globals it may use are declared in `src/core/runtime-globals.d.ts`. `noRestrictedImports` in the root `biome.jsonc` rejects an import of `src/webtransport/` from `src/core/`.

## Commands

From the repository root, the checks CI runs (Biome, oxlint, type check and tests, with the tool versions of `mise.toml`):

```sh
mise run check:ts
```

While working in this directory, after `bun install` in the repository root:

```sh
bun run typecheck
bun test
```

Runs against moqtail peers in a real browser are in [`interop/`](../../interop/README.md).
