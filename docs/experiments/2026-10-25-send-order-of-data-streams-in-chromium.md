# Whether Chromium gives freed connection credit to a stream by its `sendOrder`

- Date: 2026-10-25
- Commit SHA: the commit that gives `@xmoq/moqt`'s data streams `sendOrder` -1 together with this record, on top of `f2922637`
- Roadmap item: none closed. A known gap of the implementation notes: connection flow control for control streams first (§11.4.2, MUST)

## Objective

`@xmoq/moqt` now opens its data streams (subgroup and fetch streams) with `sendOrder` -1, and its control stream and request streams with the default, 0, so that the control streams come first where the browser is the sender (§11.4.2: "endpoints MUST allocate connection flow control to the control streams before allocating it to any data streams"). Find out whether Chromium hands the connection credit the peer frees to a stream in the order of `sendOrder`.

## Environment

| Item | Value |
|---|---|
| Host | macOS 14.4, arm64, loopback only |
| Browser | Chromium 156.0.8078.4, new headless, launched by `playwright-core` 1.64.0 with `channel: "chromium"` |
| Server | `spikes/send-order-in-chromium`, on `web-transport-quinn` 0.13.2, debug build, rustc 1.99.0. Connection receive window 256 KiB, stream receive window 64 KiB. It reads the client's unidirectional streams at 512 KiB a second together, 4 KiB at a time, round robin, and a bidirectional stream at once |
| Driver | Bun 1.3.12; the certificate of an interop run (`interop/.cert`) |

The page talks WebTransport directly, without `@xmoq/moqt`: what is measured is the browser. That `@xmoq/moqt` passes `sendOrder` to `createUnidirectionalStream` is held by a test (`the control stream and a data stream are opened with their send orders` in `packages/moqt`).

## Commands

```sh
(cd spikes/send-order-in-chromium && cargo build)
bun spikes/send-order-in-chromium/run.ts --rounds 5
```

Run twice. In each round the page opens 16 unidirectional streams and writes 16 KiB to each again and again, which is more than the connection window, so the page waits for credit that the server frees as it reads. After 1.5 seconds it opens a bidirectional stream and writes 64 KiB on it. The server prints when it has all 64 KiB; the driver takes the time from the write to that. Four modes, each a new session in a new page: the data streams with `sendOrder` -1 and the request with none; neither with one; data -1 and request 1; data 0 and request 1000. The page waits 6 seconds for the server; a request that took longer is `NaN` below.

## Observed results

The time from the write of the request to the server having all of it, in milliseconds, sorted:

| Mode | First run | Second run |
|---|---|---|
| data -1, request none | 3232, 3433, 3508, 3693, 3790 | 2587, 3105, 3177, 3723, more than 6 s |
| data none, request none | 3768, 3839, 3839, 5681, more than 6 s | 3267, 3287, 3897, 3929, more than 6 s |
| data -1, request 1 | 3541, 3595, 3912, 3969, more than 6 s | 3001, 4224, more than 6 s three times |
| data 0, request 1000 | 3223, 3715, 3747, 4016, 4232 | 2740, 2995, 3443, 3477, more than 6 s |

**No mode got the request through sooner than another.** Had the request been given the credit first, its 64 KiB would have taken about 125 ms of what the server frees at 512 KiB a second. Shared evenly with the 16 data streams it would take about 17 times that, 2.1 seconds; it took 3 to 4 seconds or more in every mode.

## Limitations and blockers

- Why `sendOrder` has no effect here is not known. Chromium may not order by `sendOrder` what it sends when connection credit comes, or the page's writes may sit in buffers of the browser that take the credit before the request is written. Neither was looked at in Chromium's code, and no capture was taken.
- One shape of traffic, on loopback, with a server that frees credit at a fixed rate. A peer that frees it otherwise was not tried.
- `@xmoq/moqt` keeps `sendOrder` -1 on its data streams all the same: it costs nothing, it is what §11.4.2 asks of a sender, and another browser or version may follow it.

## Open questions raised

None new. Whether the client should end a subscription its application does not read, which is what frees the credit of the browser as a receiver, is in [open questions](../open-questions.md).
