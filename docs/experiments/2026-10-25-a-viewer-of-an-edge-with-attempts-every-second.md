# A viewer of an edge, with attempts to connect every second

- Date: 2026-10-25
- Commit SHA: the commit that adds this record (`connect` of `xmoq-moqt-io` beginning one more attempt every second, on `main` `76c58169`; the page's frame times and `spikes/edge-stall/run.ts` of #350 applied for the runs and not part of the commit)
- Roadmap item: Phase 7, relay scale; the fix of what [a viewer of an edge when the origin goes](2026-10-25-a-viewer-of-an-edge-when-the-origin-goes.md) found

## Objective

That record found an edge connecting 3.9 s after its origin was back: its one attempt to connect had begun while nothing listened on the port, and QUIC sent its Initial again only after waits that double. `connect` now begins one more attempt every second while none has its session (`CONNECT_ATTEMPT_EVERY`), within the 10 seconds as before, takes the first and closes the others. Does the viewer of an edge now lose its picture for no longer than one at the origin itself?

**Loopback, one machine**, as before.

## Expected

Said before the runs, in the proposal of the fix: with SIGINT and the origin down for 3 s, the edge connects within about a second of the origin's being back, and the viewer at the edge loses its picture for about as long as one at the origin (3.3 to 4.5 s in the record before). The other cases do not change: with SIGKILL it is the idle timeout, and with SIGINT and no time down the first attempt was already within a second.

## Environment

As in the record before: Apple M3 Pro, macOS 14.4, Chromium of Playwright, the own relays (debug builds) on 34500 and 35500, the bars of `interop/video.ts`; the same 24 runs of `spikes/edge-stall/run.ts`, with the lock of the full interop held (11:34 to 11:39).

## Result

**As expected.** The stall in milliseconds, three runs each, beside the record before:

| Stop | Down | Viewer at the edge, now | before | Viewer at the origin, now | before |
|---|---|---|---|---|---|
| SIGINT | 0 s | 1,119 / 1,083 / 1,135 | 1,084 / 1,086 / 1,104 | 401 / 401 / 417 | 352 / 399 / 915 |
| SIGINT | 3 s | **4,154 / 4,087 / 4,119** | 7,090 / 7,153 / 7,069 | 3,952 / 3,286 / 4,168 | 3,345 / 4,532 / 3,885 |
| SIGKILL | 0 s | 6,585 / 6,609 / 6,557 | 6,556 / 6,568 / 6,569 | 5,569 / 5,581 / 6,089 | 6,086 / 6,097 / 5,526 |
| SIGKILL | 3 s | 6,607 / 6,602 / 6,576 | 6,520 / 6,586 / 6,608 | 5,606 / 5,552 / 5,556 | 5,572 / 5,572 / 5,575 |

- With SIGINT and 3 s down the edge printed UPSTREAM_CONNECTED at 4,010 to 4,012 ms, **0.9 s after the origin was back** (3,114 to 3,117 ms), where it was 3.9 s; the viewer at the edge now loses its picture for about as long as one at the origin.
- The relay above printed HANDSHAKE_FAILED once to three times in each of these runs (`closed by peer: 0` during the handshake): the attempts of the edge that reached it and were given up when another had its session. That none of them stays open was looked at in the test below, not in these runs.
- With SIGINT and no time down the stall stays at 1.1 s: the first attempt began 1 to 4 ms after the stop, before the origin was back (113 to 115 ms), and the edge connected with the second, begun a second later.
- SIGKILL is as before: the idle timeout of 6 s.

The test `a_client_reaches_a_server_that_comes_after_it_began_within_about_a_second` of `crates/moqt-io/src/wt.rs` has the same on a real loopback connection: a server that comes 3.2 s after the client began is reached within 1.5 s, and one session is left open at it. With one attempt (the interval set to 100 s for one run of the test) it was reached 3.8 s after it came.

## Not looked at

- A link slow enough that one handshake takes longer than a second, where the attempts overlap.
- The terminal viewer and the Rust client connecting to a relay that comes back: they use the same `connect`, and were not run here.
