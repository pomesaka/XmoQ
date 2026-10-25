# @xmoq/json

A reader of JSON (RFC 8259) for the payloads of the tracks that Rust writes and reads with `serde_json`: the chat's ([`@xmoq/chat`](../chat/README.md)). It keeps the text of each number, and it refuses what `serde_json` refuses. The same bytes therefore give the same value, or the same reason why there is none, in TypeScript as in Rust. Why not `JSON.parse`, case by case, is at the top of `src/json.ts`.

| Export | What it is |
|---|---|
| `readJson(text)` | The one value that is the whole of `text`, or `undefined` for text that is no JSON or that `serde_json` would not read |
| `JsonValue` | An object with its members, an array with its items, a string, a number as its text, or a literal |

It is in layer 0 (`scripts/ci/check-layers.sh`): the packages of the tracks each read their payloads with it, and packages of one layer may not depend on each other. No input or output.

Its edges (`1e3`, `-0`, an escaped lone surrogate, nesting of 128) are checked through the shared tables of the payloads (`fixtures/chat-tracks/`), which the Rust tests read too. `test/json.test.ts` checks what the tables do not reach: the items of an array.
