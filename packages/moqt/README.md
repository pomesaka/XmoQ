# @xmoq/moqt

MoQ Transport for the browser, written from the text of `draft-ietf-moq-transport-18` (WebTransport subprotocol `moqt-18`).

Status: partly implemented. The client end of a session and the messages below are implemented and were run against the `moqtail` relay only. The own relay does not exist yet, so the roadmap item for this client is still open.

## Scope

Implemented: SETUP, PUBLISH_NAMESPACE, PUBLISH, SUBSCRIBE, SUBSCRIBE_OK, REQUEST_OK, REQUEST_ERROR, PUBLISH_DONE, SUBGROUP_HEADER and subgroup Objects.

Decoded and encoded, and not served or sent yet: FETCH, FETCH_OK, and the fetch stream (FETCH_HEADER, its Objects and End of Range indicators; `src/core/fetch.ts`). A FETCH of the peer is decoded whole and then refused with NOT_SUPPORTED, so that one the draft has close the session does; a stream of type FETCH_HEADER still ends the session with INTERNAL_ERROR. The Rust implementation reads the same captures (`fixtures/moqt-18/`) and the same hand-written streams (`fixtures/moqt-18-by-hand/`).

Not implemented: the server role, requests opened by the peer (answered with NOT_SUPPORTED), REQUEST_UPDATE (refused), GOAWAY, serving and sending a FETCH, namespace subscription, datagrams, the content of authorization tokens and delivery timeouts. [The experiment record](../../docs/experiments/2026-10-10-own-ts-client-vs-moqtail.md) has the full list and what the runs exercised.

## Entries

| Import | Content |
|---|---|
| `@xmoq/moqt` | `connect`, and the types and errors of a connection, a published Track and a subscription |
| `@xmoq/moqt/diagnostics` | The types of the logging hooks in `ConnectOptions.diagnostics`. They show the session's internal events, which can change with the implementation |

How a subscription, a published Track or the session ended is a value with a `type` (`Subscription.ended`, `PublishedTrack.ended`, `MoqtConnection.closed`), and every rejection is one of the error classes listed at `RequestFailedError` in `src/webtransport/endings.ts`.

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
