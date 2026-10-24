# A GOAWAY the clients take

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a session that goes on; a known gap of the implementation notes (GOAWAY ended the session with `INTERNAL_ERROR`).

## Objective

A server that is to stop says so with GOAWAY (draft 18 §3.6, §10.4), and may name where to go. Both implementations decoded no GOAWAY and closed the session with `INTERNAL_ERROR`. Now they take one on the control stream by the rules of §10.4, open no request after it, and the clients go: the page's `keepPlaying` and the terminal viewer close the session with `NO_ERROR` and connect to the New Session URI; the Rust client closes with `NO_ERROR` and leaves. The relay takes one from a client and asks it for nothing more.

The peer is `xmoq-quiet-peer --quiet goaway`: on its first session it sends GOAWAY as soon as the client's SETUP has come, naming `https://127.0.0.1:<port>/moved` of itself, with a Timeout of 5,000 ms and the Request ID 0, and answers SUBSCRIBE with `SUBSCRIBE_OK` and nothing on it. Its later sessions get no GOAWAY.

## Expected, written before the full interop run

**Each of the three was run once alone before this was written**, while the runs were made; what follows is what they are held to.

- **own-rust** (`--mode subscribe`): prints `GOAWAY uri=https://127.0.0.1:<port>/moved timeout_ms=5000 request_id=0`, leaves with 0; the peer prints `SESSION_ENDED session=0 ClosedByPeer { code: 0,` and no second session.
- **The page** (`keepVideoPlayed`, `keepPlaying` of `@xmoq/media`): its states go `connecting`, `moved https://127.0.0.1:<port>/moved`, `connecting`, `playing 1`, without the pause of 500 ms; the peer prints `SESSION_ENDED session=0 ... code: 0,` and then `SESSION session=1 path=/moved`.
- **The terminal viewer**: prints `MOVED https://127.0.0.1:<port>/moved`; the peer the same two lines as for the page.
- The byte rows of moqtail npm 0.13.0 (`fixtures/moqt-18-from-libraries/goaway-*`) decode to the values of their README in both implementations and are encoded to the same bytes; the rules of §10.4 (a second GOAWAY, no Request ID, a URI at a server, the wrong parity, a URI over 8,192 bytes) are tests of both.

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the working copy; `mise run interop -- --base-port 45300`, and `bun interop/quiet-peer.ts --case goaway --client <client> --port 45390` one by one before it.

## Result

`mise run interop -- --base-port 45300`: 171 of 171 pairings ended as expected in 286.0 s, the three below among them; each of the three had passed alone before, once.

| Client | What it said | The peer | Time of the run |
|---|---|---|---|
| own-rust | `GOAWAY uri=https://127.0.0.1:45385/moved timeout_ms=5000 request_id=0`, left with 0 | `SESSION_ENDED session=0 ... code: 0,`, no second session | 0.1 s |
| The page | `connecting, moved https://127.0.0.1:45384/moved, connecting, playing 1` | `SESSION_ENDED session=0 ... code: 0,`, then `SESSION session=1 path=/moved` | 0.6 s |
| The terminal viewer | `MOVED https://127.0.0.1:45386/moved` | the same two lines | 1.1 s |

- As expected. **How long the page took from the GOAWAY to the new session was not measured**, so that it skips its pause of 500 ms is read from the code and not seen.
- `mise run check` passed, with the tests of the byte rows and of the rules of §10.4 in both implementations.
- **Not run**: a GOAWAY on a request stream (both end the session with `INTERNAL_ERROR`, a known gap); a GOAWAY that names another server, with another certificate (the page and the terminal viewer connect with the certificate hash they have); the relay taking a GOAWAY from a client in a run (a test of `xmoq-relay-core` has it); the relay sending GOAWAY, which it does not.

## After the review of the change (2026-10-24)

The review found that the peer answered SUBSCRIBE after its GOAWAY where §10.4 has it refused with `GOING_AWAY`, that a refusal read before the GOAWAY made the clients connect again to the URL they had, that the terminal viewer dropped a GOAWAY that came while it waited for the catalog and closed with `NO_ERROR` for a GOAWAY that broke a rule, and that `keepPublishing` did not look at GOAWAY. Mended in the same change: the peer refuses every request of its first session with `GOING_AWAY` and sends the GOAWAY after the first refusal, so that the client reads the refusal first; the clients wait for the GOAWAY behind such a refusal; `keepPublishing` moves as `keepPlaying` does. A fourth run has the page as a publisher.

Run one by one, once each (`bun interop/quiet-peer.ts --case goaway|goaway-publisher --client <client> --port 45390`):

| Client | What it said |
|---|---|
| own-rust | `GOAWAY uri=https://127.0.0.1:45390/moved timeout_ms=5000 request_id=0`, left with 0 |
| The page, playing | `connecting, moved https://127.0.0.1:45390/moved, connecting, playing 1` |
| The page, publishing | `connecting, moved https://127.0.0.1:45390/moved, connecting, publishing 1`; the peer read its PUBLISH_NAMESPACE on session 1 |
| The terminal viewer | `GOAWAY timeout_ms=5000 after GOING_AWAY`, then `MOVED https://127.0.0.1:45390/moved` (a run with `--verbose`) |

- The peer printed `SESSION_ENDED session=0 ... code: 0,` in each, and `SESSION session=1 path=/moved` for the three that move.
- Whether the page and own-rust read the refusal or the GOAWAY first was not printed; the terminal viewer's line says it read the refusal first.
- **Not run**: a GOAWAY that breaks a rule against the terminal viewer (the code it closes with is the one the Rust client's tests have; the viewer's own path was not run).

- `mise run interop -- --base-port 45300` after these mends: 172 of 172 pairings ended as expected in 285.9 s, the four runs above among them.
