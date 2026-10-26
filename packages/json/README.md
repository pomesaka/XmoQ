# @xmoq/json

A reader of JSON (RFC 8259) for what Rust writes and reads with `serde_json`: the payloads of the chat's tracks ([`@xmoq/chat`](../chat/README.md)), of the transcripts' ([`@xmoq/transcript`](../transcript/README.md)), and the catalogs of MSF ([`@xmoq/msf`](../msf/README.md)). It keeps the text of each number, and it refuses what `serde_json` refuses. The same bytes therefore give the same value, or the same reason why there is none, in TypeScript as in Rust. Why not `JSON.parse`, case by case, is at the top of `src/json.ts`.

| Export | What it is |
|---|---|
| `readJson(text)` | The one value that is the whole of `text`, or `undefined` for text that is no JSON or that `serde_json` would not read |
| `JsonValue` | An object with its members, an array with its items, a string, a number as its text, or a literal (`true`, `false`, `null`) |
| `readJsonPayload(payload)` | The value of bytes of UTF-8, as `readJson` reads text; `undefined` for bytes that are not UTF-8 or begin with a byte order mark, which `serde_json` does not take |
| `membersOf(value)` | The members of an object, `undefined` for any other value |
| `wholeNumberOf(value, most?)` | A number written as a whole number from 0 to `most` (2^53 - 1 at most), with no fraction, exponent or sign, as `as_u64` of `serde_json` takes it |
| `plainOf(value)` | The value as `JSON.parse` gives it, for a reader that checks a document by the types of its fields |

It is in layer 0 (`scripts/ci/check-layers.sh`): the packages of the tracks each read their payloads with it, and packages of one layer may not depend on each other. No input or output.

Its edges (`1e3`, `-0`, an escaped lone surrogate, nesting of 128) are checked through the shared tables of the payloads (`fixtures/chat-tracks/`, `fixtures/transcript-tracks/`), which the Rust tests read too. `test/json.test.ts` checks what the tables do not reach: the items of an array, and the helpers above. The catalogs' table ([`fixtures/msf-01/`](../../fixtures/msf-01/README.md)) has a nesting of 127 and of 128, and a key twice, read by `crates/msf` as well.
