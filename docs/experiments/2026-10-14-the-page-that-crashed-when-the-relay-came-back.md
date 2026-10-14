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

## The run says which page crashed, and why

So that the next crash in CI leaves more: the driver asks each page for its report under the page's name (`interop/video.ts`, `reportOf`), every page of the interop run says so in the log when its process is gone (`interop/harness.ts`, the `crash` event of Playwright), and the matrix runs this pairing alone with `DEBUG=pw:browser`, with which playwright-core passes on what Chromium prints to stderr (`interop/matrix.ts`). With `chrome://crash` on macOS, Chromium printed `Intentionally crashing`, `Received signal 11 SEGV_ACCERR` and a stack of addresses there, with no flag; nothing of it reached the run's log before.

Tried here by crashing a page while the pairing ran: its renderer process is sent a signal by its PID once the viewer plays on its first session.

Expected, written before the runs:

- **SIGSEGV to the publisher's renderer**: the log has `[publisher] CRASHED: the page's process is gone`, the driver's last line is `FAILED reconnect video: the publisher's report: evaluate: Target crashed`, and Chromium's lines under `pw:browser` have `Received signal 11`.
- **SIGSEGV to the viewer's renderer**: the same with `viewer`.
- **SIGKILL to the publisher's renderer**: the same two lines of the run; no `Received signal` (a process that is killed prints nothing).

Result: as expected. The renderers of a run's Chromium are five; which is which page was found by sending SIGSEGV to each in turn, in five runs. Three of them belong to no page: the run passed. One was the publisher's and one the viewer's, and three things agreed each time: the `CRASHED` line, the name in the driver's last line, and the page whose lines stopped (after the publisher crashed the viewer printed on).

| Signal, renderer | `CRASHED` line | The driver's last line | `Received signal 11` under `pw:browser` |
|---|---|---|---|
| SIGSEGV, the publisher's | `[publisher]` | `FAILED reconnect video: the publisher's report: evaluate: Target crashed` | yes |
| SIGSEGV, the viewer's | `[viewer]` | `FAILED reconnect video: the viewer's report: evaluate: Target crashed` | yes |
| SIGKILL, the publisher's | `[publisher]` | the publisher's, as above | no |
| SIGSEGV, any of the other three | none | `SUMMARY`, the run passed | yes |

`bun interop/matrix.ts --relay own --only "the relay is stopped and started anew"` passed, with 32 lines of `pw:browser` in the pairing's log on macOS. A pairing that fails has its log printed into the job's log by the matrix, so a crash in CI leaves these lines there. **Not seen**: whether Chromium on Linux, as playwright-core installs it in CI, prints `Received signal` too.

## The relay's changes, read (not run)

The relay's changes merged on 2026-10-12 before the failures are #224 and #225 (what the relay's fetch streams keep beyond its cache is counted and bounded), in the code of both failed runs, and #227 and #228 (timers of a session), only in the second; so #227 and #228 cannot be the cause of the first. In this pairing the relay takes part in one FETCH before it stops: the viewer's Joining FETCH, which it hands to the publisher, which refuses it (`REQUEST_ERROR 0x3` in both logs); no fetch stream is opened, and so nothing that #224 and #225 count. When it stops, the relay closes every session and sends PUBLISH_DONE, which those changes do not touch. And all four are in both sides of the comparison: `test/without-226` is `main` without #226 alone, so they cannot make the difference between 2 of 14 and 0 of 14. That difference is small: if the 2 failures of the 28 runs had nothing to do with the side, both would fall on the side of #226 about once in four (Fisher's exact test, one-sided: 91 of 378, 0.24). The reading that fits the logs is a crash of Chromium's renderer on Linux in a page that is writing a stream when its session is closed (the publisher writes a frame every 33 ms; the viewer only reads). Not checked: whether this pairing ever crashed in CI before #224.
