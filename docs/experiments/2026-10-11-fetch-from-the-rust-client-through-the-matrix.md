# The own Rust client fetches in the interop matrix

- Date: 2026-10-11
- Commit SHA: the commit that adds `interop/fetch-rust.ts` together with this record. The runs were made on its working copy, on `main` at `a5b8f5b0`
- Roadmap item: groundwork for Phase 1 (a viewer gets the catalog of a broadcast with a Joining FETCH). It closes no item

## Objective

The matrix had a page as the only client that sends a FETCH. The runs are to show that

1. the own Rust client gets the same answers, for a Standalone FETCH and for a Joining FETCH, from a page that answers a FETCH and from `moqtail`'s publisher;
2. a viewer that sends its SUBSCRIBE and its Joining FETCH at once gets its FETCH answered, whichever of the two the relay reads first.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser (the publishing page) | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relays | the own relay of `main` at `a5b8f5b0`; for the last table also the own relay of the change that holds a Joining FETCH (`1c7a12f0`, not on `main`); `moqtail` relay 0.15.0 |
| Clients | the own Rust client of `main`; `moqtail`'s test client 0.15.0 |
| Draft | draft-ietf-moq-transport-18 |

## Commands

```sh
mise run interop
cd interop
bun fetch-rust.ts --fetch page-join-at-once --relay own   # one run of the matrix on its own
```

## Observed results

What is expected was written before the runs, at `EXPECTED` in `interop/fetch-rust.ts`. All came out as written in the first run.

| Run | Relay | Seen |
|---|---|---|
| Standalone, {0, 0} to {2, 1}, from a page | own | FETCH_OK with End Location {2, 1}; Object 0 of Groups 0, 1, 2 with the page's payload; a FIN |
| The client subscribes, then a Relative Joining FETCH, one Group back, from a page | own | FETCH_OK with End Location {2, 1}; Object 0 of Groups 1 and 2 |
| The same, an Absolute Joining FETCH from Group 1 | own | The same |
| Standalone, {0, 1} to {1, 2}, of `moqtail`'s test client | own, `moqtail` | FETCH_OK with End Location {1, 2}; {0, 1}, {0, 2}, {1, 0}, {1, 1} |
| A Relative Joining FETCH, one Group back, of `moqtail`'s test client, which goes on publishing | own | In the run that was read: End Location {3, 1}; the three Objects of Group 2 and {3, 0}. The run takes any End Location {G, o} and wants the three Objects of Group G - 1 and the first `o` of Group G |

- `mise run interop`: 100 of 100 pairings, the 94 from before and these 6.
- Temporary changes, each taken back: with Joining Start 0 the second run fails with Object {2, 0} alone.

### The SUBSCRIBE and the Joining FETCH at once (`--join-at-once`)

| Relay | Runs | The FETCH refused with `INVALID_JOINING_REQUEST_ID` (0x32) | The relay read the FETCH first and held it |
|---|---|---|---|
| `main` at `a5b8f5b0` | 50 | **3** | it has no such thing |
| the change that holds a Joining FETCH (`1c7a12f0`) | 50, then 30 with the check below | 0 | 9 of 50, 3 of 30 |

- Where the relay printed `HOLD_REQUEST session=2 stream=2 FETCH`, it printed `SEND session=2 stream=1 SUBSCRIBE_OK` next and `SEND session=2 stream=2 FETCH_OK` after it, and the client got Object 0 of Groups 1 and 2, as in the run that sends the FETCH after its SUBSCRIBE_OK.
- The run checks that a FETCH that was held got its FETCH_OK on that stream. With the check looking for another line, the run fails in the first round in which the FETCH was held.
- The run went into the matrix together with the change that holds a Joining FETCH: through the relay of `main` it failed in 3 of the 50 runs above.

## Limitations and blockers

- Loopback only. Which of the two streams the relay reads first is not chosen by the run: 12 of 80 had the FETCH first.
- The runs above were made on a working copy whose `mise run check` failed in one Rust test, for a change that is not part of this one (two rows of `fixtures/peer-subscribe/opened.tsv` the Rust test did not know yet).
- No own Rust publisher answers a FETCH, so the publishers are a page and `moqtail`'s test client.

## Open questions raised

None.
