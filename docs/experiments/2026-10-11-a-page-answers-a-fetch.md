# A page answers the FETCH a relay sends it

- Date: 2026-10-11
- Commit SHA: the commit that adds `serve({ fetch })` to `@xmoq/moqt` together with this record. The runs were made on its working copy, on `main` at `59d76ab8` with the pull request for a page that answers a SUBSCRIBE below it
- Roadmap item: groundwork for Phase 1 (a viewer gets the catalog of a broadcast with a Joining FETCH). It closes no item

## Objective

A viewer that joins a broadcast subscribes to a track and sends a Joining FETCH for what came before. The own relay has no cache, and asks a publisher for the range with a Standalone FETCH. `@xmoq/moqt` refused every FETCH of the peer, so a page could not be that publisher. This change lets the application answer one. The runs are to show that

1. a page answers the FETCH the own relay sends it, for a Standalone FETCH of another page and for a Joining FETCH, and the other page gets the Objects;
2. the same holds against a relay that is not ours;
3. the session answers a FETCH of the peer, and a REQUEST_UPDATE for it, as the Rust session does.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4 (23E214), arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relays | the own relay of `main` at `59d76ab8`; `moqtail` relay 0.15.0 |
| Other clients | the own Rust client, as the first subscriber |
| Draft | draft-ietf-moq-transport-18 |
| Runtime of the tests | Bun 1.3.12 for the runs of this record; after the review below, Bun 1.3.14, to which the repository moved meanwhile |

## Commands

```sh
mise run check
mise run interop
cd interop
bun matrix.ts --only "own page"                                   # the seven runs
bun fetch.ts --fetch page-unsent --relay moqtail --report-only    # what a relay does, compared with nothing
bun fixtures/peer-fetch/generate.ts                                # from the repository root: the two tables
```

## Observed results

### In Chromium, the page as the publisher

The publishing page announces the namespace and sends no PUBLISH. The own Rust client subscribes, so the relay asks the page for the track; the page accepts and sends Object 0 of Groups 0, 1 and 2. Then the page that fetches subscribes, the publisher sends Group 3, and the FETCH is sent. The publishing page keeps what it sent and answers from that: the library keeps nothing.

**What is expected of the own relay was written before the runs**, from the implementation notes; `EXPECTED` in `interop/fetch.ts` has each with its source. All six came out as written, in the first run.

| Run | Expected, and seen |
|---|---|
| Standalone, {0, 0} to {2, 1} | FETCH_OK with End Location {2, 1}; Object 0 of Groups 0, 1, 2 with the publisher's payload; a FIN |
| **The page that fetches subscribes and sends a Relative Joining FETCH, one Group back** | The relay asks the publisher for {1, 0} to {2, 1}: the Joining Location is {2, 0}, the LARGEST_OBJECT of the SUBSCRIBE_OK it sent the page. FETCH_OK with End Location {2, 1}; Object 0 of Groups 1 and 2 |
| Standalone, {0, 1} to the end of Group 0, where the publisher has nothing | FETCH_OK with End Location {0, 0}; the publisher's End of Range indicator, `NonExistent` at {0, 2^64 - 1} |
| Standalone, {100, 0} to {102, 1}: Objects the publisher has and never sent | FETCH_OK with End Location {102, 1}; the three Objects |
| The publisher accepts and writes nothing; the page gives up after 1 second | FETCH_OK with End Location {2, 1}, then the FETCH ends as cancelled. The relay prints `RECV session=0 own=K FETCH_OK` and then `CANCEL_OWN_REQUEST session=0 own=K code=0x1`. The publishing page: `{"type":"peer-cancelled","code":1}` |
| The publisher refuses with `DOES_NOT_EXIST` | REQUEST_ERROR `0x10` |

