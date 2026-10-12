# The pages connect again when the relay comes back

- Date: 2026-10-12
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 1, "Test late join, keyframes, reconnect, cancellation and slow subscribers"

## Objective

The demo's pages connected again after a break, with code of their own and in no test. That code is now `keepPublishing` and `keepPlaying` of `@xmoq/media`. Do a publisher and a viewer made with them go on when the relay is stopped and started anew?

Expected, written before the first run: both pages are told that their session ended; each fails to connect at least once while the relay is gone; after the relay is back the publisher announces on a second session and the viewer plays on a second session, 25 frames or more, each with a number after the one before it.

## Environment

| Item | Value |
|---|---|
| OS, machine | macOS 14.4, arm64, loopback |
| Browser | Chromium 156.0.8078.4 (playwright-core 1.64.0), headless |
| Relay | own, stopped with SIGINT and started anew on the same port with the same certificate |
| Pages | `interop/page.ts`: `keepVideoPublished` (320x180 bars, 25 frames a second, a key frame a second) and `keepVideoPlayed` (`keepPlaying`, `playVideoTrack`, `createPlayout`); a pause of 500 ms between tries |

## Commands

```sh
bun interop/video.ts --video reconnect --port 45310
mise run interop -- --base-port 45300   # the pairing "the relay is stopped and started anew, ..."
```

## Result

Passed as expected, 8 runs of 8 alone and once in the whole interop run.

- The viewer showed 26 or 27 frames on the first session and 25 on the second when the run looked (it stops there), each after the one before it.
- The publisher failed to connect 1 to 8 times while the relay was gone, the viewer 1 to 4 times.
- **After the relay was back, the viewer's first try was refused in most runs** (`REQUEST_ERROR 0x10`): it asked before the publisher had announced. It tried again and played.
- **Not as expected: the viewer was told late that its session ended, in one run whose log was read**: about 4 seconds after the publisher was, which was told at once and failed 8 times meanwhile. Another run of four had 7 such failures and was not read. Not looked into; the relay may go without closing every session, and the page then waits for its idle timeout.
- The demo's pages, with the same parts, through `spikes/lossy-link` cut for 12 s: both said "the session ended" after 6 s, and played again (30 frames and 50 pieces of sound a second) 4 s after the link was back.

## Not done

- A link that breaks without the relay going was run with the demo alone, not in the interop run.
- Sound over a break in the interop run; in the demo the first piece after the break began apart from the one before, with no break counted.
- A viewer whose publisher alone goes and comes back.
