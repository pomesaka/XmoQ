# xmoq-again

When a track that ended, or was refused, is asked for again on the same session. No input or output, and no dependencies. The terminal viewer ([`xmoq-watch`](../watch/README.md)), the agent ([`xmoq-agent`](../agent/README.md)), the part they share (`xmoq-viewer-io`), the chat's gatherer ([`xmoq-chat`](../chat/README.md), a participant's `said`) and the transcriber ([`xmoq-transcriber`](../transcriber/README.md)) ask by it.

The rule and its default values are the pages' (`createAskAgain` of `@xmoq/moqt`, `packages/moqt/src/core/again.ts`), all **provisional**; both are tested with the rows of [`fixtures/ask-again/same-session.tsv`](../../fixtures/ask-again/README.md):

| Value | Default | Where |
|---|---|---|
| The pause after an end, and after a refusal that names no time | 1,000 ms | `ASK_AGAIN_MS` |
| The shortest pause after a refusal that names a time; it doubles while such refusals follow each other without a subscription between | 100 ms | `SHORTEST_MS` |
| What the shortest pause grows to | 5,000 ms | `MOST_SHORTEST_MS` |
| The longest pause a refusal is let name | 30,000 ms | `MOST_NAMED_MS` |

A caller may set others (`Pauses`); the gatherer sets the first with `--ask-again-ms`. Why a crate of its own, at the top of `src/lib.rs`.
