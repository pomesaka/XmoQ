# Guide for implementation agents

Before coding, read the [requirements](requirements.md), the [architecture](architecture.md), the [open questions](open-questions.md), and for the part you touch its README and the rows of the [implementation notes](moqt-implementation-notes.md). What to work on next is the [roadmap](roadmap.md)'s or the issue's, not this guide's.

1. Do not invent product requirements, performance targets or policy.
2. Distinguish **discussed**, **proposed**, **undecided**, **validation required**, and **implemented**, as the [requirements](requirements.md) define them.
3. Pin and record exact dependency/protocol versions. Library names alone do not prove compatibility.
4. Prefer small vertical slices with tests and reproducible commands.
5. Explicitly handle backpressure, slow subscribers, bounded buffers, cancellation and cleanup.
6. Do not silently decide authentication, logging, retention, deployment or license.
7. Never claim tests passed unless actually run.

Each change should report objective, changed files, verification commands, actual results, blockers and open questions.

Implement from the draft text, cite the section for non-obvious behaviour, and run every own component against a `moqtail` peer before pairing it with another own component.

Before implementing a part of the draft, list what the draft requires for that part (the MUST and SHOULD statements, with section numbers) and what a long-running server has to guarantee (bounds, cleanup, isolation between sessions). Write tests for the abnormal paths from that list, not only for a successful transfer. Reviews of the first implementations found most of their issues there.
