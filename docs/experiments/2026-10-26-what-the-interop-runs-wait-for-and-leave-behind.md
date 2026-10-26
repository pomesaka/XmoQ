# What the interop runs wait for, and what they leave behind

- Date: 2026-10-26
- Commit SHA: the change that adds this record and the one before it, on top of `e3930f50`; the first change is in `main` (#452); the first two full runs were on top of `856c6cad`
- Roadmap item: none; the interop run (`interop/matrix.ts`, `interop/harness.ts`, the drivers)

## Objective

A read of the interop drivers named these as sources of failures that are not the code's:

1. The runs that mostly wait started together when the first of them was reached and went on beside the runs after it, among them those that count the frames of a video and the moves of a GOAWAY.
2. Three ways for what a driver started to stay behind: the matrix killed a driver alone 10 s after SIGTERM, a command that could not be started kept the teardown waiting, and `chat-tokens.ts` started its relays outside the `try` whose `finally` stops them.
3. Bounds of time counted from before a page or a process started (`quiet-peer.ts`, `rendezvous.ts`), a `sleep(1000)` to wait for a state of the relay (`relay-one-session-asks-for-much`), the run with a stopped subscriber against the relay's idle timeout of 6 s, and lines of the chat's gatherer read "so far" (`chat-pages.ts`, `chat-tokens.ts`).

This records what was measured to change them.

## Measurements

macOS, arm64, 11 cores; Bun 1.3.14 (the one of `mise.toml`) unless said.

### A driver killed with what it started

A driver that launched Chromium with the options of `harness.ts` (playwright-core 1.64.0) was started in a process group of its own, and the group was sent SIGKILL. Chromium (Google Chrome for Testing) led a group of its own, and its parent was the driver; after the kill its parent was 1, and it and its helpers were still there 1 and 5 seconds later. Nothing ends it when the driver is killed: Playwright's `exitHandler` does not run on SIGKILL, and Chromium does not end when its pipe closes.

`killEverything` of `matrix.ts`, copied as it is, was run on a driver that ignored SIGTERM and had started Chromium and a `sleep` of its own group: 2 seconds after it, no process of the driver's group or of Chromium's was there, and a second call on the groups that were gone did not throw.

### A command that cannot be started

Spawned through `node:child_process` with a listener for `error`, a path that does not exist gave `pid` undefined, the events `error` (ENOENT) and `close`, and no `exit`, under Bun 1.3.12 and 1.3.14; `kill` returned false. `quiet-peer.ts --case no-stream --client own-browser` with `target/debug/xmoq-moqt-client` moved aside ended in 26 s with `FAILED` and its teardown done; its log had `[fence] could not be started (ENOENT: …)`. Its `FAILED` line is the 20 s it waits for the quiet peer's line, and not the client's, which it does not wait for.

### A stopped subscriber against the idle timeouts

`disconnect.ts --scenario slow-subscriber` (`--only "a stopped own-rust subscriber"`) with the relay's `--idle-timeout-ms 30000`, and the driver made to wait before it lets the subscriber go on (a change made for these runs alone, and taken out):

| Stopped for | Pairing | Ended as expected |
|---|---|---|
| 9 s | 13.5 s | yes |
| 9 s | 12.8 s | yes |
| 15 s | 18.8 s | yes |

In the run of 15 s, the steps between the stop and the letting go took 1,383 ms; when it went on, the client printed the `PUBLISH_DONE` with TOO_FAR_BEHIND, and the relay saw its session end with `ClosedByPeer { code: 0 }`. The client's own idle timeout (6 s, `IDLE_TIMEOUT` of `xmoq-moqt-io`) did not end its session.

### Full runs

`mise run interop -- --base-port 37500`, each under the lock of the full run:

| Change | Ended as expected | Seconds | Load at the end |
|---|---|---|---|
| The runs that mostly wait after the others | 246 of 246 | 874.3 | 22.2 |
| And what a driver started not left behind | 246 of 246 | 679.7 | 3.7 |
| And the bounds of time and the waits, on top of `e3930f50` | 246 of 246 | 612.6 | 3.9 |

No `cleanup took` or `could not be started` line was printed.

In the last run, the quiet peer runs closed 1,503 to 2,008 ms after their session began and 9 to 177 ms later counted from before the client started; the viewer beside the one session that asks for much had all its requests passed on, 4 SUBSCRIBEs and 3 FETCHes; own-rust's held SUBSCRIBE was refused 2,007 ms after it was told to send it. The seconds cannot be compared with each other or with the 570.5 s of an earlier run: other work held the machine, and pairings that the changes do not touch took 4 to 15 s longer in the first run than before (`own relay: own-browser to own-rust, then PUBLISH_DONE`: 0.6 s before, 11.0 s). Of the runs that mostly wait, now at the end, the longest took 27.1 s in the first run (18.1 s in the earlier run).

## Not looked at

- Whether the runs that count time fail less often: no run failed before the changes either.
- The leaving runs that still run beside each other at the end, among them two whose bounds count the time a kill is noticed.
- A run on Linux in CI.
