# @xmoq/moqt

MoQ Transport for the browser, written from the text of `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`).

Status: partly implemented. The client end of a session and the messages below are implemented and were run against the `moqtail` relay only. The own relay does not exist yet, so the roadmap item for this client is still open.

## Scope

Implemented: SETUP, PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE, SUBSCRIBE_OK, REQUEST_OK, REQUEST_ERROR, PUBLISH_DONE, SUBGROUP_HEADER and subgroup Objects.

Sent: FETCH (§10.12), Standalone and Joining. `MoqtConnection.fetch` sends one and gives its FETCH_OK and the bytes of its fetch stream; `decodeFetchBody` makes Objects and End of Range indicators of them (`src/core/fetch.ts` has the codec). **The shape of this part of the API is provisional.** A FETCH of the peer is decoded whole first, so that one the draft has close the session does. The session refuses by itself: with INVALID_RANGE where a Standalone range ends before it starts, with NOT_SUPPORTED for a Joining FETCH that names a SUBSCRIBE of the peer that is waiting or accepted, or a Request ID of the peer that is yet to come, and with INVALID_JOINING_REQUEST_ID for any other Joining FETCH (a relay sends a publisher none; `fixtures/joining-request-id/` has which ID is yet to come, in a table both implementations read). The Rust implementation reads the same captures (`fixtures/moqt-18/`) and the same hand-written streams (`fixtures/moqt-18-by-hand/`). Run in Chromium against the `moqtail` relay and the own one: [record](../../docs/experiments/2026-10-11-fetch-from-a-page.md).

Served: a SUBSCRIBE of the peer, which a relay sends the publisher of a namespace for a track it has no source of. `MoqtConnection.serve({ subscribe })` hands each one to the application, which answers with `accept()` or `reject()`; before `serve` is called every one is refused with NOT_SUPPORTED. `accept()` gives a `PublishedTrack`, the same as `publish()` does, so the code that sends Objects does not care which of the two made the subscription. `publish()` rejects with a `RangeError` for a Track this endpoint already publishes to the peer, by a PUBLISH of its own or by a SUBSCRIBE of the peer that waits or was accepted; before this, a second `publish()` for one Track was sent. The library keeps no Object: what was sent before is the application's to know, and it says how far it is with `parameters.largestObject` of the answer. The session refuses by itself a SUBSCRIBE for a Track it already publishes to the peer (DUPLICATE_SUBSCRIPTION) and one more request than `limits.maxPeerRequests` (64, EXCESSIVE_LOAD); the first is in a table both implementations read (`fixtures/peer-subscribe/`). **The shape of this part of the API is provisional.** Run in Chromium against both relays: [record](../../docs/experiments/2026-10-11-a-page-answers-the-relays-subscribe.md).

Served as well: a Standalone FETCH of the peer, which a relay sends a publisher for a range it does not hold; a viewer's Joining FETCH reaches the publisher as one of these. `serve({ fetch })` hands each one to the application: its range as the FETCH has it (**the End Location is the last Object plus 1, and Object 0 stands for a whole Group**), its Group Order and its FILL_TIMEOUT. `accept({ endLocation })` sends FETCH_OK and opens the fetch stream at once, and gives a `FetchWriter`: `write(object)`, `endOfRange(kind, location)` for what the publisher does not have, `close()`, `reset()`. **`close()` finishes the fetch stream, and leaves this endpoint's direction of the request stream open until the subscriber has ended its own**, for `peerStreamEndTimeoutMs` (3 seconds) at most: a subscriber that sees the request stream finished before it has read the fetch stream to its end can take the FETCH for failed, which moqtransport 0.14.0 does. The library keeps no Object and runs no timer: the application answers from what it has, in the order that was asked for, within the time the relay gives (an XmoQ relay: 5 seconds for the answer, and 5 seconds, or FILL_TIMEOUT, without a byte on the stream). An Object that goes against the order is refused, and whether an Object lies in the range is the application's to keep. A REQUEST_UPDATE for a FETCH is accepted and changes nothing. A subscriber that stops the fetch stream alone has given the FETCH up, as one that stops the request stream has. At most `limits.maxPeerFetches` (32) are open at once, and 16 writes of one stream may be under way, on a fetch stream and on a subgroup stream alike: a write is to be waited for before the next. Two tables both implementations read hold what the session answers by itself (`fixtures/peer-fetch/`). Run in Chromium, the page as the publisher behind the own relay and the `moqtail` one: [record](../../docs/experiments/2026-10-11-a-page-answers-a-fetch.md).

An AUTHORIZATION TOKEN (§10.2.2) is given to a connection once, `authorizationToken` of `connect`, as the bytes of the Token structure: the connection puts it into every PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE and FETCH it sends, before the tokens a request has of its own. It goes into SETUP as well, as the AUTHORIZATION TOKEN Setup Option (§10.3.1.4, Option Type 0x03), so that a relay can turn a connection down before any request. It is carried and not read, and no alias is registered. Without the option nothing is added.

Not implemented: the server role, the other requests opened by the peer (refused), acting on a REQUEST_UPDATE (one for a subscription this endpoint serves is answered with REQUEST_OK when it carries nothing but delivery timeouts, and refused otherwise), GOAWAY, serving a Joining FETCH, namespace subscription, datagrams, the content of authorization tokens and delivery timeouts. [The experiment record](../../docs/experiments/2026-10-10-own-ts-client-vs-moqtail.md) has the full list and what the runs exercised.

## Entries

