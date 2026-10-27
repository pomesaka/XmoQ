# tail -F of a file written anew, and the loop of AGENT.md

- Date: 2026-10-27
- Commit SHA: the commit that adds this record
- Roadmap item: Phase 7, an agent in a meeting: a guide in the folder of a reader, which any agent reads to take part.

## Objective

The page of a meeting wrote `events.jsonl` anew every second (a copy of it with the new lines, moved over it). Does `tail -F` of macOS print the file again from its beginning at each such write, so that an agent that follows it with `tail -F` reads each line many times? And does the loop of `fixtures/agent-lines/AGENT.md`, which reads the files of `events/` that took its place, read each file once, in order, in the shells an agent may run it in?

**One machine, files only.**

## Expected

Said before the run (to the PM, not in a file): `tail -F` follows the name, so it opens the new file at each move, and was not known to begin it at its end.

## Environment

macOS 14.4, `/usr/bin/tail` of macOS; `/bin/dash`, `/bin/bash` (GNU bash 3.2.57) and `/bin/zsh` (zsh 5.9) of macOS. GNU `tail` is not on the machine.

## Commands

The file written as the page wrote it: a line `line1`, `tail -F` on it, then three times, 2 s apart, `cp` of it to a swap file, one line added there, and `mv` of the swap file over it.

The loop of `AGENT.md`, its rounds counted to 4, in a folder where `events/` has `1700000000001.jsonl` (`a`), `1700000000002.jsonl` (`b`), an empty `1700000000003.jsonl`, `1700000000004.jsonl` (`d`), `.1700000000009.jsonl.tmp`, `1700000000008.jsonl.crswap` and `notes.jsonl`; 1.5 s in, `1700000000003.jsonl` gets `c` and `1700000000005.jsonl` (`e`) comes. Once with each shell. The first form of the loop went over `events/*.jsonl`, and was run in zsh with `events/` empty.

## Observed results

**Not as expected: `tail -F` printed the whole file again at each move.** Its output was `line1`, then `line1 line2`, `line1 line2 line3`, `line1` to `line4`: 10 lines for 4. An agent that follows the file so would read the meeting again every second.

The loop, in each of the three shells, printed `==> 1700000000001`, `a`, `==> 1700000000002`, `b`, then, after the empty file was written, `3` with `c`, `4` with `d` and `5` with `e`: each file once, in the order of the names, none read while empty, and none of the other names.

The first form of the loop ended in zsh at once with `no matches found: events/*.jsonl` where `events/` had no file yet, as before the reader's first write. The loop of `AGENT.md` lists the folder with `ls` in its place; with `events/` empty it ended its round and went on.

## Limitations and blockers

- The Monitor tool of Claude Code was not run with the loop: which shell it runs a command in, and how long it runs before it stops, were not looked at.
- A folder written by a page in Chromium (not OPFS) was not looked at: the empty file before its lines are moved in, and the name of the swap file, are from the comment at `flush` in `packages/meeting/src/web/agent-folder.ts`, not from a run.

## Open questions raised

None.
