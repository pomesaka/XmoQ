# A file of OPFS read while it is written

- Date: 2026-10-26
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, an agent in a meeting: the interop case of the page that writes for an agent ("three pages ... one writes for an agent and takes a picture its inbox asks for") failed once in CI with `NotFoundError: A requested file or directory could not be found at the time an operation was processed`, on a change that touched the relay's key alone (PR #414, run 40255902385).

## Objective

The bridge writes `events.jsonl` anew every second (`createWritable`, then `close`). Does a `File` that `getFile()` gave before such a `close` fail to be read, and is that the failure of the case?

**Loopback, one machine.**

## Expected

Said before the first run (to the PM, not in a file): a `File` taken before a `close` that moved the swap file over the name points at a backing file that is deleted, and reading it fails with NotFoundError, now and then. Read from Chromium's main on 2026-10-26: `ObfuscatedFileUtil::CopyOrMoveFile` of `storage/browser/file_system/obfuscated_file_util.cc`, the case of a move with overwrite, "Point target entry to source entry's backing file. Delete target entry's old backing file".

## Environment

macOS 14.4, arm64, Chromium of Playwright 1.64.0, headless, a page at `http://localhost`. In its OPFS, one loop appends a line to a file 300 times as `agent-folder.ts` flushes (`createWritable({ keepExistingData: true })`, `seek`, `write`, `close`); another, meanwhile, takes `getFile()` and reads it with `text()`, once with a turn of the event loop between the two and once without. For the fix, `bun interop/matrix.ts --relay own --base-port 31500 --only "one writes for an agent"` ten times.

## Result

**As expected for the reads.** Of the reads with a turn between, 539 were whole and 24 failed with NotFoundError; of those without, 531 and 32. Reading at once does not avoid it.

`readWhole` of `@xmoq/meeting` (`src/web/agent-folder.ts`) takes `getFile()` again after a NotFoundError, three times at most, and lets another error through; the bridge reads `inbox.txt` with it, and the interop page reads `events.jsonl` and the pictures with it. Its rows (`test/agent-folder.test.ts`) fail with the count of reads at 1, and with every DOMException read again.

The ten runs of the case all passed (3 s each). Before the change it failed once in the runs of CI that were seen, and was not run ten times here, so the ten say only that the change breaks nothing; what tells the cause is the count of the reads above.

## Not looked at

- The CI run itself: its log was not read, so which read failed there is inferred from where the page reads a file the bridge writes.
- A folder the person chose (not OPFS): whether a write there replaces the file in the same way was not looked at. `readWhole` is used there as well.
