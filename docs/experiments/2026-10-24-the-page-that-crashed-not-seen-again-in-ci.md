# The page that crashed when the relay came back: not seen again in CI

- Date: 2026-10-24
- Commit SHA: the commit that adds this record
- Roadmap item: the interop run; a follow-up of [the page that crashed when the relay came back](2026-10-14-the-page-that-crashed-when-the-relay-came-back.md).

## Objective

Twice in CI (jobs 115024210327 and 115072205706, ubuntu-24.04) a page of the pairing `own relay: video: the relay is stopped and started anew ...` crashed (`Target crashed`). Since #238 (merged 2026-10-14) the run names the page whose process is gone (`CRASHED: the page's process is gone`) and runs that pairing with `DEBUG=pw:browser`, so that Chromium's lines are kept. This looks for a crash in every CI run of the interop job since then.

## Expected, written before the logs were read

None or few crashes: 400 runs of the pairing on macOS had none (the record before), and CI had two before #238.

## Environment

GitHub Actions, `.github/workflows/ci.yml`, the job `Interop (own and moqtail clients through the moqtail relay)` on ubuntu-24.04. Every run of the workflow from 2026-10-14T14:01Z (the run of #238's own branch, 38576824178) to 2026-10-24T04:02Z (40060746103): 30 runs, read with `gh run list` and `gh run view --job <id> --log` on 2026-10-24. The log of each was searched for `CRASHED`, `Target crashed` and `Received signal`, and for the line of the pairing.

## Result

| Runs of the workflow | The interop job ran the matrix | Skipped (nothing under `crates/`, `packages/`, `interop/`, `fixtures/` changed) | Cancelled or running |
|---|---|---|---|
| 30 | 23 | 4 | 3 |

- **No crash in the 23**: no `CRASHED`, `Target crashed` or `Received signal` line; every one ended `N of N pairings ended as expected` (167 to 172 pairings), and the pairing that restarts the relay passed in each (23 of 23).
- How many runs of the job came before #238 was not counted here, so how often the crash came then is not known; whether something changed since or the 23 were lucky cannot be told from these numbers.
- `pw:browser` lines are not in a job's log when the pairing passes: the matrix prints a pairing's log only when it fails. So whether Chromium on Linux prints `Received signal` for a crashed renderer is still not seen.
- Nothing was changed: with no crash there is no log to find a cause in, and nothing in the waits or the browser's flags to mend by.
