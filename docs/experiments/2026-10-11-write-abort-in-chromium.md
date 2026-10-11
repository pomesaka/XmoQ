# What a writer fails with in Chromium: a stopped stream, an ended session, a lost connection

- Date: 2026-10-11
- Commit SHA: the commit that adds `spikes/wt-write-abort/` together with this record
- Roadmap item: groundwork for Phase 1 (the publisher side of `@xmoq/moqt`). It closes no item

## Objective

`@xmoq/moqt` learns that the peer stopped a stream it writes from `writer.closed`, and takes only a `WebTransportError` with `source` "stream" for it, so that the end of a session is not taken for a stop. Chromium sometimes rejects with a `DOMException` named NetworkError instead ([the run that failed for it](2026-10-10-two-publishers-in-the-interop-matrix.md)); the library then goes on counting the stream as open, and the publisher cannot send PUBLISH_DONE. Before that is mended: can the NetworkError be told from the end of a session by what it is, or only by waiting for the session?

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless, raw `WebTransport`, no library |
| Server | `spikes/wt-write-abort`, on `web-transport-quinn` 0.13.2, the library of the own relay. It reads nothing from a stream. Idle timeout 3 seconds |

## Commands

```sh
(cd spikes/wt-write-abort && cargo build --locked)
bun spikes/wt-write-abort/run.ts --rounds 60 --out spikes/wt-write-abort/out/rounds-60.txt
```

A sample: the page opens a session and one stream, writes one byte, and then one thing happens. Recorded: what `writer.closed` is rejected with; what a write that was under way is rejected with; what a write made 300 ms later is rejected with; what `closed` of the session did, and how long after `writer.closed`.

| What happens | |
|---|---|
| stop | the server sends STOP_SENDING, code 7, 100 ms after the stream came |
| stop at once | the same without the 100 ms, as a relay does that decides on the first bytes |
| the peer closes | the server closes the session, code 9, after 400 ms |
| the page closes | `close({ closeCode: 4 })` after 200 ms |
| the connection is lost | the server is killed; the page notices by the idle timeout |

Each for a unidirectional and a bidirectional stream, and for a writer that is idle, one with a write of 8 MiB waiting for the peer's window, and one that writes 100 bytes at a time, each write awaited. 60 samples of each: 1800 in all, eight at a time (ten of each for the lost connection, one kill a batch). One run, 1 minute 45 seconds.

## Observed results

`out/rounds-60.txt` has every row. In short:

| What happens | Samples | `writer.closed` and every write of the stream are rejected with | The session |
|---|---|---|---|
| The peer closes, the page closes | 720 | **`WebTransportError`, `source` "session", "The session is closed."**, in all 720 | `closed` resolves with the code and reason |
| The connection is lost | 360 | **`WebTransportError`, `source` "session", "Connection lost."**, in all 360 | `closed` rejects with the same |
| Stop, stop at once | 720 | `WebTransportError`, `source` "stream", `streamErrorCode` 7, "Received STOP_SENDING." in 664. **`DOMException`, name NetworkError, "The stream was aborted by the remote server", no code, in 56** | open 300 ms later, in all 720 |

Where the NetworkError came:

| | Idle | A write waiting | Writing |
|---|---|---|---|
| Stop, unidirectional | 0 of 60 | 0 of 60 | 1 of 60 |
| Stop, bidirectional | 0 of 60 | 0 of 60 | 1 of 60 |
| Stop at once, unidirectional | 0 of 60 | 18 of 60 | 0 of 60 |
| Stop at once, bidirectional | 0 of 60 | 36 of 60 | 0 of 60 |

