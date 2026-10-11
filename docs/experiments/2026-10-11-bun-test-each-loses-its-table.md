# `test.each` of Bun 1.3.12 loses its table

Date: 2026-10-11. Checked on `main` at `59d76ab`.

## What happened

The TypeScript job of CI failed twice with the same five tests, on pull request 63 and on pull request 68, which changes documents only. Both passed when run again.

- The tests are the five rows of the first `test.each` in `packages/moqt/test/webtransport/close.test.ts`.
- Each failed with `RangeError: a session is closed with a whole number from 0 to 4294967295, not undefined`: the callback got a row without `code`.
- The names of the tests were right (`closeCode(0) is a code of 32 bits`).
- Runs: 38096733422 (job 114344053777), 38098773661 (job 114350135356).

## Cause

A defect of `bun:test` in Bun 1.3.12, not of XmoQ's code or of its fake timers. The function `test.each(table)` returns does not keep the table where the garbage collector sees it. When a collection runs between `test.each(table)` and the call `("name", callback)` that follows, the table and its rows are freed, and the callback gets whatever is at that address afterwards.

Fixed in Bun 1.3.14: oven-sh/bun pull request 30050, "bun:test: root the .each() table array in the JS wrapper", merged on 2026-05-01 as `812ad3b464` (read with `gh api` on 2026-10-11; the commit is after 1.3.13 and before 1.3.14).

**That the two failures in CI were this defect is concluded from the symptoms agreeing. It was not reproduced on Linux x64.**

## What was run

On macOS arm64, with a copy of `main` outside the repository.

| Run | Bun 1.3.12 | Bun 1.3.14 |
|---|---|---|
| `bun test` in `packages/moqt`, as CI calls it | 0 of 200 failed | not run that often |
| The same with `BUN_JSC_slowPathAllocsBetweenGCs=50`, which makes collections frequent | 10 of 60 failed | 0 of 40 |
| A file with three `test.each` and nothing of XmoQ, the same option | fails | 0 of 120 over collection intervals 1 to 30 |
| A file that keeps what `test.each(table)` returned, calls `Bun.gc(true)`, then calls it; no option | 20 of 20 failed | 0 of 20 |

- In one of the ten failures the five tests of `close.test.ts` failed as in CI: the names right, and `closeCode` called with the values of the rows of the **second** `test.each` of the file (4294967296, -1, 1.5, NaN, Infinity).
- In the others a `test.each` of `core/fetch.test.ts` or `core/bytes.test.ts` failed in the same way, or Bun ended with a segmentation fault.
- The name is right because it is formatted at once, while the freed memory still has its content; the test runs later.

## What was ruled out

- Fake timers or mocks leaking from another file: the file without any reproduces it; `first-bytes.test.ts` restores its timers and `write-abort.test.ts` runs after `close.test.ts`.
- The order of the files: the same in the two failed runs and in five that passed, as are the Bun version, the number of tests (748 in 16 files) and the time taken.
- A test changing the table: it is an array literal nothing else can reach.

## Not known

- Why it is always the first `test.each` of `close.test.ts` in CI. The defect is not about that place: other tables failed in the runs above. The guess is that the allocations up to there cross a threshold of the collector in CI; not checked.
- Why it began after pull requests 61 to 63. The guess is that more tests moved where a collection falls.
- Why it never happened on macOS without the option.

## What was done

`mise.toml` pins Bun 1.3.14.

WHY NOT keep 1.3.12 and assign every table to a variable first: that avoids it (0 of 20), and every `test.each` of the repository would need it, also the ones that have not failed, with nothing to hold a new one to it.

WHY NOT run the job again when it fails: a row that was replaced can satisfy the expectation by chance, and the test then passes with arguments it was not written for.

## Limits

- The comments that say what Bun 1.3.12 does (`interop/harness.ts`, `packages/moqt/test/webtransport/fake-transport.ts` and `first-bytes.test.ts`) were written for 1.3.12. That `mise run check` and `mise run interop` pass with 1.3.14 shows the code still works; it does not show that each of those workarounds is still needed.
