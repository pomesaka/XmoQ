# A page stops a peer that does not end its direction after a refusal

- Date: 2026-10-11
- Commit SHA: the commit that makes `@xmoq/moqt` wait for the peer's end after a refusal, together with this record. The runs were made on its working copy, on `main` at `a5b8f5b0` with four changes that are not on `main` below it
- Roadmap item: groundwork for Phase 1 (a browser publisher that a relay can ask for a track). It closes no item

## Objective

`@xmoq/moqt` keeps a request of the peer that it refused until the stream of the request is gone, and counts it among the 64 requests the peer may have (`limits.maxPeerRequests`). A peer that never ends its direction held that place, and one of its 64 bidirectional streams, for good. This change gives the peer 3 seconds after the refusal and then stops its direction. The runs are to show that

1. the stop is sent after that time, with `CANCELLED`, and the session stays open;
2. a peer that ends its direction in time is not stopped.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Server | `spikes/wt-write-abort` on `web-transport-quinn` 0.13.2, the library of the own relay, with an idle timeout of 30 seconds |
| Draft | draft-ietf-moq-transport-18 |
| Runtime of the tests | Bun 1.3.14 |

## Commands

```sh
(cd spikes/wt-write-abort && cargo build --locked)
bun spikes/wt-write-abort/moqt-refused.ts --rounds 5
mise run check:ts
mise run interop
```

## Observed results

### In Chromium

The server sends the SETUP of the `moqtail` relay (`fixtures/moqt-18/setup-moqtail-relay.hex`), opens a bidirectional stream with a request, and reads the answer to its FIN. The page waits 4.5 seconds and says whether its session is open. 5 runs each, all the same; `spikes/wt-write-abort/out/moqt-refused-5.txt` has every line.

| The request | The server after the answer | What the server saw | The page after 4.5 s |
|---|---|---|---|
| A TRACK_STATUS, which the session refuses by itself (`NOT_SUPPORTED`) | leaves its direction open | STOP_SENDING with code 1 (`CANCELLED`), 3,002 to 3,004 ms after the FIN of the answer | open |
| A SUBSCRIBE, which the page rejects with `DOES_NOT_EXIST` | leaves its direction open | the same, 3,001 to 3,002 ms | open |
| The TRACK_STATUS | finishes its direction | no stop | open |

- A first run had the session end after 2 seconds in every row: the server of the spike had an idle timeout of 3 seconds, and neither side sends anything while it waits. The timeout is now set by the run.

### In the tests, without a browser

- Against the session alone, for six ways a request is refused (the application rejects a SUBSCRIBE or a FETCH; the session refuses a SUBSCRIBE, a FETCH, a request it does not implement; a REQUEST_UPDATE before the answer): when the wait is over the direction is stopped once, what still arrives is dropped where it would close the session on a stream that is read, the place among the peer's requests is free, and nothing of the stream is left when it has ended. A FIN or a reset of the peer is reported, and nothing is stopped after it.
- Against the fake transport: the same six, the stop not before the time and at it, the place among the peer's streams free again, and no timer left when the peer ended its direction in time or the session ended.
- 11 temporary changes to the implementation, one at a time, each taken back. 10 made between 2 and 21 tests fail. One did not: a request the application rejects after its peer had finished its direction was not reported as ended, so its wait would have run for nothing. A test was added, and that change now makes 2 fail.

## Limitations and blockers

- No relay was the peer: neither the own relay nor the `moqtail` relay leaves its direction open after a refusal, so the peer is the server of a spike.
- A request that is refused while as many are kept as may be is given the same 3 seconds. Stopping it at once was not tried: §3.3.2 has a request rejected with "a REQUEST_ERROR and FIN" and has a direction stopped to cancel a request, and what a peer that is not ours makes of a stop that can arrive before the answer was not looked at.
- The time is provisional, like the one after PUBLISH_DONE that it is taken from.
- `mise run check` fails in this working copy in one Rust test that reads `fixtures/peer-subscribe/opened.tsv`, for two rows a change below this one added.

## Open questions raised

- What the Rust side does with a refused request whose peer does not end its direction is not written yet. Until it is, the two implementations differ here.