- **In 1080 samples the end of a session never gave a NetworkError, and in 720 a stop never gave an error of the session.** By what was seen, a `DOMException` named NetworkError on a writer is a stream that the peer stopped, and the session is open.
- **The NetworkError needs a write under way**: an idle writer got the `WebTransportError` with the code in all 240 samples. It is what `OutgoingStream::HandlePipeClosed` makes when the data pipe of the stream closes before the STOP_SENDING is reported (Chromium's `outgoing_stream.cc`, read for the earlier record).
- **Once a stream has the NetworkError, it has nothing else.** `writer.closed`, the write that was under way and the write 300 ms later were rejected with the same error in all 56. The code of the STOP_SENDING does not reach the page then.
- **When a session ends, the writer and the session settle together**: at most 0.1 ms apart in all 1080, the writer first in 1072 and the session first in 8. A page cannot count on either order.

What a wrong reading would cost, should a NetworkError come at the end of a session after all, by the paths of `@xmoq/moqt` as they are: for a subgroup stream, a reset is written to a session that has ended, and nothing else. For the request stream of a PUBLISH, the subscription would be reported as ended by the peer (`peer-reset`) a moment before the session is reported closed. For the control stream, a stop is a PROTOCOL_VIOLATION of the peer, which is why that stream has to be left out of any such reading.

### A stream the server opened

Added later on 2026-10-11, for `MoqtConnection.serve`: a subscription the page accepted is on a bidirectional stream the relay opened, and a relay that cancels it resets its direction of that stream and sends STOP_SENDING for the page's. The runs above were on streams the page opened. Same server, with paths that open a bidirectional stream, write one byte on it, and then act; same browser. `bun spikes/wt-write-abort/run.ts --rounds 40 --opened-by peer`, 840 samples (`out/peer-opened-40.txt`), and the two runs of one scenario named below.

| What the server does to its stream | `writer.closed` of the page, and a write under way | Samples |
|---|---|---|
| STOP_SENDING after 100 ms | `WebTransportError`, `source` "stream", code 7 | 120 of 120 |
| Reset of its direction and STOP_SENDING, after 100 ms | `DOMException` NetworkError, "The stream was aborted by the remote server"; no code | 120 of 120 |
| The same, at once | The same NetworkError in 107. **In 13 the stream was not handed to the page at all** (nothing from `incomingBidirectionalStreams` within a second) | 120 |
| STOP_SENDING at once | Code 7 in 94, NetworkError in 19, **and in 7 nothing**: the first write resolved, `writer.closed` stayed pending, and the connection ended 2 seconds later | 120 |
| Closes the session; the page closes it; the server is killed | `WebTransportError`, `source` "session" | 360 of 360 |

- **The reading of the library holds on a stream the peer opened**: a NetworkError is a stream the peer stopped, with a code the page does not get, and the end of a session is never one (0 of 360 here, 0 of 1080 above).
- **A relay's way of cancelling, reset and STOP_SENDING, gives the page no code on the writer**: every one of the 227 writers got the NetworkError. The code of the reset reaches the page on the reading side, which is not looked at here.
- **A STOP_SENDING that arrives before the page has taken the stream can go unreported.** Run again alone (`--only stop-now`, `out/peer-opened-stop-now-40.txt`): 13 of 120. The server saw each of those connections closed by Chromium for "No recent network activity after 2000 ms", with two active streams; the writes the page made meanwhile resolved or stayed pending, and nothing went out. On streams the page opened, the same scenario reported every stop (240 of 240, `out/own-opened-stop-now-40.txt`): the page has the writer before the server can stop it.
  - The server of the spike sends no keep-alive. A relay that does keeps such a session open, and the page then goes on believing in a stream that was stopped.
  - What `@xmoq/moqt` reads on such a stream is still read: a peer that also resets its direction, as the own relay does when it cancels, is noticed there.

## Limitations and blockers

No blocker.

- **"Never in 1080" is not "cannot".** The reading that a NetworkError is a stopped stream rests on these samples and on the one function in Chromium's source that makes it; the code paths of the browser for a session that ends were not read.
- One browser, one version, one machine, loopback, one run. A loaded machine and a real network were not run.
- A stream that the peer resets or stops while the session is closing was not run.
- **A NetworkError is not only a stopped stream.** Chromium rejects `createUnidirectionalStream()` and `createBidirectionalStream()` with a `DOMException` of the same name when a stream cannot be made (`web_transport.cc`, read by a reviewer of the change that followed this record). That is a stream that was never there, not a writer that failed, and it was not run here. `@xmoq/moqt` ends the session for it and looks at that before it reads an error as a stop.
- Added later the same day, 20 samples of each, `spikes/wt-write-abort/own-reset.ts`: when the page itself aborts the writer with a `WebTransportError` that has a code, as `@xmoq/moqt` does to reset a stream, `writer.closed` and a later write were rejected with that very error (`source` "stream", the page's own code) in 80 of 80, and so was the write of 8 MiB that was under way in the 40 that had one. **So a writer's error with a code is not always the peer's**: whoever reads it has to know whether it reset the stream itself.
- The page was in the foreground.
- The server's library sends STOP_SENDING and keeps its own side as it is; a peer that also resets its side of a bidirectional stream was not run here (the own relay does, in the interop runs).

## Open questions raised

- Whether `@xmoq/moqt` takes a NetworkError on a writer for a stop with an unknown code, by its type alone. The numbers here are for that decision.