- The seven runs of the matrix (the six, and the fourth through the `moqtail` relay), two matrices at once, 20 rounds each: 40 of 40 rounds, 280 runs. `mise run interop`: 94 of 94 pairings, the 87 from before and these 7. `mise run check`: 930 tests of `@xmoq/moqt`.
- Temporary changes, each taken back: with Joining Start 0 the Joining run fails with Object {2, 0} alone; with a publisher that writes no End of Range indicator the third fails with an empty stream; with a publisher that serves where it is to stall the fifth fails with the Objects; with a publisher that writes past the end of the range the first fails with {3, 0} as a fourth Object.
- **Once the relay has a cache, a FETCH for what the relay holds does not reach the publisher** (decided for Phase 1: the newest Group). The Joining run then gets its answer from the relay, and the runs that are to reach the publisher have to ask for what the relay does not hold, as the fourth does.

### Against the `moqtail` relay

Run without an expectation first (`--report-only`), then written into the matrix as what the publisher's side has to show.

| Run | What the `moqtail` relay did |
|---|---|
| {0, 0} to {2, 1}, and the Joining FETCH, and a publisher that refuses | **It did not ask the publisher.** It answered from what it had seen pass: FETCH_OK and the Objects, also where the publisher would have refused. For the Joining FETCH: End Location {3, 1} and Object 0 of Groups 2 and 3 (its Joining Location was {3, 0}) |
| {100, 0} to {102, 1}, which it had not seen | **It asked the publisher**: a Standalone FETCH with Request ID 3, End Location {102, 0}, the whole Group, where the page had asked for {102, 1}, and GROUP_ORDER Ascending. The publishing page answered FETCH_OK with End Location {102, 1} and wrote the three Objects; the relay printed `Upstream FETCH 3 accepted, end location Location { group: 102, object: 1 }` and passed that FETCH_OK on |
| The same, on the way to the page that fetched | **The relay sent that page a fetch stream that begins `05 03`: the FETCH_HEADER with Request ID 3, the relay's own, where the page's FETCH has Request ID 2.** `@xmoq/moqt` closed the session, as it does for a fetch stream that is for no FETCH of the endpoint. The page got no Object |

- So the publisher's side is run against a relay that is not ours, up to the relay's reading of the fetch stream; the way from that relay to a viewer is not, for the Request ID. The matrix has the run as that: the relay's line, the FETCH_OK the page that fetches got, and that the publishing page served its FETCH whole.
- **This is read as a fault of the `moqtail` relay.** §11.4.4 has the FETCH_HEADER name the FETCH: "all objects on the stream belong to the track requested in the Fetch message identified by Request ID". The session of the relay and the page that fetched has one FETCH, with Request ID 2; 3 is odd, and §10.1 has "the server generates odd numbered Request IDs", so it is no ID that page could have given a request. That the relay copies the publisher's header as it is, is a guess: its source was not read. The own relay writes a header with the subscriber's Request ID (`crates/relay-core/src/fetch.rs`).

### In the tests, without a browser

- `fixtures/peer-fetch/opened.tsv` (22 rows) and `updates.tsv` (13 rows): what becomes of a FETCH of the peer, and what is sent for each thing that happens to one. **Run through the Rust session before the outcomes were written** (a test that is not kept, on a copy of `crates/` at `59d76ab8`), then through `@xmoq/moqt`: 35 of 35 the same, but for the Joining FETCH that `@xmoq/moqt` refuses by design.
  - Two outcomes had been written from reading `peer.rs`, and were wrong: an End Location that names the Start Location is taken, in a FETCH and in its FETCH_OK.
  - An End Location in the answer that lies beyond what was asked for is not checked by either session.
- 22 temporary changes to the implementation, one at a time, each taken back: each makes between 1 and 14 tests fail. Two did not at first: a REQUEST_UPDATE of a FETCH that did not use up its Request ID (a test was added), and one that was no change to what it meant to break.

### After two reviews

Changed, with a test each, on Bun 1.3.14:

