# The terminal viewer refuses the requests of the relay

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 3, the terminal viewer; the known gap found in [the Request IDs of requests the Rust client refuses](2026-10-24-the-request-ids-of-requests-the-rust-client-refuses.md) (the terminal viewer accepted no bidirectional stream of the peer).

## Objective

The terminal viewer read the peer's unidirectional streams alone: a request the peer opened towards it got no answer, which §3.3.2 has as REQUEST_ERROR and a FIN for a refusal, and its Request ID was not checked (§10.1). Now it reads them, refuses each once the broadcast plays (`Session::refuse_peer_request` of `xmoq-moqt`, `NOT_SUPPORTED`: §10.6.2, "The endpoint does not support the type of request"), feeds what still comes on a refused stream to its session, and closes the session with the code of a fault. The Rust client's modes that serve no request use the same function now.

The peer is `xmoq-quiet-peer` with the two cases of that record, which now also answer the client's SUBSCRIBEs, so that a viewer comes to play: `--quiet request-id-parity` (a SUBSCRIBE with the Request ID 0, a client's) and `--quiet request-id-again` (one with the ID 1 and, after its refusal, a REQUEST_UPDATE with the ID 1).

## Expected

**Written after the runs**, which were made before this record was begun: the expectation was the code §10.1 names, as for the Rust client.

- The terminal viewer closes the session with `INVALID_REQUEST_ID` (0x4) in both cases, and leaves with a failure; the peer prints `SESSION_ENDED session=0 ClosedByPeer { code: 4, ...`.
- own-rust and the page do as in the record before, with the peer that now answers their SUBSCRIBEs.

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the working copy on `main` at `lnvzpzkr`; `bun interop/quiet-peer.ts --case request-id-parity|request-id-again --client xmoq-watch|own-rust|own-browser --port 45390`, once each.

## Result

| Case | Client | The peer's line | The client |
|---|---|---|---|
| request-id-parity | xmoq-watch | `ClosedByPeer { code: 4, reason: "" }` | left with 1 |
| request-id-parity | own-rust | `ClosedByPeer { code: 4, reason: "invalid request id 0" }` | `CLOSED_FOR_A_REQUEST_OF_THE_PEER code=0x4 ...` |
| request-id-parity | own-browser | `ClosedByPeer { code: 4, reason: "Request ID 0 from the server is even" }` | |
| request-id-again | xmoq-watch | `ClosedByPeer { code: 4, reason: "" }` | left with 1 |
| request-id-again | own-rust | `ClosedByPeer { code: 4, reason: "invalid request id 1" }` | `CLOSED_FOR_A_REQUEST_OF_THE_PEER code=0x4 ...` |
| request-id-again | own-browser | `ClosedByPeer { code: 4, reason: "duplicate Request ID 1" }` | |

- As expected. The terminal viewer closes without a reason, where the two others give one: it closes every session with an empty reason.
- `mise run check` passed, with a test of `refuse_peer_request` in `xmoq-moqt` (a refusal, then a REQUEST_UPDATE with a fresh ID, with the refused one's ID, and a request of the wrong parity).
- **Not run**: a request the relay opens while the viewer does not play yet; it waits in a queue of 16 and is refused once the broadcast plays, which is read from the code.
- `mise run interop -- --base-port 45300`: 181 of 181 pairings ended as expected in 284.6 s, the six runs above among them.
