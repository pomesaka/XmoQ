# Pages without a token, at a relay that checks SETUP: what they are told

- Date: 2026-10-11
- Commit SHA: the commit that adds this record (the relay as of #127)
- Roadmap item: Phase 1, authorization (an experiment; how the relay authorizes is undecided)

## Objective

Since #125 the relay holds the streams of a session it closed, so that the peer sees the close first; since #127 a relay with a key checks SETUP by default. Does a page of the demo without a token say "not authorized" every time? Before, the answer came in four forms.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay, rooms | own, `bun demo/serve.ts --auth` (the flag `--auth-setup` of the demo is gone: the relay's default does it) |

## Commands

```sh
bun demo/serve.ts --auth --port 45381 --relay-port 45382 --rooms-port 45384
# a script of this run: /publish and /watch with a token; then 10 times each /watch?token=no and /publish?token=no, the first line of the page after 1.8 s
mise run check
mise run interop -- --base-port 45300
```

## Observed results

Expected: all say that the relay closed the session with code 2.

| Page without a token | As #127 was | With the change below |
|---|---|---|
| Watching, 10 runs (20 with the change) | 10 of 10: closed, code 2, reason `authorization` | 20 of 20 the same |
| Publishing, 10 runs (20 with the change) | **6 of 10** the same; **4 of 10** `NetworkError: The stream was aborted by the remote server` | 20 of 20: closed, code 2 |

- Pages with a token published and played (30 frames a second, the sound decoded).
- **Why the 4**: the page writes PUBLISH_NAMESPACE as the relay closes. The write fails with a `DOMException` named NetworkError that does not say it is the session's (no `source`), and `closed` of the WebTransport settles 0.3 ms after it (10 runs with a probe in the page: 3 that way, 7 where the session's end was known first). The library gave the request the error of the write.
- **Changed in `@xmoq/moqt`**: a request whose write fails waits one turn of the timers for the session's end, and fails with `SessionClosedError` where the session ended. The same wait was there already for the writes of a publisher.
- **No test holds this**: with the fake transport the session's end is known before the write fails, and a test written for it passed without the change as well. It was taken out again.

## Limitations and blockers

- Chromium only, loopback only. One turn of the timers was enough for 0.3 ms here; on another machine it may not be.
- The watching page was right before the change because it asks later (after the room's answer), not because its path differs.
