# moqlivemock's subscriber takes a finished request stream for a failed FETCH, at times

- Date: 2026-10-11
- Commit SHA: the commit that has `@xmoq/moqt` keep its direction of the request stream of a FETCH it served open for the subscriber, together with this record. The runs were made on its working copy, on `main` at `20452147`
- Roadmap item: Phase 1, "The relay answers a FETCH" and "MSF catalog track". It closes no item

## Objective

One pairing of the interop run failed once in CI: an own page publishes a catalog, and `mlmsub`, the subscriber of moqlivemock, reads it through the own relay with a Joining FETCH. `mlmsub` said `joining fetch: reading catalog: peer finished its half of the request stream`. To find: why, whether it happens on the machine here, and what the draft has a publisher do with the request stream of a FETCH.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Subscriber | `mlmsub` of `Eyevinn/moqlivemock` v0.16.1 (commit `91d903e9`), with the patch of this repository; its library is `Eyevinn/moqtransport` v0.14.0 |
| Relay | the own relay of `main` at `20452147` |
| Browser | Chromium 156.0.8078.4, headless |
| Draft | draft-ietf-moq-transport-18 |

## Commands

```sh
# the one pairing, by itself, 40 and then 100 times
(cd interop && bun catalog.ts --catalog to-mlmsub --port 45391)
mise run check
mise run interop -- --base-port 45300
```

## Observed results

### The failure, here

| Runs | Passed | Failed with that error |
|---|---|---|
| 40, before any change | 35 | 5 |
| 100, with the change to `@xmoq/moqt` below, the relay as it is | 83 | 17 |

In a run that fails, the relay's log has the lines of a run that passes, in the same order: the fetch stream is passed on to its end, the relay finishes its request to the page, and then finishes its direction of `mlmsub`'s request stream (`FETCH_PIPE_ENDED … finished`, `FINISH_OWN_REQUEST`, `FINISH_REQUEST`). In a run that passes, `mlmsub` then resets the request stream with CANCELLED.

- The first 40 runs that were tried all failed for another reason: the `mlmsub` of the working copy had been built without the patch, on a revision that did not have it. Those are not counted.

### Why, from the code of moqtransport 0.14.0

- The reader of a request stream takes a FIN of the peer for the end of the request, with the cause "peer finished its half of the request stream" (`request_stream.go`, `run`).
- `FetchStream.ReadObject` waits for three things at once: an Object, the end of the data stream, and the end of the request. Where the end of the request is seen first and the data stream is not yet read to its end, it returns that cause (`fetch.go`, `ReadObject` and `drained`). A comment has the assumption: "A request stream that ends without the data stream having finished means the response will never complete."
- The two streams arrive apart. A publisher that finishes the data stream and then the request stream can have the second seen first.
- **As a publisher the library does not finish the request stream of a FETCH itself** (`FetchResponse.Close` finishes the data stream alone); its subscriber resets the request stream when it has the answer and the data. Between `mlmsub` and `mlmpub` the race cannot happen.

### What the draft says

§5.2: "A subscriber keeps FETCH state until it cancels the request (see Section 3.3.2), receives REQUEST_ERROR, or the FETCH data stream receives a FIN or is reset", and of the publisher, "It can destroy all FETCH state after closing the data stream with a FIN." A FIN of the request stream is not among what ends the subscriber's state, and nothing asks the publisher for one. So the relay does nothing §5.2 forbids, and a publisher that leaves its direction open for the subscriber does nothing it forbids either.

**The draft can be read the subscriber's way as well.** §11.4.1: "Termination of a bidi request stream terminates the Subscription, Fetch, Track Status, Publish Namespace, or Subscribe Namespace request." Where a FIN counts as a termination, a request stream that is finished ends the FETCH, which is what moqtransport does; `@xmoq/moqt` reads the sentence so itself for a peer's FIN on a request that is not answered. The two sections do not say which holds for a FETCH whose data stream is still on its way, so this is two readings, and not a fault of the subscriber.

### The change to `@xmoq/moqt`

`FetchWriter.close()` finishes the fetch stream, and no longer the request stream with it: this endpoint's direction is finished when the subscriber has ended its own (a FIN or a reset), or after `peerStreamEndTimeoutMs` (3 seconds). After that the subscriber has that time again before its direction is stopped, as before.

- The tests of the session and of the connection have the new order, with the clock moved by hand: finished at once where the subscriber had finished before; finished when it finishes or resets; finished when the time is over and not a millisecond before it; the subscriber's direction stopped when that time is over again. **The tests run with 20 ms for the time**, so what they show is 19 ms and 20 ms, and 3 seconds is the value without a setting, which no test waits for. Eight temporary changes, each taken back, made tests fail.
- `mise run check` passed; `mise run interop -- --base-port 45300`: 104 of 104 pairings.
- In a run of a page that answers a relay's FETCH (`fetch.ts --fetch page-standalone`), the relay finished its request when the stream had been passed on, the page saw that (`peer-direction-ended … fin`), and no wait ran to its end.
- **The `moqtail` relay does not end its direction first.** In the run of a page that answers the FETCH of that relay (`fetch.ts --fetch page-unsent --relay moqtail --verbose`, with the end of the run held back by 4.5 seconds for this, a change that was taken back), the page said `fetch served`, the relay logged "Stream closed cleanly while waiting for data" 3.001 seconds later, when the page's FIN reached it, and the page then saw the relay's direction end (`peer-direction-ended … fin`). So with this relay as the subscriber the whole first wait runs, and the request stream and its place are held for 3 seconds a FETCH. It is the one pairing of the matrix in which `@xmoq/moqt` answers a FETCH of a `moqtail` peer; the pairing passes, as it does not wait for the page's FIN.
- **A subscriber that stops this endpoint's direction during the wait ends both at once** (added after the review that compared the three implementations): the direction is reset with CANCELLED, the subscriber's is stopped, and the stream is forgotten, as §5.2 has it for a STOP_SENDING ("It MUST reset the bidi request stream") and as the relay does. Before, the stop was only noted, and the stream was held for both waits.

## Limitations and blockers

- **The failure is not gone**: the relay is what `mlmsub` talks to in this pairing, and it still finishes the request stream at once. 17 of 100 runs failed after the change here, which is no more than chance against 5 of 40. **The record of the Rust side has 5 of 100 before its change, a third of the 22 of 140 here, and why is not known**: the runs here were made while other working copies ran their own interop on the machine, and whether that is the reason was not tried. The same change in the relay and in the Rust client is the other half.
- **That the change in the relay ends the failure was not run.** It follows from the code of the library, which was read.
- What the `moqtail` relay does with the request stream of a FETCH it answered was not looked at.
- A subscriber that never ends its direction keeps the request stream, and its place among the requests of the peer, for 6 seconds where it was 3.
- One machine, loopback. How often the race is lost on a real link was not measured.

## Open questions raised

- Whether to report the race to `Eyevinn/moqtransport`.
