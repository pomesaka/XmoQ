# The page that crashed when the relay came back: with #226 and without it

- Date: 2026-10-14
- Commit SHA: the commit that adds this record
- Roadmap item: none; a failure of the interop run in CI. No code is changed.

## Objective

In CI, the pairing "own relay: video: the relay is stopped and started anew, and the pages that publish and play connect again and go on" (`interop/video.ts --video reconnect`) failed with `Target crashed` 2 times in 14 with the code of #226 (a sender can stamp two tracks by one clock) and 0 times in about 14 without it. That difference can come by chance. Here the pairing runs alone, 100 times on each side, alternately, on one machine.

- (a) `main` at `fc9b7ce`.
- (b) `main` without #226: `test/without-226`, `44b4a27` (PR #232). It differs from (a) in `packages/media` (`src/core/stamp.ts`, `src/index.ts`, `src/web/audio-track.ts`, `src/web/encode-video-track.ts`, a test) and in `demo/`, which the pairing does not load.

## Expected, written before the runs

Reading the diff found no way for #226 to crash a page in this pairing: it has video alone, the encoder is made once, and under the default clock `"first"` the time is read once and every timestamp is the same as before. So: **neither side crashes**, or both do now and then at about the same rate. If (a) alone crashes, #226 is the cause after all. A run takes about 5 s here (CI takes longer; the machine here has more CPU and memory), so 200 runs take about 20 minutes.

## Environment

macOS 14.4, arm64, loopback; Chromium of playwright-core as `bun.lock` pins it, as the interop run launches it (`interop/harness.ts`); the own relay, debug build, from the same Rust code on both sides. The two sides are copies of the two commits (`git archive`) sharing one `target/`. A script runs `bun video.ts --video reconnect --port 45300` in (a), then in (b), 100 rounds, with a limit of 240 s a run, and keeps every log.

## Result

| Runs | (a) with #226 | (b) without | Seconds a run |
|---|---|---|---|
| One at a time, alternately, 100 each | 0 failed | 0 failed | 4.0 to 5.6 |
| Four at once on four ports (45310 to 45313), alternately, 100 each | 0 failed | 0 failed | 4.1 to 6.9 |

**No run crashed here, with #226 or without it** (400 runs). The expectation held, and the difference in CI is not explained.

What the two runs that failed in CI show (job logs 115024210327, run 38313395520 attempt 1, the branch of #226; and 115072205706, run 38328360299 attempt 1, a branch on `main` after #226; both on `ubuntu-24.04`):

- **It was the publishing page both times.** The relay stopped; the viewer printed `session-ended`, tried again and failed twice or three times (`QUIC_PACKET_WRITE_ERROR`, write error -102, where macOS says `ERR_CONNECTION_REFUSED`). The publisher printed nothing after `publishing 1`: not `session-ended`, which it prints first here in every run. The next `evaluate` of the driver, in the wait for both pages to fail to connect, failed with `Target crashed`. Which page that `evaluate` went to is not in the log (the two are awaited together); a publisher silent from the moment its session ended points to it.
- The other pairings with an encoding page in the same jobs passed, and the pairing passed in the later attempts of both runs.
- Chromium's own log of the crash is not in the job log (the run keeps no crash dump), so the reason is not known.

Not done: a run on Linux. Here the page that publishes with #226 differs from the one without in this: with the default clock, `createStamp("first")` is made once, makes one `createFastestWay` that is never used, and adds what the first frame says once; a time is `Math.round` of the same sum as before. Nothing of it runs when a session ends.
