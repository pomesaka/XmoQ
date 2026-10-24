# The Request IDs of requests the Rust client refuses

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the own MoQ Transport; a known gap of the implementation notes ("Request IDs of requests the peer opens, in the subscribe and fetch modes of the Rust client").

## Objective

Draft 18 §10.1 has an endpoint close the session with `INVALID_REQUEST_ID` for a Request ID of the wrong parity or one used before, and "Each ... REQUEST_UPDATE ... consumes a Request ID". In its `subscribe` and `fetch` modes the Rust client answered every request of the peer with `NOT_SUPPORTED` from a task that had no session, so it checked no Request ID and read nothing of a refused stream. Now that task keeps a session for the peer's requests (`refuse_requests` in `crates/moqt-client/src/io.rs`): the IDs are checked, what comes on a refused stream is fed to it, and a fault closes the session with its code (`CLOSED_FOR_A_REQUEST_OF_THE_PEER code=...`). `@xmoq/moqt` did both already; the same runs hold it to them.

The peer is `xmoq-quiet-peer`: `--quiet request-id-parity` sends the client a SUBSCRIBE with the Request ID 0, a client's; `--quiet request-id-again` sends one with the ID 1 and, after its refusal, a `REQUEST_UPDATE` with the ID 1 on the same stream.

## Expected

**Written after the runs**: the expectation was the code §10.1 names, and the runs were made before this record was begun.

- Both clients close the session with `INVALID_REQUEST_ID` (0x4) in both cases; the peer prints `SESSION_ENDED session=0 ClosedByPeer { code: 4, ...`.
- In the second case the client answers the SUBSCRIBE first (`REQUEST_ERROR` `NOT_SUPPORTED`).

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the working copy on `main` at `mykszzlw`; `bun interop/quiet-peer.ts --case request-id-parity|request-id-again --client own-rust|own-browser --port 45390`, once each.

## Result

| Case | Client | The peer's line | The client's |
|---|---|---|---|
| request-id-parity | own-rust | `ClosedByPeer { code: 4, reason: "invalid request id 0" }` | `CLOSED_FOR_A_REQUEST_OF_THE_PEER code=0x4 error=InvalidRequestId(0)` |
| request-id-parity | own-browser | `ClosedByPeer { code: 4, reason: "Request ID 0 from the server is even" }` | |
| request-id-again | own-rust | `ClosedByPeer { code: 4, reason: "invalid request id 1" }`, after the client's `REQUEST_ERROR` `NOT_SUPPORTED` | `... error=InvalidRequestId(1)` |
| request-id-again | own-browser | `ClosedByPeer { code: 4, reason: "duplicate Request ID 1" }`, after the page's `REQUEST_ERROR` `NOT_SUPPORTED` | |

- As expected. `mise run check` passed.
- **Not run**: the same against the client before this change; that it checked nothing is read from its code, which had no session for these requests. The terminal viewer reads no request of the peer at all (it accepts no bidirectional stream), which leaves such a request without an answer; that is another gap, not looked at here.
- `mise run interop -- --base-port 45300`: 179 of 179 pairings ended as expected in 289.3 s, the four runs above among them.
