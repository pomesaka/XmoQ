# A request that waits for a stream

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 0b, the transport layer in two implementations (limits on what a peer decides)

## Objective

A request goes on a bidirectional stream of its own, and a stream can be opened only when the peer allows one more. Two things were open: in `@xmoq/moqt` a request that waited for its stream held back everything behind it, the answers of the peer that had been read among them; and nothing bounded the wait, in either implementation.

## What the draft says

Nothing of how long. §13.5: "Endpoints MAY impose a MAX STREAM count limit which would restrict the number of concurrent streams which an application could have in flight." The row "A request that waits for a stream" of [the implementation notes](../moqt-implementation-notes.md) has what the implementations do.

## What was built

- `@xmoq/moqt`: a stream that is being opened is no longer waited for by the one line in which outputs are performed. What is to happen on it is kept for that stream, in order, and everything else goes on. A request that has no stream after 10 seconds (`openStreamTimeoutMs`) fails with `RequestAbortedError` of `not-sent-in-time`; nothing was sent, and the session stays open. A stream that comes after that is reset and stopped with CANCELLED, with nothing written on it.
- The Rust client (`--open-wait-ms`) and the terminal viewer (`--open-wait-ms`): the same 10 seconds. The Rust client prints `NOT_SENT request=... after_ms=...`, and the mode fails. They had no line of outputs to hold back.
- `xmoq-quiet-peer --quiet streams`: a peer that sends its SETUP and allows the client no bidirectional stream.

## Environment

One machine, macOS, loopback; Chromium 156.0.8078.4 of Playwright, headless. No relay: the clients connect to `xmoq-quiet-peer`. The clients' wait at 1.5 seconds.

## Commands

```sh
cd interop
bun quiet-peer.ts --case no-stream --client own-browser|own-rust|xmoq-watch
```

## Observed results

**Chromium does not wait for a stream.** Where the peer allows no further bidirectional stream, `createBidirectionalStream()` rejected at once, with "Failed to create bidirectional stream." The API describes a call that resolves when a stream can be made; the fake transport of the tests did that, and Chromium does not. The first run of the page, before this was known:

| | The page | The peer |
|---|---|---|
| As it was written first (a creation that fails ends the session) | `subscribe()` failed with `SessionClosedError: the session failed: Failed to create bidirectional stream.` | `SESSION_ENDED session=0 ClosedByPeer { code: 1, reason: "Failed to create bidirectional stream." }` |

So a page whose peer had no room for one more request for a moment lost its whole session, with INTERNAL_ERROR. **That was so before this change as well**, and no run had reached it. The library now asks for the stream again every 50 ms (provisional) until the time for it is over, which is the wait the API describes.

| Client | What it said | What the peer printed |
|---|---|---|
| own-browser | `subscribe()` failed with `RequestAbortedError: the request ended without a response: not-sent-in-time`; its session was open afterwards | no `REQUEST` line |
| own-rust | `NOT_SENT request=subscribe after_ms=1500`, and it left with a failure | no `REQUEST` line; `SESSION_ENDED session=0 ClosedByPeer { code: 0, reason: "" }` |
| xmoq-watch | `the relay allowed no stream for object within 1500 ms` | the same |

That an answer is handed on while another request waits for its stream is in the tests of `@xmoq/moqt` alone, with the fake transport: a SUBSCRIBE_OK resolves its `subscribe()` while a request behind it has no stream, and a stream of the peer is stopped in time (the first-bytes rule) while one waits.

## Not run

- **A unidirectional stream the peer allows no room for.** A subgroup stream is opened in the same way, and a creation that fails there still ends the session: the fake transport's `failUni` has had Chromium's "Failed to create send stream" since before, and that is what a publishing page would meet at a relay that allows it no further stream. No time was put on it, and nothing asks again.
- The 10 seconds themselves against a real peer; a peer that allows a stream after some seconds (in the tests alone, on a clock moved by hand: refused for 3 seconds, then sent).
- Firefox and Safari, which may wait where Chromium refuses.

## Next action

Whether a subgroup stream that cannot be opened should wait and ask again as a request does, in place of ending the session.
