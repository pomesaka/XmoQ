# A viewer of an edge when the origin goes

- Date: 2026-10-25
- Commit SHA: the commit that adds this record (the relay at `main` `f2cb34cf`, with the edge's first three steps; no `--auth-key-hex`)
- Roadmap item: Phase 7, relay scale; how long a viewer of an edge loses its picture when the relay above goes and comes back ([design draft](../scale-and-meetings.md), losing the origin)

## Objective

A page publishes at an origin with `keepPublishing`, and another plays at an edge below it with `keepPlaying`. The origin is stopped, kept down for a while, and started anew on its port. For how long does the viewer's picture stand still, and where does the time go? Beside it, the same with the viewer at the origin itself, as the interop run `reconnect` has it.

**Loopback, one machine**: the relays, the browser and the driver share one Mac.

## Environment

Apple M3 Pro, 11 cores, 18 GiB, macOS 14.4; Chromium of Playwright; the own relays (`xmoq-relay/0.0.0`, debug build, as in the interop run) on 34500 (origin) and 35500 (edge, `--upstream`); the bars of `interop/video.ts` (320x180, 25 frames a second, a key frame a second, a Group on one stream); the pages wait 500 ms before they connect again (`KEPT_DELAY_MS` of `interop/xmoq-page.ts`). The full interop's lock was held for all the runs (10:59 to 11:04).

```
bun spikes/edge-stall/run.ts --through edge|origin --stop sigint|sigkill --down-ms 0|3000
```

The stop is SIGINT, at which the own relay sends GOAWAY without a URI and closes its sessions (the interop run gives it a GOAWAY Timeout of 0), or SIGKILL, at which it vanishes. The stall is the longest time between two frames the viewer drew (a time stamped by the page at each frame, `shown[].at`), from a second before the stop on. The times of the relays' lines and of the pages' states are when the driver saw them (the pages asked every 50 ms), in milliseconds from the stop.

## Result

Three runs of each case; the stall in milliseconds:

| Stop | Down | Viewer at the edge | Viewer at the origin |
|---|---|---|---|
| SIGINT | 0 s | 1,084 / 1,086 / 1,104 | 352 / 399 / 915 |
| SIGINT | 3 s | 7,090 / 7,153 / 7,069 | 3,345 / 4,532 / 3,885 |
| SIGKILL | 0 s | 6,556 / 6,568 / 6,569 | 6,086 / 6,097 / 5,526 |
| SIGKILL | 3 s | 6,520 / 6,586 / 6,608 | 5,572 / 5,572 / 5,575 |

Where the time went, from the lines of the edge and the states of the pages:

- **SIGINT: the edge connects at once, and its one attempt to connect waits on QUIC's retransmissions.** The edge printed CONNECT_UPSTREAM 2 to 9 ms after the stop (the GOAWAY without a URI names the same place, §10.4) and UPSTREAM_CONNECTED at 1,011 to 1,018 ms with the origin back at 114 to 124 ms, and at 7,009 to 7,012 ms with it back at 3,112 to 3,123 ms: **3.9 s after the origin was there again**. The attempt had begun while nothing listened on the port, and its handshake was sent again only after growing waits; it did not run into the 10 s of a connect (no UPSTREAM_FAILED). The viewer was told the track ended at about 60 ms, connected to the edge again 500 ms later and played as soon as the edge had its session. The pages that connect to the origin itself try again every 500 ms or so, each attempt failing at once (`failed` right after `connecting`) while the port is closed, and played 0.2 to 1.4 s after it was open again.
- **SIGKILL: QUIC's idle timeout.** Nothing tells anybody that the origin went: the edge printed the end of its session upstream at 5,966 to 6,003 ms, the publishing page found its session ended at about 5,000 ms in four of the six runs, and the viewer at the origin at about 5,000 ms (the relay's idle timeout is 6 s, and QUIC takes the smaller of the two ends). Through the edge the viewer comes about 0.5 to 1 s later than at the origin: it learns of the end from the edge after the edge has, and waits its 500 ms before it asks again. The time down (0 or 3 s) adds nothing, since it is within the idle timeout.
- In 2 of the 6 SIGKILL runs at the origin the publishing page found its session ended at about 60 ms, and connected again before the viewer noticed; nothing was looked at to say why.

## Not looked at

- The edge's own GOAWAY and a GOAWAY that names another relay: the relay's test on in-memory connections has it.
- Release builds, two machines, real links; xmoq-watch or the Rust client as the viewer (each a client of `xmoq-moqt-io`, whose connect is the edge's).
- Why the browser's attempts fail at once on a closed port where `quinn`'s retries wait.