- **Writes that are not waited for were held without an end**: 20,000 writes of 1 KB to a subscriber that read nothing were all taken and held, about 37 MB. A stream now has 16 writes under way at most, on a fetch stream and on a subgroup stream alike, and one more is refused with a `RangeError` that leaves the stream as it was.
- **REQUEST_UPDATEs that came before the answer to a FETCH were counted without an end**, and each got its REQUEST_OK in one go after the FETCH_OK: 50,000 of them, 50,002 outputs. 64 may wait, as in the relay; one more refuses the FETCH with `EXCESSIVE_LOAD`.
- A handler that was writing when the subscriber gave the FETCH up had the rejection of its write reported as an error of the application. It is not one, and is no longer reported; what a handler throws after it closed or reset the writer itself still is.
- An End of Range indicator that goes back took the encoder back with it, and an Object that goes back was then written. It is refused like such an Object.
- `close()` left a timer behind where the subscriber had finished its direction before.
- At most 32 FETCHes of the peer are open at once, where it was 16, the number the relay asks one publisher for: the relay has room for its next FETCH when it cancels one, and the page when it has read that, on another stream. **Not seen in a run**; the number is twice the relay's by that argument alone.

`mise run check`: 961 tests of `@xmoq/moqt`. Each of 14 temporary changes that take one of these back makes between 1 and 4 tests fail.

### Later the same day: the page that has Objects it never sent goes on after them

In the run "Objects the page has and never sent", the page said in its SUBSCRIBE_OK that its Largest Object is {102, 0} and then sent Groups 0, 1 and 2. That passes only through a relay that does not look at the Subscription Filter: both subscribers of the run ask with Largest Object, and draft 18 §5.1.2 has such a subscription begin after the Largest Object. The page now sends Groups 103 and on, so the three Objects are ones it published before the relay subscribed. The range of the FETCH and what comes back are as before.

| Relay | Run | Seen |
|---|---|---|
| own, `main` at `c212d503` | as it is now | passes: FETCH_OK with End Location {102, 1} and the three Objects |
| `moqtail` 0.15.0 | as it is now | passes. **One expectation changed, from the draft and not from the run**: the page's FETCH_OK to the relay has {102, 0}, the End Location the relay asked with (the whole of Group 102), where it had {102, 1}. The page has sent later Groups, so that end is no longer beyond its Largest Object (§10.13). The first run with the old expectation failed on exactly that line |
| own, **built from the change that makes the relay keep to the filter** (`fb0e72e1`, not in `main`), its binary put in place of the relay's for the run | as it is now | passes: both subscribers get Group 106 |
| the same relay | as it was (the page sends from Group 0) | **fails**: the subscriber the relay asks the page for waits for Group 0, which the relay does not send it |

- The relay's binary was put back after the two runs and has the checksum it had before.
- `mise run check` passed. `mise run interop -- --base-port 45300`: 102 of 102 pairings in the second run. In the first, one pairing that has no part in this (subscribers that are killed) stood still from the start of its relay to the limit of 120 seconds, while another working copy ran its own interop on the machine; it was not looked into further.

## Limitations and blockers

- Chromium only, loopback only, Objects of some bytes, one Object a Group, Ascending order. A fetch stream larger than a window, a subscriber that reads slowly, and Descending order were run against the fake transport and the session alone.
- "A subscriber stops the fetch stream alone, and the FETCH is given up" is what the own relay does for a subscriber of its own (the notes, "The subscriber gives its FETCH up"). No Rust client answers a FETCH yet, so the reading of `@xmoq/moqt` as the origin of a track has only the relay to compare with.
- A FETCH of the peer that was accepted and that the subscriber neither reads nor gives up holds its two streams for as long as the session lasts. Behind the own relay that does not happen: the relay gives up on a subscriber that does not read, and cancels upstream.
- The viewer of the Joining run sends its Joining FETCH after its SUBSCRIBE_OK. One that sends the two at once was not run.
- A FETCH from the own Rust client was not run.

## Open questions raised

- The `moqtail` relay's fetch stream under its own Request ID was not reported to its authors.
