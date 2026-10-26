# Guide for implementation agents

The rules of work are in [`CLAUDE.md`](../CLAUDE.md), in Japanese: what to read before coding, undecided items, the draft and `moqtail`, bounds and cleanup, checks and reviews. This guide has the ones it does not.

1. Pin and record exact dependency and protocol versions. Library names alone do not prove compatibility.
2. Prefer small vertical slices with tests and reproducible commands.
3. Before implementing a part of the draft, list what the draft requires for that part (the MUST and SHOULD statements, with section numbers) and what a long-running server has to guarantee (bounds, cleanup, isolation between sessions). Write tests for the abnormal paths from that list, not only for a successful transfer. Reviews of the first implementations found most of their issues there.
4. Report each change with its objective, changed files, verification commands, actual results, blockers and open questions.
