# Guide for implementation agents

Read README, requirements, architecture, protocols, roadmap, validation and open questions before coding.

1. Do not invent product requirements, performance targets or policy.
2. Distinguish **discussed**, **proposed**, **undecided**, **validation required**, and **implemented**.
3. Start with browser ↔ Rust MoQ Object interoperability before media features.
4. Pin and record exact dependency/protocol versions. Library names alone do not prove compatibility.
5. Prefer small vertical slices with tests and reproducible commands.
6. Explicitly handle backpressure, slow subscribers, bounded buffers, cancellation and cleanup.
7. Do not silently decide authentication, logging, retention, deployment or license.
8. Never claim tests passed unless actually run.

Each change should report objective, changed files, verification commands, actual results, blockers and open questions.

Current assignment: Phase 0b in the [roadmap](roadmap.md), the own MoQ Transport implementation pinned to draft 18. Implement from the draft text, cite the section for non-obvious behaviour, and run every own component against a `moqtail` peer before pairing it with another own component. Do not build media features before this gate.