| Import | Content |
|---|---|
| `@xmoq/moqt` | `connect`, and the types and errors of a connection, a published Track and a subscription. `createSubgroupStreamDecoder`, the decoder of one subgroup stream as bytes, which a connection uses itself: **exported for tests outside this package that read a captured stream** (`@xmoq/media` reads the one of `moqlivemock` in `fixtures/moqt-18/`), not as a second way to receive Objects |
| `@xmoq/moqt/diagnostics` | The types of the logging hooks in `ConnectOptions.diagnostics`. They show the session's internal events, which can change with the implementation |

`Subscription.update(parameters)` sends a REQUEST_UPDATE for a subscription this endpoint made with SUBSCRIBE (§10.9) and resolves with the parameters of the REQUEST_OK; a parameter that is left out stays as it was. A publisher that refuses ends the subscription (§10.9.1): the call rejects with `RequestFailedError`, and `ended` has the PUBLISH_DONE. At most 16 updates of a subscription wait for their answer (provisional), and one that gets none within `answerTimeoutMs` rejects while the subscription goes on. Not for a FETCH, a PUBLISH_NAMESPACE, or a subscription a PUBLISH of the peer made.

How a subscription, a published Track, a FETCH or the session ended is a value with a `type` (`Subscription.ended`, `PublishedTrack.ended`, `Fetch.ended`, `MoqtConnection.closed`), and every rejection is one of the error classes listed at `RequestFailedError` in `src/webtransport/endings.ts`.

`MoqtConnection.fetch()` resolves with a `Fetch` once the FETCH is sent, before any answer: the FETCH_OK and the fetch stream come on two streams, in either order, so they are two things. `response` resolves with the FETCH_OK. `body` is the fetch stream after its header, as bytes, read from the transport only as fast as it is read here; it closes when the publisher finished the stream and fails otherwise, with an error whose class says who ended it (the table is at `Fetch.body` in `src/webtransport/connection.ts`). `decodeFetchBody(connection, fetch)` takes `body` over and gives the Objects. The bytes do not carry the Group Order by which their Group IDs count; the `Fetch` remembers the one it asked for (`groupOrder`), and the caller cannot give another. A FETCH is over when it was rejected or cancelled, or when both `response` has resolved and `body` has ended. **A FETCH can get no answer at all**; it is then given up after `answerTimeoutMs` (below). One that is answered and gets no stream, or a stream that does not end, is ended by nothing here: give the call a `signal`, or call `cancel()`. A FETCH without an answer was seen with the `moqtail` relay, which passes a FETCH on to the publisher of the track, when that publisher was a client that answers none ([record](../../docs/experiments/2026-10-11-fetch-from-a-page.md)). Cancelling, by either or by cancelling `body`, stops the fetch stream at once.

A request waits `openStreamTimeoutMs` (10 seconds) for its stream, which the peer has to allow, and then fails with `RequestAbortedError` of `not-sent-in-time`; nothing else waits with it. A request has `answerTimeoutMs` (10 seconds, and as long again as its RENDEZVOUS_TIMEOUT or FILL_TIMEOUT) for its answer, from when it was written; then it alone is given up and fails with `RequestAbortedError` of `no-answer-in-time`, and the session stays open. A SUBSCRIBE, a FETCH and a PUBLISH_NAMESPACE are cancelled; a PUBLISH is ended with PUBLISH_DONE (the row "A request the peer does not answer" of [the implementation notes](../../docs/moqt-implementation-notes.md)).

A stream the peer opens has `firstBytesTimeoutMs` (10 seconds) to say what it is: a unidirectional stream its type, with its whole Subgroup Header or FETCH_HEADER, and a request its first message, whole. One that has not is stopped, and the session stays open; only `ConnectOptions.diagnostics` reports it. With 0 the time is not switched off: a stream is then stopped unless its first read says what it is. The Rust implementation has the same time, points and codes ([record](../../docs/experiments/2026-10-11-first-bytes-timeout-in-chromium.md), also on how late a page in the background runs the timer).

`PublishedTrack.done()` sends PUBLISH_DONE and then waits for the subscriber to end its direction of the request stream, for at most `peerStreamEndTimeoutMs` (3 seconds) from the moment PUBLISH_DONE and the FIN were written. It resolves with what it saw, a `PeerStreamEnd`: `fin`, `reset` with its code, `nothing-in-time`, or `session-closed`. None of them is an error. Close the session after `done()` has settled: a session closed right after the last Object can lose it, and `close()` called without waiting for `done()` can end the session before PUBLISH_DONE has gone out. `fin` says that the subscriber finished its direction, not that everything arrived; it can be read so only when the peer is an XmoQ relay and did not finish before PUBLISH_DONE ([implementation notes](../../docs/moqt-implementation-notes.md), "A publisher that ended its PUBLISH with PUBLISH_DONE"). With `peerStreamEndTimeoutMs: 0`, `done()` does not wait and stops the subscriber's direction at once. A second `done()` rejects with `PublishEndedError`.

The peer has the same time, `peerStreamEndTimeoutMs`, to end its direction after a request of it was refused, by the application or by the session, and after a FETCH that was served. **After a FETCH that was served it has that time twice**, once before this endpoint finishes its own direction and once after, so a subscriber that never ends its direction holds the stream, and its place among the requests of the peer (`limits.maxPeerRequests`), for 6 seconds. When it is over, its direction of the stream is stopped with `CANCELLED`, and the session stays open. A peer that ends its direction in time is not stopped.

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
