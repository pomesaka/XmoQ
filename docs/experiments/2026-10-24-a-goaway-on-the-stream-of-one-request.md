# A GOAWAY on the stream of one request, and how soon the next session stands

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 2, a session that goes on; the known gap left by [a GOAWAY the clients take](2026-10-24-a-goaway-the-clients-take.md) (a GOAWAY on a request stream ended the session with `INTERNAL_ERROR`), and the time from a GOAWAY to the next session, which that record did not measure.

## Objective

Draft 18 §10.4: "A GOAWAY MAY also be sent on a request stream to initiate migration of that individual request. Upon receiving a GOAWAY on a request stream, the endpoint SHOULD re-issue that specific request on a session at the specified URI (or the current session if no URI is provided), and close the old request stream". Both sessions now take one by the rules of §10.4 (one on a stream, no Request ID, no URI from a client) and tell it for that request; the viewers move their session where it is the video's (the reasons are in the row "GOAWAY" of the implementation notes).

The peer is `xmoq-quiet-peer --quiet goaway-request`: on its first session it answers the first SUBSCRIBE with `SUBSCRIBE_OK` and then sends GOAWAY on its stream, naming `https://127.0.0.1:<port>/moved` of itself, with a Timeout of 5,000 ms and no Request ID; it answers SUBSCRIBE and PUBLISH_NAMESPACE on every session. The time is measured by the driver: from the peer's line `REQUEST_GOAWAY_SENT` (or `GOAWAY_SENT`, for the control stream with `--quiet goaway`) to its line `SESSION session=1 path=/moved`, as the driver reads them.

## Expected, written before the full interop run

**Each run was made alone once before this was written**, while the runs were made, and the pause of one second the terminal viewer took before it connected again (1,000 and 1,005 ms) was found that way and taken out; what follows is what the runs are held to.

- own-rust: prints `REQUEST_GOAWAY uri=https://127.0.0.1:<port>/moved timeout_ms=5000` and leaves with 0; the peer prints `SESSION_ENDED session=0 ... code: 0,`.
- The page (`keepVideoPlayed`: `playVideoTrack` and `keepPlaying` with `movedBy`): its states go `connecting, playing 1, moved https://127.0.0.1:<port>/moved, connecting, playing 2`; the peer prints the same end and `SESSION session=1 path=/moved`.
- The terminal viewer: `MOVED https://127.0.0.1:<port>/moved`, and the same two lines of the peer.
- From the GOAWAY to the next session, on one machine: under 50 ms for the page and the terminal viewer, with a GOAWAY on a request stream and on the control stream alike (no pause of `keepPlaying`, 500 ms in the interop run, nor of the terminal viewer, 1 s).

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core 1.64.0, headless; the working copy; `bun interop/quiet-peer.ts --case goaway|goaway-request --client <client> --port 45390` one by one, and `mise run interop -- --base-port 45300`.

## Result

Alone, once each, after the pause of the terminal viewer was taken out:

| Client | GOAWAY on | What it said | From the GOAWAY to the next session |
|---|---|---|---|
| own-rust | the SUBSCRIBE's stream | `REQUEST_GOAWAY uri=https://127.0.0.1:45390/moved timeout_ms=5000`, left with 0 | (does not connect again) |
| The page | the SUBSCRIBE's stream | `connecting, playing 1, moved https://127.0.0.1:45390/moved, connecting, playing 2` | 9 ms |
| The terminal viewer | the video's stream | `MOVED https://127.0.0.1:45390/moved` | 3 ms |
| The page | the control stream | `connecting, moved https://127.0.0.1:45390/moved, connecting, playing 1` | 6 ms |
| The terminal viewer | the control stream | `MOVED https://127.0.0.1:45390/moved` | 4 ms |

- One run each, on loopback: the numbers say that neither waits for a pause, and nothing of how long it takes over a network.
- **Not run**: a GOAWAY on the sound's stream alone (cancelled and asked for again, by the code); one on the stream of a PUBLISH, a PUBLISH_NAMESPACE or a FETCH (taken and not acted on); one on a request of the peer at the relay.
- `mise run interop -- --base-port 45300`: 175 of 175 pairings ended as expected in 285.9 s. From the GOAWAY to the next session in that run: the page 10 ms (request stream) and 9 ms (control stream), the terminal viewer 3 ms and 2 ms.
