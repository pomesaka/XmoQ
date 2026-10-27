# Claude Code takes part in a meeting, in Chrome

- Date: 2026-10-27
- Commit SHA: the commit that adds this record; run on `main` with #553 to #556 in it
- Roadmap item: Phase 7, an agent in a meeting: an agent that takes part by reading `AGENT.md` of the folder of a reader, from the page of a meeting and from `xmoq-agent`.

## Objective

Does Claude Code take part in a meeting of the demo, with no skill of its own, from the line to paste alone: from the folder a page of a meeting writes, and from the `--out` of `xmoq-agent`? Do the parts that were not run before in a real browser hold there: a folder the person chose (not OPFS), the loop of `AGENT.md` with the Monitor tool, the first and the last line, the mark of the agent's own lines?

**One machine, a person in the meeting.** Run by the PM session and the person; this record is written from the PM's account of what was seen, by another session.

## Environment

macOS, Chrome (its version was not recorded), the demo (`bun demo/serve.ts`) with the rooms at `http://127.0.0.1:8787`, the room `demo`. Claude Code with the Monitor tool. The page's folders `~/Documents/xmoq/demo/55aeeb4041ff3c7f` and `~/Documents/xmoq/demo/56935f355397c25f`. The transcriber of the demo with `--transcribe-language auto`, then `ja`.

## Observed results

**The page of a meeting.** With the line the page showed, Claude Code read `AGENT.md`, ran its loop with the Monitor tool, and took part:

- The files of `events/` were listed and read; no temporary file of Chrome was seen among them.
- The first line, `reader` `began`, gave the room and the page's id.
- A line added to `inbox.txt` was in the chat about a second later, and came back with `"by":"agent"`.
- After **stop**, `reader` `ended` came.

**`xmoq-agent`** (`--room http://127.0.0.1:8787/meetings/demo`): it wrote the 28 lines of the chat said before it came, and `reader` `began` without an id; it said lines under the name `agent`, which came back with `"by":"agent"`; the line to paste was on its stderr.

**The transcript.** With `auto`, two tabs on one machine took the same microphone, and the same voice was read as Japanese, English and Korean, a piece in each. With `ja` it was read as Japanese; short words were misheard (「クロワッサン」 as 「黒ワスさん」).

**What held the agent back.**

- While the PM session was working, it did not read the notices of the Monitor tool, and it answered late.
- A subagent that waited was not woken by the notices of the Monitor tool; once it wrote an answer late.
- Before running a command that a line of the meeting asked for, the agent asked the person on the PM's screen, as `AGENT.md` says.

## Limitations and blockers

Not run:

- Mute with a real microphone, and the transcript of a muted page.
- The transcriber that leaves out a piece of marks in brackets alone (#556), in a meeting.

## Open questions raised

None.
