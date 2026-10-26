# A slow teardown, and a decoder behind on mlmpub's video

- Date: 2026-10-26
- Commit SHA: the commits that add this record and the change before it, on their working copy on top of `8fa6356f`; the measurements of the teardown on `7d51b4c8`
- Roadmap item: none; the interop run (`interop/harness.ts`, `interop/video.ts --video from-mlmpub`)

## Objective

Two things seen in full interop runs on this machine, on 2026-10-26:

1. The pairing `own relay: fetch: own-browser asks for a Group of 8.3 MB …` took 42 to 55 s while another full run held the machine, about 30 s of it after the last step of its driver ([record](2026-10-26-a-large-group-cut-off-before-it-is-fetched.md)). Where did the 30 s go?
2. `no relay: video: own-browser plays a video track of moqlivemock's publisher` failed once in a full run of another working copy with `frame 13 is 520000 microseconds after the one before it, not 40000`, and passed alone.

## The teardown

### Runs

The driver printed the time of each cleanup, of the start and the end of the teardown, and of its own exit (a change made for these runs alone, and taken out). `mise run interop -- --base-port 37500 --only "asks for a Group of 8.3 MB"` on `7d51b4c8`, macOS, arm64, 11 cores:

| How the machine was | Runs | Pairing | Longest cleanup |
|---|---|---|---|
| Nothing else running | 2 | 3.6 s and a failure (below) | 169 ms, closing Chromium |
| 11 busy loops of the shell, one a core, under the lock of the full run | 6 | 3.0 to 4.6 s | 269 ms, closing Chromium |
| Beside a full interop run of another working copy (`--base-port 36000`) | 3 | 4.4 to 6.9 s | 219 ms, closing Chromium |

The driver exited within 1 ms of the end of its teardown in every run. The 30 s did not come again. The first of the two runs alone failed: it was on `main` before the change of [the large Group](2026-10-26-a-large-group-cut-off-before-it-is-fetched.md), and its log was written over by the next run.

### What the earlier runs show

In the runs of 42 to 55 s, the own Rust client's session ended at the relay with `Lost("connection error: timed out")` before the relay's `GOING_AWAY`. The relay's idle timeout is 6 s (`IDLE_TIMEOUT` of `xmoq-moqt-io`), and the teardown stops the own Rust client first and the relay last, with Chromium's close and the server of the pages between them: something between took 6 s or more. `browser.close()` had no limit. That it was the one is a guess: those runs had no time for each cleanup.

### The change

- Each cleanup has a name (`stop <label>`, `close Chromium`, `stop the server of the pages`), and one that takes more than a second is printed with its time (`cleanup took N ms: …`).
- Chromium's close has the limit of a step (`STEP_TIMEOUT_MS`). Where it is met, the cleanup is printed as failed and the teardown goes on to stop the relay. Chromium is not left behind: playwright-core 1.64.0 kills the process group of each browser it launched when the driver exits (`exitHandler` in `lib/coreBundle.js`, which `handleSIGINT: false` leaves in place). Under Bun 1.3.14 a driver that launched Chromium and exited without closing it left no process of it (its child's PID was gone a second later). Playwright gives the driver no PID of the browser, so the driver does not kill it itself.

## mlmpub's video

### What the failed run shows

From the output of that run (sent by the session that ran it):

- `"dropped":{"decoder-behind":1,"late":11}`, `"decoded":50,"submitted":50`, no decoder failure.
- `mlmpub` wrote each Group whole, 25 Objects: `writing LOC group … objects=25`. Its `failed to write LOC object … object=13` came after the driver's report, when the run was being stopped.
- Frame 12 is 480 ms after the start of its Group, and frame 13 came 520 ms after it: the first frame of the next Group. 12 frames were passed over, as many as were dropped.

### Why

`playVideoTrack` gives up the rest of a Group when its decoder is behind (more than `maxDecodeQueue` chunks wait in it): what was there is dropped as `decoder-behind`, what comes after as `late`, and it goes on from the next key frame (`abandonGroup` of the video receiver). The page decoded 1280x720 slower than it came, in a full run: the machine's speed. The run of a page's video already lets these two through, for that reason; the run of mlmpub's video let nothing but `no-key-frame` through, and asked for every frame 40 ms after the one before.

### The change

The run of mlmpub's video lets `decoder-behind` and `late` through. A frame is 40 ms after the one before it, or, where frames were passed over, the first of a later Group (a time of a whole number of seconds); the frames passed over are no more than those dropped as `decoder-behind` and `late` (fewer can be dropped than passed over where a Group was being given up when the page reported). Any other drop, a decoder failure, a first frame that is not the first of a Group, and fewer than 50 frames still fail the run.

### Runs

`mise run interop -- --base-port 37500 --only …`, after the change:

- `no relay: video: own-browser plays a video track of moqlivemock's publisher`: 5 runs, 5 ended as expected, none with a frame passed over.
- `no relay:`: 36 of 36 ended as expected.
- The decoder was made to fall behind by setting `DEFAULT_MAX_DECODE_QUEUE` of `play-video-track.ts` to 1 for 4 runs (taken back): nothing was dropped in any of them, as `mlmpub` sends at the speed of the video. So the check was given reports made in the driver for 3 runs (a change taken out afterwards), with the 50 frames of the run's report, at the times of the failed run: frames 0 to 12 of a Group and 37 from the next Group on with `decoder-behind` 1 and `late` 11 ended as expected; the same with nothing dropped failed (`12 frames were passed over, and 0 dropped for a decoder that was behind`); a jump of 320 ms, into the middle of a Group, failed (`… not 40000, nor the first of a later Group`).

## Not looked at

- Where the 30 s of the earlier runs went: they did not come again.
- Whether the change of `main` between `49e30200` and `7d51b4c8` has anything to do with that.
- A decoder that falls behind in a real run of mlmpub's video, after the change: it was not made to happen.
