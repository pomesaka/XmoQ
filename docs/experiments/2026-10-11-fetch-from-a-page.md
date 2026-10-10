# A FETCH from a page: the session state of `@xmoq/moqt`, in Chromium against both relays

- Date: 2026-10-11
- Commit SHA: the commit that adds `MoqtConnection.fetch` together with this record. The runs were made on its working copy, on `main` at `a0fdfdeb` with one change on top that touches `interop/` only
- Roadmap item: Phase 1, FETCH and Joining FETCH in the TypeScript session. It closes no item: nothing serves a FETCH on the TypeScript side, and the own relay did not serve one when the runs were made

## Objective

`@xmoq/moqt` could encode and decode a FETCH, a FETCH_OK and a fetch stream, and did nothing with them. It got the state of a FETCH it sends (the rows on FETCH in the implementation notes, which the Rust session has as well) and a way to send one from a page. See a page send a Standalone and a Joining FETCH to another implementation and read what comes back, and see what the own relay answers.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relays | `moqtail` relay 0.15.0 (`moqtail/0.15.0+dd8623d`), and the own relay as built from the commit above, which refuses every FETCH |
| Publishers | the own Rust client, the own page, and the `moqtail-rust` client of `interop/peers/` |

## Commands

```sh
cd interop
bun fetch.ts --relay moqtail --fetch standalone --publisher own-rust --verbose
# --relay moqtail|own, --fetch standalone|relative-joining|absent-track, --publisher own-rust|own-browser|moqtail-rust
```

A publisher publishes (`xmoq`, `phase0`) / `object` and sends one Object, {0, 0}. The page subscribes and has it. Then the page sends the FETCH with `connection.fetch()`, reads the fetch stream through `decodeFetchBody`, and reports the answer, the items, how the stream ended and how the FETCH ended. The FETCH is cancelled by its `signal` after 5 seconds.

**What was expected was written into `fetch.ts` before the first run**, for the `moqtail` relay from [the record of its FETCH](2026-10-11-moqtail-relay-fetch.md), for the own relay from the implementation notes.

## Observed results

Twelve runs, each relay with each case and with an own Rust client and an own page as the publisher: all as expected, after one expectation was corrected (below).

| Relay | FETCH | Expected | Seen, with both publishers |
|---|---|---|---|
| `moqtail` | Standalone, {0, 0} to {1, 0} | FETCH_OK; the fetch stream has the Object {0, 0} with the payload that was published; the stream ends with a FIN; the FETCH ends as served. End Location: see below | as expected. On the wire: `05 02 1c 00 00 00 1b` and the payload, then `18 00 04 00 00 01 00` on the request stream. Which of the two the page read first differed from run to run |
| `moqtail` | Relative Joining, Joining Start 0, for the page's SUBSCRIBE | FETCH_OK, the Object {0, 0}, FIN, served. The End Location is not compared | as expected; End Location {0, 1} |
| `moqtail` | Standalone for a track nobody published | REQUEST_ERROR `DOES_NOT_EXIST` (0x10); the FETCH ends as rejected, without Objects | as expected: "Track does not exist" |
| own | all three | REQUEST_ERROR `NOT_SUPPORTED` (0x3), rejected | as expected |

- **One expectation was wrong, and was corrected for a reason, not to the result alone.** The End Location of the FETCH_OK for the Standalone FETCH was written as {1, 0}, the one asked for, from runs 7 and 9 of the moqtail record. It came as {0, 1}. In those runs the publisher was moqtail's test client, which answers a FETCH itself. The relay's log of this run shows what it does: it passes the FETCH on to the publisher (`control message sending ... Fetch`), the own publisher refuses it with `NOT_SUPPORTED` (the Rust client: `RequestError (6 bytes): 05 00 03 03 00 00`; the page: 48 bytes, with a reason), and the relay then answers from what it holds, with the Location after the last Object it has. So the End Location depends on who the publisher is, which the moqtail record names as not known.
- **With the `moqtail-rust` client of `interop/peers/` as the publisher, the Standalone FETCH got no answer at all**: the relay passed it on, that client does not answer a FETCH, and nothing came back in 5 seconds. The page's `signal` cancelled the FETCH, which then ended as `cancelled`, its `response` rejected with `RequestAbortedError`. One run. This is why `fetch.ts` has an own publisher by default.
- **Both own clients refused the relay's FETCH as the notes have it**, with bytes another implementation sent them: the own Rust client and the page (`@xmoq/moqt` decodes a FETCH of the peer whole, then refuses it).
- The bytes of a fetch stream that another implementation wrote were decoded in a browser by `@xmoq/moqt`: the header `05 02`, Serialization Flags 0x1C, Group 0, Object 0, priority 0, a payload of 27 bytes.

What the tests hold, and what breaks them (`packages/moqt/test/core/session-fetch.test.ts`, `test/webtransport/fetch.test.ts`; each change was temporary and taken back):

| Temporary change | Tests that fail |
|---|---|
| A fetch stream for a request that is over closes the session | 9 |
| The fetch stream is not stopped when its FETCH ends | 9 |
| `body` holds 100 reads and not 4 | 1: a fetch stream nobody reads is read no further |
| A Standalone FETCH of the peer is never refused with `INVALID_RANGE` | 2 |
| A FETCH is over when its stream ended, without its answer | 6 |
| `cancel()` closes `body` and does not fail it | 2, one of them: a FETCH cancelled inside an Object must not close the session |

`mise run check` and `mise run interop` passed: 624 tests of the TypeScript packages, 74 of 74 pairings.

## Limitations and blockers

No blocker.

- **One Object, one Group.** A fetch stream of several Groups, one in Descending order, one with an End of Range indicator and one with a gap were not obtained from a relay here; the tests read the captures and the hand-written streams of `fixtures/`.
- The Joining FETCH was sent when the subscription was Established. One for a SUBSCRIBE without an answer yet was not run against a relay.
- Where the `moqtail` relay differs from draft 18 (five places in its record) was not looked at again, but for the End Location above.
- The own relay refuses every FETCH, so a FETCH that it serves, a fetch stream that it resets, and a FETCH it cancels were run against the played transport only.
- **A FETCH without an answer stays open** until the caller cancels it; the run with `moqtail-rust` as the publisher is that case. Nothing in `@xmoq/moqt` ends it after a time.
- A fetch stream whose FETCH_HEADER does not come is not stopped after a time (implementation notes, known gaps: a stream that never says what it is).
- `fetch.ts` is not in the matrix. Chromium only, loopback only.

## Open questions raised

- Whether the pairings of the matrix get a FETCH, and against which relay: the own one will serve a Standalone FETCH with a change that was in review when this was written, and the expectations for the own relay here are for the one that refuses.
