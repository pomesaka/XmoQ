# A page that only announced a namespace answers the relay's SUBSCRIBE

- Date: 2026-10-11
- Commit SHA: the commit that adds `MoqtConnection.serve` to `@xmoq/moqt` together with this record. The runs were made on its working copy, on `main` at `70f2dc20` with the change to one interop run that was a pull request then (`7367beb4`) below it
- Roadmap item: groundwork for Phase 1 (a browser publisher that a relay can ask for a track). It closes no item

## Objective

`@xmoq/moqt` refused every request the peer opens with `NOT_SUPPORTED`. A page that had announced a namespace and sent no PUBLISH could therefore not be a source: a relay asks such a publisher for a track with a SUBSCRIBE of its own, and the page refused it. This change lets the application answer that SUBSCRIBE. The runs are to show that

1. a page that only announced is asked, accepts, and its Object reaches a subscriber, through the `moqtail` relay and through the own one;
2. the session answers the same SUBSCRIBE in the same state as the Rust session does, where the session answers by itself.

Not an objective here: a FETCH of the peer, which is still refused.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relays | `moqtail` relay 0.15.0; the own relay of `main` at `70f2dc20` |
| Other clients | `moqtail-rust` (the `moqtail` crate 0.15.0), the own Rust client |
| Draft | draft-ietf-moq-transport-18 |
| Runtime of the tests | Bun 1.3.12 |

## Commands

```sh
mise run check
mise run interop
cd interop
bun matrix.ts --only "only announces the namespace"   # the page as the one publisher, both relays
bun matrix.ts --only "only announce the namespace"    # the page and an own-rust publisher, the own relay
```

## Observed results

### In Chromium

| Run | Through | Result |
|---|---|---|
| The page announces and sends no PUBLISH; `moqtail-rust` subscribes | `moqtail` relay | The relay sent the page a SUBSCRIBE (Request ID 1, FORWARD 1, the Largest Object filter, Ascending). The page accepted with SUBSCRIBE_OK, Track Alias 0, and the subscriber got the Object the page sent on that subscription |
| The same, `own-rust` subscribes | `moqtail` relay | The same |
| The same two | own relay | The same; the own relay's SUBSCRIBE has no parameters |
| The page and an `own-rust` publisher announce; `own-rust` and `own-browser` subscribe | own relay | The relay asked both once each, in the order they announced. The subscribers got Group 0 from the page and Group 1 from the other. The page ended its subscription with PUBLISH_DONE, and the relay finished its direction of the request stream (`publish-done-peer-ended`, `fin`) |

- **Both relays ask a publisher that only announced.** For the `moqtail` relay this was not known before these runs.
- `mise run interop`: 87 of 87 pairings, the 82 from before and these 5.
- The run in which a page announces and refuses (it does not call `serve`) passes as before: the SUBSCRIBE gets `NOT_SUPPORTED`.

### In the tests, without a browser

`mise run check` passed, with 843 tests of `@xmoq/moqt` where there were 748.

- `fixtures/peer-subscribe/opened.tsv`, 10 rows: what becomes of a SUBSCRIBE of the peer by what went before it (handed to the application, refused with `DUPLICATE_SUBSCRIPTION`, the session closed with `INVALID_REQUEST_ID`). The TypeScript session gives the outcome of every row. Its outcomes were written from reading `crates/moqt/src/session/peer.rs`; a review then ran the ten rows through the Rust session, in a test that is not kept, and all ten gave the outcome of the table.
- 14 temporary changes to the implementation, one at a time, each taken back: the check for a Track that is published already, the limit, the search for a free Track Alias, the request that goes on after its subscriber's FIN, the REQUEST_UPDATE before the answer, the Joining FETCH that names a SUBSCRIBE of the peer, the bytes that follow the SUBSCRIBE in one read, the watch for a STOP_SENDING on the stream, the code of the peer's reset, the abort of `signal` on a reset and at the end of the session, the second answer, and the report of a handler's error. Each made between 1 and 6 tests fail. A review then broke the implementation in 28 ways and found 7 that no test noticed, all of them about what is kept after a stream has ended or about an order of two events; tests were added, and each of the 7 now makes between 1 and 8 fail. To see what is kept, the session and the I/O layer each say how many streams they keep something of (`heldStreams`), which nothing but the tests reads.

### Two faults a review found, by running the same events through both sessions

- **A connection that never called `serve` closed the session, with `INTERNAL_ERROR`, when a SUBSCRIBE and a REQUEST_UPDATE of the peer arrived in one read**, which §10.9 allows. The session had refused the update and ended the request; the connection then refused the SUBSCRIBE for want of a handler, straight at the session, and took the session's "no such request" for a failure of its own. The refusal now goes the way an application's answer goes. Four tests, for no handler and three kinds of handler; without the fix the first fails.
- **A Request ID of the peer that an open SUBSCRIBE has could be taken a second time**, once more than 1,024 later IDs above a gap had pushed it out of what is remembered: the second request replaced the first, which could then not be ended. The open requests are asked now, as in the Rust session, and the session closes with `INVALID_REQUEST_ID`.

### Seen on the way

- One test of the Rust workspace failed once in `mise run check` and passed in the run after it and in 6 runs alone: `contract::web_transport_quinn::a_fetch_stream_given_up_while_a_write_waits_is_reset_with_that_code` ("the stream arrives: Elapsed"). This change touches no Rust.

## Limitations and blockers

- Chromium only, loopback only, one Object a publisher.
- What Chromium reports to the writer of a stream the peer opened, when the peer stops it, was measured after these runs: [record](2026-10-11-write-abort-in-chromium.md), the section on a stream the server opened. The reading the library makes holds there. A relay that cancels resets its direction and stops the page's, and the page then gets the code of the cancellation or none, by which of the two it sees first. The `moqtail` relay 0.15.0 cancels in that way too, by its source (`subscribe_handler.rs`: "Reset the send half and stop the recv half with the same CANCELLED code"); it was not run.
- In the browser, of what a subscriber can do to a subscription the page accepted, only the end in order was run (PUBLISH_DONE, then the relay's FIN). A subscriber's STOP_SENDING, its reset, its REQUEST_UPDATE and a request that goes before it is answered were run against the fake transport alone.
- LARGEST_OBJECT in the page's SUBSCRIBE_OK was not sent in the browser runs: the page had sent nothing when it was asked. The bytes of a SUBSCRIBE_OK with it are compared with a capture in the tests.
- The limit on open requests of the peer (64) was reached in the tests alone.
- A FETCH of the peer is refused. A Joining FETCH that names a SUBSCRIBE of the peer gets `NOT_SUPPORTED`; one that names a Request ID that has not arrived yet gets `INVALID_JOINING_REQUEST_ID` at once.

## Open questions raised

- A subscription the page accepted keeps its request stream open and one of the 64 places for the peer's bidirectional streams. Nothing ends a subscription whose subscriber neither ends it nor sends anything.
- Whether the library should keep Objects it sent, to answer a FETCH with, waits for the decision on the relay's cache (open question 11).
