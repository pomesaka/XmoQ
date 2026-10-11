# The FETCH runs of the interop matrix, with a relay that answers from its cache

- Date: 2026-10-11
- Commit SHA: the commit that changes the runs, together with this record. The runs were made on its working copy, on the change that gives the relay its cache of the latest Group, which was under review and not in `main`
- Roadmap item: Phase 1, "The relay keeps the latest Group of each track and answers a FETCH from it where it can". This is the interop side of it

## Objective

The own relay now keeps the latest Group of a track and answers a FETCH that lies in it without asking a publisher. Two runs failed for it, because they asked for the latest Group and expected a publisher's answer. To do: have each run that is about a publisher's answer ask for what the relay does not hold, add a run in which the relay answers, check in each who answered, and show the relay's writing being cut short.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | own, with the cache (`b43c0eee`, a copy of the change under review); `moqtail` relay 0.15.0 |
| Draft | draft-ietf-moq-transport-18 |

## Commands

```sh
mise run check
mise run interop -- --base-port 45300
(cd interop && bun fetch.ts --fetch latest-group --relay own)          # each run, alone
(cd interop && bun fetch.ts --fetch page-large-group-cancelled --relay own)
```

## Observed results

### Before the change

`mise run interop`: 102 of 104. The two that failed got FETCH_OK with End Location {0, 1} and Object {0, 0} from the relay, where one expected REQUEST_ERROR from `own-rust` and the other a FETCH that nobody answers. In the logs of all the runs with a FETCH, the relay wrote a fetch stream itself in those two and in the run with FILL_TIMEOUT 0 (its one indicator), and sent a FETCH to a publisher in every other: no run passed with the wrong one answering.

### The runs as they are now

| Run | Expected, and seen |
|---|---|
| `own-rust`, which refuses a FETCH, sends Groups 0 and 1; a page asks for Group 0 | The relay holds Group 1 alone and asks: REQUEST_ERROR 16, and `OPEN_REQUEST … FETCH` in the relay's log. As expected |
| `moqtail-rust`, which answers no FETCH, sends Group 0, a second publisher Group 1; a page asks for Group 0 and gives up after a second | The relay asks the first publisher, and cancels that FETCH when the page gives up. As expected. `moqtail-rust` sends one Group and no more, which is why the later Group comes from a second publisher |
| **New**: `own-rust` sends Group 0; a page asks for {0, 0} to {1, 0} | The relay answers: FETCH_OK with End Location {0, 1} and the one Object, `WRITE_FETCH_STREAM … pieces=1`, and no FETCH to the publisher. As expected, through the own relay; the `moqtail` relay gives the same answer, which was the expectation of the old run through it |
| **New**: a page publishes a Group of 8 Objects of 900,000 bytes; another asks for it, reads nothing and gives up after a second | FETCH_OK with End Location {3, 8} and `WRITE_FETCH_STREAM … pieces=8`, no FETCH to the publisher: as expected. **`ABORT_FETCH_WRITE` was expected and did not come, in 7 runs of 7** (below). With the line the relay prints for the end of a write, the run now waits for `FETCH_WRITE_ENDED … stopped pieces=K code=0x1` with K below 8: `pieces=6` in 6 runs of 6 |
| **New**: a page publishes a Group on two subgroup streams (Subgroup 0 with Publisher Priority 10 and a property of an odd type, Subgroup 1 with 200 and one of an even type), written in the order 1, 0, 2, then 4, 3, 5. Another page subscribes after the first three and sends a Relative Joining FETCH with Joining Start 0, and asks for the whole Group when it is there | The relay answers both itself, in the order of the Object IDs, each Object with its Subgroup ID, Publisher Priority, property and payload: Objects 0 to 2 with End Location {3, 3} (`pieces=3`, `finished`), and all six with {3, 6} (`pieces=6`, `finished`). The subscription brings Objects 3 to 5 and no other of the Group. No FETCH to the publisher. As expected, at the first run |

- Temporary changes to the runs, each taken back: with one Group where two are to be sent, the run with the publisher that refuses fails (the relay answers); without the second publisher, the run with the FETCH that is given up fails; with two Groups before the FETCH of the latest, that run fails (the publisher refuses); with the publisher named as the one to answer the latest Group, it fails waiting for the relay's FETCH.
- `mise run check` passed. `mise run interop -- --base-port 45300`: 106 of 106 pairings, the 104 from before and these two; 107 of 107 with the run on two subgroup streams.
- Temporary changes for the two runs above, each taken back: a large Group of 3 Objects fails the run (`finished pieces=3`); the fetch stream expected in the order the Objects were sent fails; a page that reports every fetched Object as of Subgroup 0, without properties, or with Publisher Priority 10, fails each; Objects 0 to 5 expected of the subscription fails.

### The relay's writing, cut short

In the run with the large Group the page took about 0.5 MB of the 7.2 MB and no more (four chunks of the fetch stream in its log, with `--verbose`), so the relay's writer stood still when the page gave up. What the relay printed then was `RESET_REQUEST session=2 stream=2 code=0x1` and, after it, `RESET_RECEIVED session=2 stream=2 code=0x1`; no `ABORT_FETCH_WRITE`.

- **Why, as read from the two sides and not shown by a run**: `cancel` of `@xmoq/moqt` resets the request stream and stops the fetch stream at once. The relay's writer meets the stop of the stream it writes and ends by itself; the relay then resets the request stream, and its order to stop the writing has nothing left to stop.
- So the run takes either line, and says in its summary which it was.
- **`ABORT_FETCH_WRITE` is still not seen with real streams.** It needs a subscriber that resets or stops the request stream and leaves the fetch stream alone, which a page of `@xmoq/moqt` does not do.

## Limitations and blockers

- **`pieces=6` of 8 in the run with the large Group is what the relay handed to the transport, not what the page took**: the page's log of one run with `--verbose` had about 0.5 MB. Two Objects is the margin by which the run tells a write that was cut short from one that was whole; the Group cannot be larger, for the 8 MiB the relay keeps.
- Not written: a run in which a publisher leaves an Object of the Group out, so that the relay asks it; a FETCH of the latest Group from the own Rust client and from the `moqtail` clients.
- The relay with the cache is a copy of a change under review; what it does can still change.

## Open questions raised

- Whether a subscriber of ours should have a way to give a FETCH up by the request stream alone, as §5.2 has it ("It MUST send STOP_SENDING for the bidi request stream", and MAY for the data stream), which would also be the way to see `ABORT_FETCH_WRITE`.
