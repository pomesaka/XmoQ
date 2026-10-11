# A page publishes an MSF catalog, and a page and moqlivemock's subscriber read it

- Date: 2026-10-11
- Commit SHA: the commit that adds `createCatalogPublisher` to `@xmoq/msf` together with this record. The runs were made on its working copy, on `main` at `36e51810` with the two changes that read a catalog over a connection below it
- Roadmap item: Phase 1, "MSF catalog track: published by the browser publisher, read by the viewers". With this the catalog is published and read; no media is published with it

## Objective

`@xmoq/msf` could read a catalog track and had nothing that publishes one. This change adds the publisher. The runs are to show that

1. a catalog a page publishes reaches a page that reads it as MSF-01 §5 asks, through the own relay, which keeps nothing and asks the publisher for the range of the viewer's Joining FETCH;
2. a catalog that replaces it reaches the reader, which no publisher that is not ours could show (`mlmpub`'s catalog never changes);
3. a subscriber that is not ours reads the catalog.

Before the publisher was written, one thing was not known: how `mlmsub`, the subscriber of moqlivemock, ends when the catalog it reads has no track it can play.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | the own relay of `main` at `36e51810` |
| Subscriber that is not ours | `mlmsub` of `Eyevinn/moqlivemock` v0.16.1 (commit `91d903e9`), Go 1.26.0, **built with a patch of one line** (`interop/peers/moqlivemock/mlmsub-legacy-connect-protocol.patch`) |
| Draft | draft-ietf-moq-transport-18; draft-ietf-moq-msf-01 |
| Runtime of the tests | Bun 1.3.14 |

## Commands

```sh
mise run check
mise run interop -- --base-port 45300
(cd interop && bun catalog.ts --catalog own)         # one run, alone
(cd interop && bun catalog.ts --catalog to-mlmsub)
```

## Observed results

### Before the publisher was written: `mlmsub` against the relays

A page of the interop run that answers a FETCH from what it sent published the JSON of a catalog as its Objects, and `mlmsub` was told to take that track for its catalog.

| `mlmsub` | Relay | Seen |
|---|---|---|
| as the tag builds it | own | **No session.** The relay: `HANDSHAKE_FAILED connection lost: failed to exchange h3 connect: protocol error: expected webtransport, got: Some("webtransport-h3")`. `mlmsub`: `http3: parsing frame failed: Application error 0x0 (remote)` |
| as the tag builds it | `moqtail` relay 0.15.0 | **No session.** `mlmsub`: `http3: parsing frame failed: EOF` |
| **with `LegacyConnectProtocol: true`** in its dialer | own | A session. `fetched catalog via joining fetch groupID=2 objectID=0`, the catalog printed and written to `-catalogout`. Then `no matching tracks found`, and after its `-duration` it exits with 1 (`context deadline exceeded`). The same for a catalog whose `tracks` is empty |

- The first row was known ([survey](2026-10-10-loc-msf-fetch-survey.md), section 8) and was overlooked when the peer was chosen for this run: the two sides send and accept different tokens in the `:protocol` of the CONNECT request of WebTransport. The patch makes `mlmsub` send the one the relay accepts, and touches nothing of MoQ Transport or MSF; `interop/peers/moqlivemock/README.md` has it.
- **Every row below that has `mlmsub` is with that patch.**

### The runs in the matrix

What has to be seen was written at the top of `interop/catalog.ts` before the runs. Both came out as written in the first run.

| Run | Expected, and seen |
|---|---|
| A page publishes a catalog of one LOC video track; another page reads it; the publisher replaces it with one of two tracks | The catalog is at Object 0 of a Group that is the time in milliseconds ({1791689859543, 0} in the run that was read). The reader has it, at that Location, and its Joining FETCH was served with one Object. The replacement is at Object 0 of the next Group, and the reader has it as its second state, not outdated |
| The same publisher; **`mlmsub` with the patch** reads | `fetched catalog via joining fetch` at the publisher's Location; what `-catalogout` has is read by `@xmoq/msf` as the catalog that was published |

- `mise run interop`: 104 of 104 pairings, the 102 from before and these 2.
- Temporary changes to the publisher, each taken back: with no update sent, the first run fails waiting for the second state; with every FETCH refused, both fail, the page for its first state and `mlmsub` for its line; with no LARGEST_OBJECT in the SUBSCRIBE_OK, the first fails, as the reader then sends no FETCH.
- **Not noticed by a run:** a publisher that sends the catalog that is there on a new subscription as well. The reader takes an Object once, so the run passes; the tests of the publisher have it.

### In the tests, without a browser

- `nextPublication`, 13 rows: which catalog may follow which.
- `answerCatalogFetch`, 14 rows: the End Location and what goes onto the fetch stream, for ranges before, over and after the catalog, in both orders, with and without catalogs that were replaced. The rule is the Rust client's for a publisher that answers from what it published; one expectation was written wrong at first (a whole Group of what was replaced ends at the last Location of that Group, as in the Rust client, not at its Object 0).
- `createCatalogPublisher` against requests and subscriptions made by hand: which requests it takes, the answers, the update to every subscription, a slow subscriber, FORWARD 0, the end.
- 34 temporary changes to the implementation, one at a time, each taken back. 31 made between 1 and 16 tests fail. Two did not and got a test each (a request for the catalog of a namespace that is the start of this one; a subscription that ended and was kept); one was no change to what it meant to break.

### After the two reviews of the change

The reviews ran three faults of the publisher, and all three were in the tests' blind spots: a catalog published right before `end` reached nobody; `nextPublication` compared the objects it was given, not what is written; a request or a stream that failed for another reason than that the peer went was left as it was. They are put right in the same change, with what else the reviews named.

- **The run of two pages now ends the broadcast**: after the second catalog the publisher page calls `update` with a catalog that has `isComplete` and no tracks, and `end` right after, with nothing awaited between (MSF §11.3). Expected, written before the run: the reader has a third state, that catalog at Object 0 of the Group after, and its subscription ends with PUBLISH_DONE and TRACK_ENDED. Seen in the first run, at {1791691570762, 0}.
- With the fault back for one run (the subscriber taken out of the set before the catalog is written to it), that run fails: the reader gets `publish-done` with Status Code 2 and waits in vain for the last catalog. Taken back; the run passes again.
- The hand-made subscription of the tests refuses what the real one refuses: a subgroup on a subscription that is over, a write to a stream that was reset, and PUBLISH_DONE while a subgroup stream is neither closed nor reset. With it, a stream that is left open after a failed write shows as a PUBLISH_DONE that is not sent.
- 19 temporary changes to what was put right, one at a time, each taken back: all 19 made between 1 and 9 tests fail. A twentieth was not applied (its text was not in the file) and is not counted.
- `mise run check` and `mise run interop -- --base-port 45300`: passed, 104 of 104 pairings.

### After the review of those changes

- **The mark that a catalog is being written to a subscriber left a gap**: it was taken away one turn of the microtasks after the loop had looked for a newer catalog, and an `update` in that turn was written to nobody until the next. The review ran it. The mark is now taken away in the turn the loop ends in. A test publishes a catalog 0 to 6 microtasks after the stream of the one before was closed; against the version that was reviewed the row with 2 fails. **The first way it was put right moved the gap by one turn and did not close it**, which that test showed.
- The second time `end` gives a subscriber counted the time PUBLISH_DONE takes, which the library lets take 3 seconds for a subscriber that does not end the request stream: a fault was reported for a subscriber that had been told. It now counts the write alone. The tests' subscription can be made slow to answer `done`, as the real one is.
- **The Location of the End of Unknown Range indicator follows the rule the Rust client has now**: Object 0 of the last Group that was let go of and lies in the range, or of the first in Descending order, and no indicator for a range that has no such Object. The expectations of the rows were written from that rule and from the cases of the Rust test before the code was changed. **Later the two were run beside each other**: the rows went into `fixtures/published-fetch/answers.tsv`, where the Rust client gave the same answer for all 19, and the test here reads them from that file; and the reviewer of the change ran both over 4,356 ranges and found no difference.
- 20 temporary changes, each taken back: all made tests fail, two of them after a test was added (the timer of the second wait; the look at the subscriber after its stream opened).
- `mise run check` and `mise run interop -- --base-port 45300`: passed, 104 of 104 pairings.

## Limitations and blockers

- Chromium only, loopback only, one publisher and one reader a run.
- **`mlmsub` runs with a patch.** As the tag builds it, it connects to neither relay here.
- No media: the catalog names a video track that nobody publishes.
- **No cache in the relay.** The catalog is there while its publisher is: the relay asks the publisher for every Joining FETCH. A second viewer whose FETCH fails gets nothing until the next update.
- A FETCH that is not the one a viewer makes (a range before the catalog, Descending) was run against the tests alone. The Rust client's answers to the same ranges are the rows of `fixtures/published-fetch/answers.tsv`, which both read.
- The catalog of a page that is loaded again begins at the time of that load. A clock that went back was not tried.
- `mlmsub` was not shown a catalog that replaces another, nor the end of the track: it stops after the first.
- `end` with a subscriber that takes nothing (the reset after 3 seconds) ran against the hand-made subscription alone, with 20 ms for the time.
- **Read from the code and not run**: an update whose stream to the relay is lost leaves the relay with the Group before as the largest, so a viewer that comes then gets an answer to its Joining FETCH without the catalog, and has none until the next update. And a viewer that comes while an update is on its way to the relay can get such an answer too; `mlmsub` would end there with "joining fetch returned no catalog objects". Neither happened in a run, and nothing here makes them happen.

## Open questions raised

- The own relay accepts `webtransport` alone as the `:protocol` of a CONNECT (`web-transport-proto` 0.6.2). A client that sends the token of the later draft, as moqlivemock's library does by default, cannot connect to it.
